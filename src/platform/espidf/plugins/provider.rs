use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::mpsc::{self, SyncSender},
    thread,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_error::{Context, Result as AnyResult, ensure};
use goodwin_plugin::provider::{
    CHAT_STREAM_METHOD, Capability, ChatRequest, ChatRole, MAX_CHAT_BYTES, MAX_SPEECH_CHUNK_BYTES,
    MODELS_METHOD, ModelCatalog, ModelsRequest, SPEECH_STREAM_METHOD, SpeechAudio, SpeechRequest,
};

use super::{
    Failure, PathBuf, PluginIdentity, Plugins, Runtime, connections, random_id, services, store,
};
use crate::{
    plugins::ProviderConfig,
    voice_core::{VoiceProviders, VoiceRequest},
};

pub(in crate::platform::espidf) struct Binding {
    id: String,
    revision: String,
    config: ProviderConfig,
    streaming: bool,
    _package: std::sync::Arc<()>,
}

impl Plugins {
    pub(in crate::platform::espidf) fn bind_voice(
        &self,
        pipeline: &crate::assistant::PipelineSettings,
    ) -> Result<VoiceProviders<Option<Binding>>, (crate::panel_core::Stage, Failure)> {
        let runtime = self.lock();
        let mut configs = BTreeMap::new();
        VoiceProviders::resolve(|capability| {
            let stage = pipeline.get(capability);
            if stage.provider == crate::builtin_assistant::BUILTIN_PROVIDER {
                return Ok(None);
            }
            runtime
                .bind(&stage.provider, capability, &mut configs)
                .map(Some)
        })
    }

    fn supports_streaming(&self, provider: &str, capability: Capability) -> bool {
        let runtime = self.lock();
        let Ok((id, connection)) = runtime.resolve_provider(provider, capability) else {
            return false;
        };
        let Some(entry) = runtime.entries.get(&id) else {
            return false;
        };
        let Some(registration) = entry.registration() else {
            return false;
        };
        match capability {
            Capability::Chat => registration.chat_streaming,
            Capability::Speech => {
                registration.speech_streaming
                    && connection.is_none_or(|id| {
                        entry
                            .connections
                            .iter()
                            .find(|view| view.id == id)
                            .is_some_and(|view| view.speech_streaming)
                    })
            }
            Capability::Transcription => false,
        }
    }

    pub(in crate::platform::espidf) fn provider_available(
        &self,
        provider: &str,
        capability: Capability,
    ) -> bool {
        self.lock().resolve_provider(provider, capability).is_ok()
    }

    pub(in crate::platform::espidf) fn connections(
        &self,
        id: &str,
    ) -> Result<Vec<connections::View>, Failure> {
        let (_, connections) = self.lock().connections(id)?;
        Ok(connections.view(id))
    }

    pub(in crate::platform::espidf) fn save_connection(
        &self,
        id: &str,
        update: connections::Update,
    ) -> Result<(String, Vec<connections::View>), Failure> {
        let mut runtime = self.lock();
        let (directory, mut connections) = runtime.connections(id)?;
        let saved = connections.update(update, || Ok(format!("c-{}", random_id()?)))?;
        let connection = connections.connection(&saved).ok_or(Failure::Storage)?;
        if !services::valid_provider_url(&connection.base_url, &connection.key) {
            return Err(Failure::InvalidSettings);
        }
        connections.save(&directory).map_err(store::storage)?;
        let view = connections.view(id);
        let entry = runtime.entries.get_mut(id).ok_or(Failure::NotFound)?;
        entry.connections.clone_from(&view);
        entry.settings_revision = connections.revision();
        Ok((saved, view))
    }

    pub(in crate::platform::espidf) fn remove_connection(
        &self,
        id: &str,
        connection: &str,
    ) -> Result<Vec<connections::View>, Failure> {
        let mut runtime = self.lock();
        let (directory, mut connections) = runtime.connections(id)?;
        connections.remove(connection)?;
        connections.save(&directory).map_err(store::storage)?;
        let view = connections.view(id);
        let entry = runtime.entries.get_mut(id).ok_or(Failure::NotFound)?;
        entry.connections.clone_from(&view);
        entry.settings_revision = connections.revision();
        Ok(view)
    }

    pub(in crate::platform::espidf) fn models(
        &self,
        provider: &str,
        capability: Capability,
    ) -> Result<ModelCatalog, Failure> {
        self.provider_call(
            provider,
            capability,
            MODELS_METHOD,
            ModelsRequest { capability },
        )
    }

    fn provider_call<R: serde::de::DeserializeOwned>(
        &self,
        provider: &str,
        capability: Capability,
        method: &str,
        params: impl serde::Serialize,
    ) -> Result<R, Failure> {
        self.invoke_provider(
            ProviderTarget::Current(provider, capability),
            method,
            params,
            None,
        )
    }

    fn invoke_provider<R: serde::de::DeserializeOwned>(
        &self,
        target: ProviderTarget<'_>,
        method: &str,
        params: impl serde::Serialize,
        chunks: Option<services::StreamChunks>,
    ) -> Result<R, Failure> {
        let (id, config, mut instance) = {
            let mut runtime = self.lock();
            let (id, config) = match target {
                ProviderTarget::Current(provider, capability) => {
                    if !crate::plugins::valid_provider_id(provider) {
                        return Err(Failure::NotFound);
                    }
                    let (id, connection) = runtime.resolve_provider(provider, capability)?;
                    let config = connection
                        .as_deref()
                        .map(|connection| {
                            ProviderConfig::read(
                                &runtime.store.slot(&id).join("data"),
                                Some(connection),
                            )
                            .map_err(store::storage)
                        })
                        .transpose()?;
                    (id, config)
                }
                ProviderTarget::Bound(binding, request) => {
                    request.check().map_err(|_| Failure::Unavailable)?;
                    let entry = runtime.entries.get(&binding.id).ok_or(Failure::NotFound)?;
                    if entry.revision() != Some(&binding.revision) {
                        return Err(Failure::Unavailable);
                    }
                    (binding.id.clone(), Some(binding.config.clone()))
                }
            };
            let entry = runtime.entries.get_mut(&id).ok_or(Failure::NotFound)?;
            if entry.leased.is_some()
                || entry
                    .active
                    .as_ref()
                    .is_some_and(|instance| instance.timer.playing())
            {
                return Err(Failure::Busy);
            }
            let instance = entry.active.take().ok_or(Failure::Unavailable)?;
            entry.leased = Some(PluginIdentity {
                registration: instance.registration.clone(),
                revision: instance.revision.clone(),
                subscription: instance.wake_subscription,
            });
            (id, config, instance)
        };
        let result = instance.invoke(
            method,
            params,
            services::Call {
                config,
                chunks,
                request: match target {
                    ProviderTarget::Bound(_, request) => Some(request.clone()),
                    ProviderTarget::Current(..) => None,
                },
            },
            goodwin_json::from_str,
        );
        let mut runtime = self.lock();
        let entry = runtime
            .entries
            .get_mut(&id)
            .expect("leased plugin remains installed");
        entry.leased = None;
        if result.is_ok() {
            entry.active = Some(instance);
        }
        self.refresh_wake_targets(&mut runtime);
        let (value, signals) = result
            .map_err(|error| {
                log::warn!("WASM provider {id} stopped: {error:#}");
                Failure::Unavailable
            })?
            .ok_or(Failure::Unavailable)?;
        self.add_signals(signals);
        self.wake_scheduler();
        Ok(value)
    }
}

impl Runtime {
    fn bind<'a>(
        &self,
        provider: &'a str,
        capability: Capability,
        configs: &mut BTreeMap<&'a str, ProviderConfig>,
    ) -> Result<Binding, Failure> {
        let (id, connection) = self.resolve_provider(provider, capability)?;
        let entry = self.entries.get(&id).ok_or(Failure::NotFound)?;
        let revision = entry.revision().ok_or(Failure::Unavailable)?.to_owned();
        let config = match configs.entry(provider) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry
                .insert(
                    ProviderConfig::read(&self.store.slot(&id).join("data"), connection.as_deref())
                        .map_err(store::storage)?,
                )
                .clone(),
        };
        let registration = entry.registration().ok_or(Failure::Unavailable)?;
        let streaming = match capability {
            Capability::Chat => registration.chat_streaming,
            Capability::Speech => {
                registration.speech_streaming
                    && connection.as_ref().is_none_or(|id| {
                        entry
                            .connections
                            .iter()
                            .find(|view| &view.id == id)
                            .is_some_and(|view| view.speech_streaming)
                    })
            }
            Capability::Transcription => false,
        };
        Ok(Binding {
            id,
            revision,
            config,
            streaming,
            _package: entry.voice.clone(),
        })
    }

    pub(super) fn connections(&self, id: &str) -> Result<(PathBuf, connections::Store), Failure> {
        let entry = self
            .entries
            .get(id)
            .filter(|entry| entry.manifest.provider_connections)
            .ok_or(Failure::NotFound)?;
        if entry.updating {
            return Err(Failure::Busy);
        }
        let directory = self.store.slot(id).join("data");
        let connections = connections::Store::read(&directory).map_err(store::storage)?;
        Ok((directory, connections))
    }

    fn resolve_provider(
        &self,
        provider: &str,
        capability: Capability,
    ) -> Result<(String, Option<String>), Failure> {
        for (id, entry) in &self.entries {
            if let Some(registration) = entry.registration()
                && entry.manifest.provider_connections
                && let Some(connection) = entry
                    .connections
                    .iter()
                    .find(|view| view.provider_id == provider)
            {
                return registration
                    .providers
                    .contains(&capability)
                    .then(|| (id.clone(), Some(connection.id.clone())))
                    .ok_or(Failure::Unavailable);
            }
        }
        let entry = self.entries.get(provider).ok_or(Failure::NotFound)?;
        entry
            .registration()
            .filter(|registration| {
                !entry.manifest.provider_connections && registration.providers.contains(&capability)
            })
            .map(|_| (provider.to_owned(), None))
            .ok_or(Failure::Unavailable)
    }
}

pub(in crate::platform::espidf) struct Provider<'a> {
    pub plugin: &'a Plugins,
    pub settings: &'a crate::assistant::StageSettings,
    pub binding: Option<(&'a Binding, &'a VoiceRequest)>,
}

#[derive(Clone, Copy)]
enum ProviderTarget<'a> {
    Current(&'a str, Capability),
    Bound(&'a Binding, &'a VoiceRequest),
}

impl Provider<'_> {
    fn target(&self, capability: Capability) -> ProviderTarget<'_> {
        self.binding.map_or(
            ProviderTarget::Current(&self.settings.provider, capability),
            |(binding, request)| ProviderTarget::Bound(binding, request),
        )
    }

    pub(in crate::platform::espidf) fn transcribe(
        &self,
        wav: &[u8],
    ) -> goodwin_error::Result<String> {
        use goodwin_plugin::provider::{Transcription, TranscriptionRequest};

        let response: Transcription = self.call(
            Capability::Transcription,
            TranscriptionRequest {
                wav_base64: STANDARD.encode(wav),
                language: "ru".into(),
                selection: self.settings.selection.clone(),
            },
        )?;
        Ok(response.text.trim().to_owned())
    }

    pub(in crate::platform::espidf) fn complete(
        &self,
        mut request: ChatRequest,
    ) -> goodwin_error::Result<crate::voice_core::tools::CompletionMessage> {
        request.selection = self.settings.selection.clone();
        let response: goodwin_plugin::provider::Completion =
            self.call(Capability::Chat, request)?;
        goodwin_error::ensure!(
            response.message.role == ChatRole::Assistant,
            "Invalid plugin completion role"
        );
        Ok(crate::voice_core::tools::CompletionMessage(
            response.message,
        ))
    }

    pub(in crate::platform::espidf) fn streaming(&self, capability: Capability) -> bool {
        if let Some((binding, _)) = self.binding {
            return binding.streaming;
        }
        self.plugin
            .supports_streaming(&self.settings.provider, capability)
    }

    pub(in crate::platform::espidf) fn stream_chat(
        &self,
        mut request: ChatRequest,
        mut consume: impl FnMut(&str),
    ) -> AnyResult<String> {
        request.selection = self.settings.selection.clone();
        let mut answer = String::new();
        self.stream(
            Capability::Chat,
            CHAT_STREAM_METHOD,
            request,
            services::StreamChunks::Chat,
            |chunk| {
                ensure!(
                    answer.len() + chunk.len() <= MAX_CHAT_BYTES,
                    "Plugin chat response is too large"
                );
                consume(&chunk);
                answer.push_str(&chunk);
                Ok(())
            },
        )?;
        ensure!(!answer.trim().is_empty(), "Empty plugin chat response");
        Ok(answer.trim().to_owned())
    }

    pub(in crate::platform::espidf) fn synthesize(
        &self,
        answer: &str,
    ) -> goodwin_error::Result<Vec<u8>> {
        let reply: SpeechAudio = self.call(
            Capability::Speech,
            SpeechRequest {
                text: answer.into(),
                selection: self.settings.selection.clone(),
            },
        )?;
        PcmNormalizer::default().push(reply, 1_500_000)
    }

    pub(in crate::platform::espidf) fn stream_speech(
        &self,
        answer: &str,
        mut consume: impl FnMut(&[u8]) -> AnyResult<()>,
    ) -> AnyResult<()> {
        let request = SpeechRequest {
            text: answer.into(),
            selection: self.settings.selection.clone(),
        };
        let mut pcm = PcmNormalizer::default();
        self.stream(
            Capability::Speech,
            SPEECH_STREAM_METHOD,
            request,
            services::StreamChunks::Speech,
            |chunk| consume(&pcm.push(chunk, MAX_SPEECH_CHUNK_BYTES)?),
        )?;
        ensure!(pcm.frames != 0, "Empty plugin speech response");
        Ok(())
    }

    fn stream<T: Send>(
        &self,
        capability: Capability,
        method: &str,
        request: impl serde::Serialize + Send,
        wrap: fn(SyncSender<T>) -> services::StreamChunks,
        mut consume: impl FnMut(T) -> AnyResult<()>,
    ) -> AnyResult<()> {
        thread::scope(|scope| {
            let (send, receive) = mpsc::sync_channel(2);
            let worker = thread::Builder::new()
                .name("goodwin-plugin-io".into())
                .stack_size(28 * 1024)
                .spawn_scoped(scope, move || {
                    self.plugin.invoke_provider::<()>(
                        self.target(capability),
                        method,
                        request,
                        Some(wrap(send)),
                    )
                })?;
            let result = receive.iter().try_for_each(&mut consume);
            drop(receive);
            let finished = worker
                .join()
                .map_err(|_| goodwin_error::report!("Plugin stream worker stopped"))?;
            result?;
            finished.map_err(|error| goodwin_error::report!("{}", error.code()))?;
            Ok(())
        })
    }

    fn call<T: serde::de::DeserializeOwned>(
        &self,
        capability: Capability,
        request: impl serde::Serialize,
    ) -> goodwin_error::Result<T> {
        self.plugin
            .invoke_provider(self.target(capability), capability.method(), request, None)
            .map_err(|error| goodwin_error::report!("{}", error.code()))
    }
}

#[derive(Default)]
struct PcmNormalizer {
    format: Option<(u32, usize)>,
    frames: u64,
    next: u64,
    previous: i32,
}

impl PcmNormalizer {
    fn push(&mut self, audio: SpeechAudio, byte_limit: usize) -> AnyResult<Vec<u8>> {
        const RATE: u64 = crate::builtin_assistant::SPEECH_HZ as u64;
        let channels = usize::from(audio.channels);
        let rate = u64::from(audio.sample_rate);
        ensure!(
            (1..=2).contains(&channels) && (8_000..=96_000).contains(&rate),
            "Invalid plugin PCM format"
        );
        let format = (audio.sample_rate, channels);
        ensure!(
            *self.format.get_or_insert(format) == format,
            "Plugin PCM format changed"
        );
        ensure!(
            audio.pcm_base64.len() <= byte_limit.div_ceil(3) * 4,
            "Plugin PCM chunk is too large"
        );
        let pcm = STANDARD
            .decode(audio.pcm_base64)
            .context("Decode plugin PCM")?;
        ensure!(
            !pcm.is_empty() && pcm.len() <= byte_limit && pcm.len().is_multiple_of(2 * channels),
            "Invalid plugin PCM length"
        );
        let count = (pcm.len() / (2 * channels)) as u64;
        ensure!(
            self.frames + count <= rate * 30,
            "Plugin PCM response is too long"
        );
        if rate == RATE && channels == 1 {
            self.frames += count;
            return Ok(pcm);
        }
        let mut output = Vec::new();
        output.try_reserve(((count * RATE).div_ceil(rate) as usize + 1) * 2)?;
        for frame in pcm.chunks_exact(channels * 2) {
            let value = frame
                .chunks_exact(2)
                .map(|pair| i32::from(i16::from_le_bytes([pair[0], pair[1]])))
                .sum::<i32>()
                / channels as i32;
            let position = self.frames * RATE;
            if self.frames == 0 {
                self.previous = value;
            }
            while self.next <= position {
                let distance = (position - self.next) as i32;
                let sample = value + ((self.previous - value) * distance) / RATE as i32;
                output.extend_from_slice(&(sample as i16).to_le_bytes());
                self.next += rate;
            }
            self.previous = value;
            self.frames += 1;
        }
        Ok(output)
    }
}
