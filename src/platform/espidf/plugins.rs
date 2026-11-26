use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use goodwin_error::{Context, Result, ensure};
#[cfg(feature = "wake-tflite")]
use goodwin_plugin::wake::{EVENT_METHOD, Event, Observation};
use goodwin_plugin::{HttpRequest, settings::SettingsData};
use serde_json::{Map, Value, json};
#[cfg(feature = "wake-tflite")]
use std::sync::mpsc;

use crate::{
    plugin_wasm::{FRAME_LIMIT, Guest},
    plugins::{
        Failure, connections,
        package::{self, API_VERSION, Manifest},
        registration::{HttpResponse, Registration, Tool},
        settings,
        store::{self, Store},
    },
};

mod provider;
pub(super) use provider::Binding as ProviderBinding;
pub(super) use provider::Provider as PluginProvider;
pub(crate) mod services;
use services::Host;

use super::storage::MOUNT;
pub(super) const ARCHIVE_LIMIT: usize = 768 * 1024;
const EXPANDED_LIMIT: u64 = 1024 * 1024;
const STATE_LIMIT: usize = 64 * 1024;
const MAX_INSTANCES: usize = 4;

pub(super) struct Plugins {
    runtime: Mutex<Runtime>,
    signals: AtomicU32,
    voice_turn: AtomicU32,
    scheduler: Mutex<Option<thread::Thread>>,
    #[cfg(feature = "wake-tflite")]
    wake_events: mpsc::SyncSender<(u32, Event)>,
    wake_epoch: AtomicU32,
}

struct Runtime {
    store: Store,
    entries: BTreeMap<String, Entry>,
    wake_targets: Vec<(String, u64)>,
}

struct Entry {
    manifest: Manifest,
    active: Option<Instance>,
    connections: Vec<connections::View>,
    settings_revision: u64,
    leased: Option<PluginIdentity>,
    updating: bool,
    voice: Arc<()>,
}

struct PluginIdentity {
    registration: Arc<Registration>,
    revision: String,
    subscription: u64,
}

struct Instance {
    guest: Guest<Host>,
    registration: Arc<Registration>,
    state: Option<PathBuf>,
    deadline: Option<u64>,
    revision: String,
    next_id: u64,
    wake_subscription: u64,
}

pub(super) struct VoiceTools<'a> {
    plugins: &'a Plugins,
    request: &'a crate::voice_core::VoiceRequest,
    tools: BTreeMap<String, (String, String, Tool)>,
    context_tools: Vec<(String, String)>,
    turn: u32,
}

impl Plugins {
    pub(super) fn start() -> Result<Arc<Self>> {
        let store = Store::new(Path::new(MOUNT));
        store.recover()?;
        store.dirs()?;
        let mut entries = BTreeMap::new();
        for item in fs::read_dir(store.installed())? {
            let item = item?;
            let Some(id) = item
                .file_name()
                .into_string()
                .ok()
                .filter(|id| package::name(id))
            else {
                continue;
            };
            match Manifest::read(&item.path().join("package")) {
                Ok(manifest) if manifest.id == id => {
                    let active =
                        super::memory::external(|| Instance::open(&manifest, &item.path()))
                            .inspect_err(|error| log::warn!("Start plugin {id}: {error:#}"))
                            .ok();
                    entries.insert(id, Entry::new(manifest, active, &item.path()));
                }
                Ok(_) => log::warn!("Plugin {id} has a mismatched manifest identity"),
                Err(error) => log::warn!("Read plugin {id}: {error:#}"),
            }
        }
        #[cfg(feature = "wake-tflite")]
        let (wake_events, observations) = mpsc::sync_channel(3);
        let plugins = Arc::new(Self {
            runtime: Mutex::new(Runtime {
                store,
                entries,
                wake_targets: Vec::new(),
            }),
            signals: AtomicU32::new(0),
            voice_turn: AtomicU32::new(0),
            scheduler: Mutex::new(None),
            #[cfg(feature = "wake-tflite")]
            wake_events,
            wake_epoch: AtomicU32::new(0),
        });
        plugins.refresh_wake_targets(&mut plugins.lock());
        let shared = Arc::downgrade(&plugins);
        let thread = thread::Builder::new()
            .name("goodwin-plugin-tick".into())
            .stack_size(24 * 1024)
            .spawn(move || {
                while let Some(plugins) = shared.upgrade() {
                    #[cfg(feature = "wake-tflite")]
                    for _ in 0..3 {
                        let Ok((epoch, event)) = observations.try_recv() else {
                            break;
                        };
                        plugins.deliver_wake(epoch, &event);
                    }
                    let wait = plugins.tick();
                    drop(plugins);
                    thread::park_timeout(wait);
                }
            })?;
        *plugins.scheduler.lock().expect("plugin scheduler") = Some(thread.thread().clone());
        Ok(plugins)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.runtime.lock().expect("plugin runtime")
    }

    fn wake_scheduler(&self) {
        if let Some(thread) = self.scheduler.lock().expect("plugin scheduler").as_ref() {
            thread.unpark();
        }
    }

    fn add_signals(&self, count: u32) {
        if count != 0 {
            let _ = self
                .signals
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
                    Some(pending.saturating_add(count).min(32))
                });
        }
    }

    pub(super) fn take_signals(&self) -> u32 {
        self.signals.swap(0, Ordering::Relaxed)
    }

    #[cfg(feature = "wake-tflite")]
    pub(super) fn observe_wake(&self, make_event: impl FnOnce() -> Event) {
        let epoch = self.wake_epoch.load(Ordering::Acquire);
        if epoch & 1 != 0 && self.wake_events.try_send((epoch, make_event())).is_ok() {
            self.wake_scheduler();
        }
    }

    fn refresh_wake_targets(&self, runtime: &mut Runtime) {
        let targets = runtime
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                let token = entry
                    .active
                    .as_ref()
                    .map(|instance| instance.wake_subscription)
                    .or_else(|| entry.leased.as_ref().map(|instance| instance.subscription))?;
                (token != 0).then(|| (id.clone(), token))
            })
            .collect::<Vec<_>>();
        if targets != runtime.wake_targets {
            let epoch = self.wake_epoch.load(Ordering::Relaxed);
            self.wake_epoch.store(
                (epoch.wrapping_add(2) & !1) | u32::from(!targets.is_empty()),
                Ordering::Release,
            );
            runtime.wake_targets = targets;
        }
    }

    #[cfg(feature = "wake-tflite")]
    fn deliver_wake(&self, epoch: u32, event: &Event) {
        let mut runtime = self.lock();
        if self.wake_epoch.load(Ordering::Acquire) != epoch {
            return;
        }
        let Runtime {
            entries,
            wake_targets,
            ..
        } = &mut *runtime;
        for (id, subscription) in wake_targets {
            let entry = entries.get_mut(id).expect("subscribed plugin");
            let observation = Observation {
                subscription: *subscription,
                event,
            };
            if let Ok(params) = serde_json::to_value(observation)
                && let Ok((_, signals)) = entry.call(EVENT_METHOD, params)
            {
                self.add_signals(signals);
            }
        }
        self.refresh_wake_targets(&mut runtime);
    }

    pub(super) fn voice_tools<'a>(
        &'a self,
        request: &'a crate::voice_core::VoiceRequest,
    ) -> VoiceTools<'a> {
        let runtime = self.lock();
        let mut tools = BTreeMap::new();
        let mut context_tools = Vec::new();
        for (id, entry) in &runtime.entries {
            let Some(instance) = &entry.active else {
                continue;
            };
            for tool in &instance.registration.tools {
                let name = format!("{id}__{}", tool.name);
                if instance.registration.voice_context_tool.as_deref() == Some(&tool.name) {
                    context_tools.push((id.clone(), name.clone()));
                }
                tools.insert(name, (id.clone(), instance.revision.clone(), tool.clone()));
            }
        }
        VoiceTools {
            plugins: self,
            request,
            tools,
            context_tools,
            turn: self.voice_turn.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn tick(&self) -> Duration {
        let mut runtime = self.lock();
        for entry in runtime.entries.values_mut() {
            if entry
                .active
                .as_ref()
                .and_then(|instance| instance.deadline)
                .is_some_and(|wake| wake <= now_ms())
            {
                match entry.call("tick", json!({})) {
                    Ok((_, signals)) => self.add_signals(signals),
                    Err(error) => {
                        log::warn!("Scheduled plugin {}: {}", entry.manifest.id, error.code())
                    }
                }
            }
        }
        self.refresh_wake_targets(&mut runtime);
        let delay = runtime
            .entries
            .values()
            .filter_map(|entry| entry.active.as_ref()?.deadline)
            .min()
            .map_or(1_000, |next| next.saturating_sub(now_ms()).clamp(10, 1_000));
        Duration::from_millis(delay)
    }

    pub(super) fn catalog(&self) -> Vec<Value> {
        let runtime = self.lock();
        runtime.entries.values().map(|entry| {
            let manifest = &entry.manifest;
            let registration = entry.registration();
            json!({
                "id": manifest.id, "title": manifest.title, "version": manifest.version,
                "status": if entry.updating { "updating" } else if registration.is_some() { "ready" } else { "error" },
                "error": (registration.is_none() && !entry.updating).then_some("plugin_unavailable"),
                "revision": entry.revision(),
                "settings_revision": entry.settings_revision,
                "web": registration.and(manifest.web.as_ref()),
                "pages": registration.map(|value| value.pages.as_slice()).unwrap_or_default(),
                "providers": registration.into_iter().flat_map(|value| &value.providers).collect::<Vec<_>>(),
                "model_catalog": registration.is_some_and(|value| value.model_catalog),
                "speech_streaming": registration.is_some_and(|value| value.speech_streaming),
                "has_settings": manifest.provider_connections || !manifest.settings.is_empty(),
                    "connection_settings": manifest.provider_connections, "connections": entry.connections,
            })
        }).collect()
    }

    pub(super) fn install(&self, mut archive: impl Read) -> Result<(), Failure> {
        // The upload worker serializes installers. Network reads and archive
        // expansion must not hold the lock shared with plugin calls and ticks.
        let store = self.lock().store.clone();
        let work = store.work().join("incoming");
        let staged = work.join("slot");
        if work.exists() {
            fs::remove_dir_all(&work).map_err(store::storage)?;
        }
        fs::create_dir_all(&staged).map_err(store::storage)?;
        let result = (|| {
            let manifest =
                package::extract_reader(&mut archive, &staged.join("package"), EXPANDED_LIMIT)
                    .map_err(|error| {
                        log::warn!("Invalid plugin archive: {error:#}");
                        Failure::InvalidPackage
                    })?;
            // The decoder can finish before Content-Length. Require the whole
            // declared request before changing the live package.
            io::copy(&mut archive, &mut io::sink()).map_err(|error| {
                log::warn!("Read plugin archive: {error}");
                Failure::InvalidPackage
            })?;
            self.install_staged(&store, &work, &staged, manifest)
        })();
        if work.exists()
            && let Err(error) = fs::remove_dir_all(&work)
        {
            log::warn!("Clean up plugin upload: {error}");
        }
        self.wake_scheduler();
        result
    }

    fn install_staged(
        &self,
        store: &Store,
        work: &Path,
        staged: &Path,
        manifest: Manifest,
    ) -> Result<(), Failure> {
        let id = &manifest.id;
        if id == crate::builtin_assistant::BUILTIN_PROVIDER {
            return Err(Failure::InvalidPackage);
        }
        let (previous, active) = {
            let mut runtime = self.lock();
            let previous = runtime.entries.contains_key(id);
            if !previous && runtime.entries.len() >= MAX_INSTANCES {
                return Err(Failure::StartFailed);
            }
            let active = if let Some(entry) = runtime.entries.get_mut(id) {
                if entry.in_use() {
                    return Err(Failure::Busy);
                }
                entry.updating = true;
                entry.active.take()
            } else {
                None
            };
            self.refresh_wake_targets(&mut runtime);
            (previous, active)
        };

        // The old entry stays reserved while the update worker uses the flash
        // and starts the candidate. Other plugins can still run and serve HTTP.
        let update = store.update(id, work).map_err(store::storage);
        let installed = update.as_ref().map_err(|error| *error).and_then(|update| {
            update.prepare().map_err(store::storage)?;
            let mut guest =
                super::memory::external(|| Instance::open(&manifest, staged)).map_err(|error| {
                    log::warn!("Start plugin {id}: {error:#}");
                    if previous {
                        Failure::RolledBack
                    } else {
                        Failure::StartFailed
                    }
                })?;
            update.publish().map_err(store::storage)?;
            guest.relocate(&manifest, &store.slot(id));
            update.commit().map_err(store::storage)?;
            Ok(guest)
        });
        match installed {
            Ok(guest) => {
                let slot = store.slot(id);
                let id = id.clone();
                let entry = Entry::new(manifest, Some(guest), &slot);
                let mut runtime = self.lock();
                runtime.entries.insert(id, entry);
                self.refresh_wake_targets(&mut runtime);
                Ok(())
            }
            Err(error) => {
                let restored = update.ok().map_or(Ok(()), |update| update.rollback());
                let mut runtime = self.lock();
                if let Some(entry) = runtime.entries.get_mut(id) {
                    entry.updating = false;
                    if restored.is_ok() {
                        entry.active = active;
                    }
                }
                self.refresh_wake_targets(&mut runtime);
                restored.map_err(store::storage)?;
                Err(error)
            }
        }
    }

    pub(super) fn remove(&self, id: &str) -> Result<(), Failure> {
        let mut runtime = self.lock();
        if runtime.entries.get(id).ok_or(Failure::NotFound)?.in_use() {
            return Err(Failure::Busy);
        }
        if runtime.store.unresolved(id).map_err(store::storage)? {
            return Err(Failure::Storage);
        }
        // Commit removal before cleaning up so a reset cannot restore a partial package.
        let removed = runtime.store.work().join("removed");
        if removed.exists() {
            fs::remove_dir_all(&removed).map_err(store::storage)?;
        }
        fs::rename(runtime.store.slot(id), &removed).map_err(store::storage)?;
        runtime.entries.remove(id);
        self.refresh_wake_targets(&mut runtime);
        if let Err(error) = fs::remove_dir_all(&removed) {
            log::warn!("Clean up removed plugin: {error}");
        }
        Ok(())
    }

    pub(super) fn http(&self, id: &str, request: HttpRequest) -> Result<HttpResponse, Failure> {
        let mut runtime = self.lock();
        let entry = runtime.entries.get_mut(id).ok_or(Failure::NotFound)?;
        let registration = entry.registration().ok_or(Failure::Unavailable)?;
        if !registration
            .routes
            .iter()
            .any(|route| route.accepts(&request.method, &request.path))
        {
            return Err(Failure::NotFound);
        }
        let result = entry.call(
            "http",
            serde_json::to_value(request).map_err(store::storage)?,
        );
        self.refresh_wake_targets(&mut runtime);
        let (value, signals) = result?;
        self.add_signals(signals);
        self.wake_scheduler();
        serde_json::from_value(value).map_err(|_| Failure::Unavailable)
    }

    pub(super) fn asset(&self, id: &str, revision: &str, path: &str) -> Option<File> {
        let runtime = self.lock();
        let entry = runtime.entries.get(id)?;
        entry.manifest.web.as_ref()?;
        if entry.revision()? != revision {
            return None;
        }
        File::open(package::web_file(
            &runtime.store.slot(id).join("package"),
            path,
        )?)
        .ok()
    }

    pub(super) fn settings(
        &self,
        id: &str,
        patch: Option<Map<String, Value>>,
    ) -> Result<settings::View, Failure> {
        let mut runtime = self.lock();
        let path = runtime
            .store
            .slot(id)
            .join("data")
            .join(goodwin_plugin::settings::SETTINGS_FILE);
        let entry = runtime.entries.get_mut(id).ok_or(Failure::NotFound)?;
        if entry.updating {
            return Err(Failure::Busy);
        }
        let manifest = &entry.manifest;
        let fields = &manifest.settings;
        if manifest.provider_connections || fields.is_empty() {
            return Err(Failure::NotFound);
        }
        let mut data = services::read_json::<SettingsData>(&path).map_err(store::storage)?;
        if let Some(patch) = patch {
            data = settings::apply(fields, data, patch)?;
            services::save_json(&path, &data, STATE_LIMIT).map_err(store::storage)?;
            entry.settings_revision = entry.settings_revision.wrapping_add(1);
        }
        Ok(settings::View::new(fields.clone(), data))
    }
}

impl VoiceTools<'_> {
    pub(super) fn definitions(
        &self,
    ) -> impl ExactSizeIterator<Item = (&str, &str, &goodwin_plugin::provider::ToolSchema)> {
        self.tools.iter().map(|(name, (_, _, tool))| {
            (name.as_str(), tool.description.as_str(), &tool.parameters)
        })
    }

    pub(super) fn context(&self) -> BTreeMap<String, Value> {
        let mut context = BTreeMap::new();
        let mut remaining = 8_usize * 1024;
        for (id, name) in &self.context_tools {
            if self.request.failure().is_some() {
                break;
            }
            let value = self.call(name, "{}");
            if value.is_object()
                && value.get("error").is_none()
                && let Some(rest) = remaining.checked_sub(id.len() + value.to_string().len())
            {
                remaining = rest;
                context.insert(id.clone(), value);
            }
        }
        context
    }

    pub(super) fn call(&self, name: &str, arguments: &str) -> Value {
        if self.request.failure().is_some() {
            return json!({"error": "cancelled"});
        }
        let Some((id, revision, tool)) = self.tools.get(name) else {
            return json!({"error": "unknown_tool"});
        };
        let Ok(args) = serde_json::from_str(arguments) else {
            return json!({"error": "invalid_arguments"});
        };
        if !tool.accepts(&args) {
            return json!({"error": "invalid_arguments"});
        }
        let mut runtime = self.plugins.lock();
        let Some(entry) = runtime.entries.get_mut(id).filter(|entry| {
            entry
                .active
                .as_ref()
                .is_some_and(|active| active.revision == *revision)
        }) else {
            return json!({"error": "tool_unavailable"});
        };
        let result = entry.call_with(
            "tool",
            json!({
                "name": tool.name, "arguments": args, "voiceTurn": self.turn,
            }),
            services::Call {
                request: Some(self.request.clone()),
                ..Default::default()
            },
        );
        self.plugins.refresh_wake_targets(&mut runtime);
        match result {
            Ok((result, signals)) => {
                self.plugins.add_signals(signals);
                self.plugins.wake_scheduler();
                result
            }
            Err(_) => json!({"error": "tool_unavailable"}),
        }
    }
}

impl Entry {
    fn new(manifest: Manifest, active: Option<Instance>, slot: &Path) -> Self {
        let saved = manifest.provider_connections.then(|| {
            connections::Store::read(&slot.join("data"))
                .map_err(store::storage)
                .unwrap_or_default()
        });
        Self {
            connections: saved
                .as_ref()
                .map(|saved| saved.view(&manifest.id))
                .unwrap_or_default(),
            settings_revision: saved.as_ref().map_or(0, connections::Store::revision),
            manifest,
            active,
            leased: None,
            updating: false,
            voice: Arc::default(),
        }
    }

    fn in_use(&self) -> bool {
        self.updating || self.leased.is_some() || Arc::strong_count(&self.voice) > 1
    }

    fn registration(&self) -> Option<&Registration> {
        self.active
            .as_ref()
            .map(|instance| instance.registration.as_ref())
            .or_else(|| {
                self.leased
                    .as_ref()
                    .map(|instance| instance.registration.as_ref())
            })
    }

    fn revision(&self) -> Option<&str> {
        self.active
            .as_ref()
            .map(|instance| instance.revision.as_str())
            .or_else(|| {
                self.leased
                    .as_ref()
                    .map(|instance| instance.revision.as_str())
            })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<(Value, u32), Failure> {
        self.call_with(method, params, services::Call::default())
    }

    fn call_with(
        &mut self,
        method: &str,
        params: Value,
        call: services::Call,
    ) -> Result<(Value, u32), Failure> {
        if self.leased.is_some() {
            return Err(Failure::Busy);
        }
        let instance = self.active.as_mut().ok_or(Failure::Unavailable)?;
        match instance.invoke(method, params, call, |value| serde_json::from_str(value)) {
            Ok(Some(result)) => Ok(result),
            Ok(None) => Err(Failure::Unavailable),
            Err(error) => {
                log::warn!("WASM plugin {} stopped: {error:#}", self.manifest.id);
                self.active = None;
                Err(Failure::Unavailable)
            }
        }
    }
}

impl Instance {
    fn open(manifest: &Manifest, root: &Path) -> Result<Self> {
        ensure!(manifest.is_wasm(), "ESP supports WASM plugin modules");
        let data = root.join("data");
        if manifest.provider_connections {
            connections::Store::initialize(&manifest.id, &data)?;
        }
        let state = manifest
            .wasm_state_file
            .as_ref()
            .map(|file| data.join(file));
        let saved: Value = state
            .as_deref()
            .map(services::read_json)
            .transpose()?
            .unwrap_or_default();
        let host = (manifest.wasm_storage || manifest.wasm_http.is_some())
            .then(|| Host::new(data.clone(), manifest));
        let memory_limit = if manifest.wasm_http.is_some() {
            3 * 1024 * 1024
        } else if host.is_some() {
            1536 * 1024
        } else {
            512 * 1024
        };
        let guest = Guest::open(
            &root.join("package").join(&manifest.executable),
            host,
            memory_limit,
        )?;
        let mut instance = Self {
            guest,
            registration: Arc::new(serde_json::from_value(json!({"apiVersion": API_VERSION}))?),
            state,
            deadline: None,
            revision: random_id()?,
            next_id: 0,
            wake_subscription: 0,
        };
        let (registration, _) = instance
            .invoke(
                "initialize",
                json!({
                    "apiVersion": API_VERSION, "state": saved,
                }),
                services::Call::default(),
                |value| serde_json::from_str(value),
            )?
            .context("WASM plugin initialization failed")?;
        instance.registration = Arc::new(serde_json::from_value(registration)?);
        instance.registration.validate_wasm(manifest)?;
        if instance.registration.wake_events {
            ensure!(
                cfg!(feature = "wake-tflite"),
                "Wake events require an installed wake runtime"
            );
            instance.wake_subscription = instance.registration.wake_subscription;
        }
        Ok(instance)
    }

    fn relocate(&mut self, manifest: &Manifest, slot: &Path) {
        let data = slot.join("data");
        self.state = manifest
            .wasm_state_file
            .as_ref()
            .map(|file| data.join(file));
        if let Some(host) = self.guest.host_mut() {
            host.data = data;
        }
    }

    fn invoke<R, E>(
        &mut self,
        method: &str,
        params: impl serde::Serialize,
        call: services::Call,
        decode: impl FnOnce(&str) -> std::result::Result<R, E>,
    ) -> Result<Option<(R, u32)>>
    where
        E: Into<goodwin_error::Error>,
    {
        let id = self.next_id;
        self.next_id = id.wrapping_add(1);
        let frame = serde_json::to_vec(&crate::plugin_rpc::Request {
            jsonrpc: "2.0",
            id,
            method,
            params,
            now: Some(now_ms()),
        })?;
        let mut reply = None;
        let mut decode = Some(decode);
        let signals = self.exchange(id, method, &frame, call, &mut |value| {
            reply =
                Some(decode.take().context("Repeated WASM result")?(value).map_err(Into::into)?);
            Ok(())
        })?;
        Ok(reply.zip(signals))
    }

    fn exchange(
        &mut self,
        id: u64,
        method: &str,
        frame: &[u8],
        call: services::Call,
        decode: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<Option<u32>> {
        let timeout = if method.starts_with("provider.") {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(5)
        };
        let scope = self.guest.host_mut().map(|host| host.begin(call, timeout));
        let response_limit = match method {
            "provider.synthesize" => 2 * 1024 * 1024,
            "provider.models" | "provider.complete" => 256 * 1024,
            _ => FRAME_LIMIT,
        };
        let mut metadata = None;
        let result = super::memory::external(|| {
            self.guest.call_with(frame, response_limit, &mut |bytes| {
                let reply: crate::plugin_rpc::Response<&serde_json::value::RawValue> =
                    serde_json::from_slice(bytes).context("Decode WASM RPC")?;
                ensure!(
                    reply.jsonrpc == "2.0" && reply.id == id,
                    "Invalid WASM RPC response"
                );
                let rejected = reply.error.is_some();
                if !rejected {
                    decode(reply.result.context("Missing WASM result")?.get())?;
                }
                metadata = Some((
                    reply.wake_subscription,
                    rejected,
                    reply.state,
                    reply.wake_at,
                ));
                Ok(())
            })
        });
        drop(scope);
        if let Some(host) = self.guest.host_mut() {
            host.finish();
        }
        let signals = result?;
        let (subscription, rejected, state, wake_at) =
            metadata.context("Missing WASM RPC response")?;
        if self.registration.wake_events
            && let Some(token) = subscription
        {
            self.wake_subscription = token;
        }
        if rejected {
            if method == "tick" {
                self.deadline = Some(now_ms().saturating_add(1_000));
            }
            return Ok(None);
        }
        if let Some(state) = state {
            services::save_json(
                self.state
                    .as_deref()
                    .context("WASM plugin has no declared state file")?,
                &state,
                STATE_LIMIT,
            )?;
        }
        if let Some(next) = wake_at {
            self.deadline = next;
        } else if method == "tick" {
            self.deadline = None;
        }
        Ok(Some(signals))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn random_id() -> Result<String> {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes)
        .map_err(|_| goodwin_error::report!("Generate plugin identifier"))?;
    Ok(format!("{:016x}", u64::from_le_bytes(bytes)))
}
