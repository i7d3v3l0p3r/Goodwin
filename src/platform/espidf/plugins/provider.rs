use std::{
    sync::mpsc::{self, SyncSender},
    thread,
};

use anyhow::{Context, Result as AnyResult, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_plugin::provider::{
    CHAT_STREAM_METHOD, Capability, ChatRequest, MAX_CHAT_BYTES, MAX_SPEECH_CHUNK_BYTES,
    MODELS_METHOD, ModelCatalog, ModelsRequest, SPEECH_STREAM_METHOD, SpeechAudio, SpeechRequest,
};

use super::{
    Failure, PathBuf, PluginIdentity, Plugins, Runtime, Value, connections, random_id, services,
    store,
};

impl Plugins {
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
        let params = serde_json::to_value(ModelsRequest { capability }).map_err(store::storage)?;
        let value = self.provider_call(provider, capability, MODELS_METHOD, params)?;
        serde_json::from_value(value).map_err(|_| Failure::Unavailable)
    }

    pub(in crate::platform::espidf) fn provider_call(
        &self,
        provider: &str,
        capability: Capability,
        method: &str,
        params: Value,
    ) -> Result<Value, Failure> {
        self.invoke_provider(provider, capability, method, params, None)
    }

    fn invoke_provider(
        &self,
        provider: &str,
        capability: Capability,
        method: &str,
        params: Value,
        chunks: Option<services::StreamChunks>,
    ) -> Result<Value, Failure> {
        if !crate::plugins::valid_provider_id(provider) {
            return Err(Failure::NotFound);
        }
        let (id, connection, mut instance) = {
            let mut runtime = self.lock();
            let (id, connection) = runtime.resolve_provider(provider, capability)?;
            let entry = runtime.entries.get_mut(&id).ok_or(Failure::NotFound)?;
            if entry.leased.is_some() {
                return Err(Failure::Busy);
            }
            let instance = entry.active.take().ok_or(Failure::Unavailable)?;
            entry.leased = Some(PluginIdentity {
                registration: instance.registration.clone(),
                revision: instance.revision.clone(),
                subscription: instance.wake_subscription,
            });
            (id, connection, instance)
        };
        let result = instance.invoke(method, params, connection.as_deref(), chunks);
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
    pub(super) fn connections(&self, id: &str) -> Result<(PathBuf, connections::Store), Failure> {
        if !self
            .entries
            .get(id)
            .is_some_and(|entry| entry.manifest.provider_connections)
        {
            return Err(Failure::NotFound);
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
}

impl Provider<'_> {
    pub(in crate::platform::espidf) fn transcribe(&self, wav: &[u8]) -> anyhow::Result<String> {
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
    ) -> anyhow::Result<crate::voice_core::tools::CompletionMessage> {
        request.selection = self.settings.selection.clone();
        let response: goodwin_plugin::provider::Completion =
            self.call(Capability::Chat, request)?;
        anyhow::ensure!(
            response.message.is_object(),
            "Invalid plugin completion response"
        );
        Ok(crate::voice_core::tools::CompletionMessage(
            response.message,
        ))
    }

    pub(in crate::platform::espidf) fn streaming(&self, capability: Capability) -> bool {
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

    pub(in crate::platform::espidf) fn synthesize(&self, answer: &str) -> anyhow::Result<Vec<u8>> {
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
        request: impl serde::Serialize,
        wrap: fn(SyncSender<T>) -> services::StreamChunks,
        mut consume: impl FnMut(T) -> AnyResult<()>,
    ) -> AnyResult<()> {
        let params = serde_json::to_value(request)?;
        thread::scope(|scope| {
            let (send, receive) = mpsc::sync_channel(2);
            let worker = thread::Builder::new()
                .name("goodwin-plugin-io".into())
                .stack_size(28 * 1024)
                .spawn_scoped(scope, move || {
                    self.plugin.invoke_provider(
                        &self.settings.provider,
                        capability,
                        method,
                        params,
                        Some(wrap(send)),
                    )
                })?;
            let result = receive.iter().try_for_each(&mut consume);
            drop(receive);
            let finished = worker
                .join()
                .map_err(|_| anyhow::anyhow!("Plugin stream worker stopped"))?;
            result?;
            finished.map_err(|error| anyhow::anyhow!("{}", error.code()))?;
            Ok(())
        })
    }

    fn call<T: serde::de::DeserializeOwned>(
        &self,
        capability: Capability,
        request: impl serde::Serialize,
    ) -> anyhow::Result<T> {
        let reply = self
            .plugin
            .provider_call(
                &self.settings.provider,
                capability,
                capability.method(),
                serde_json::to_value(request)?,
            )
            .map_err(|error| anyhow::anyhow!("{}", error.code()))?;
        Ok(serde_json::from_value(reply)?)
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
