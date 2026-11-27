use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use goodwin_error::Result;
use goodwin_plugin::{
    HttpRequest,
    provider::{Capability, ModelCatalog},
    settings::SettingsData,
    wake::Event,
};
use serde::Serialize;
#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, mpsc, watch};

use crate::{
    panel_core::random_bytes,
    station::{Player, timer_signal},
};

use super::{
    Failure, VoiceTools, connections,
    package::{self, Manifest},
    process::Process,
    registration::HttpResponse,
    registry::{Active, Entry, Registry, Status, View},
    settings::{self, Field},
    store::{Store, storage},
    timer::Signal,
};

#[derive(Clone)]
pub struct Plugins(Arc<Manager>);

struct Manager {
    #[cfg(target_os = "linux")]
    vpn: tokio::sync::OnceCell<super::vpn::Gateway>,
    store: Store,
    registry: Registry,
    lifecycle: Mutex<()>,
    player: Player,
    sound: Mutex<Option<mpsc::Sender<Signal>>>,
}

#[derive(Serialize)]
pub(crate) struct ProviderRevision {
    package: String,
    settings: u64,
    connection: Option<u64>,
}

struct RegisteredProvider<'a> {
    entry: &'a Entry,
    active: &'a Active,
    connection: Option<&'a connections::View>,
}

fn registered_provider<'a>(
    registry: &'a BTreeMap<String, Entry>,
    id: &str,
    capability: Capability,
) -> Option<RegisteredProvider<'a>> {
    for entry in registry.values() {
        if let Some(active) = entry.live()
            && let Some(connection) = active
                .connections
                .iter()
                .find(|view| view.provider_id == id)
        {
            return active
                .registration
                .providers
                .contains(&capability)
                .then_some(RegisteredProvider {
                    entry,
                    active,
                    connection: Some(connection),
                });
        }
    }
    let entry = registry.get(id)?;
    if entry.manifest.provider_connections {
        return None;
    }
    let active = entry.live()?;
    active
        .registration
        .providers
        .contains(&capability)
        .then_some(RegisteredProvider {
            entry,
            active,
            connection: None,
        })
}

impl Plugins {
    pub fn new(dir: &Path, player: Player) -> Self {
        Self(Arc::new(Manager {
            #[cfg(target_os = "linux")]
            vpn: Default::default(),
            store: Store::new(dir),
            registry: Registry::default(),
            lifecycle: Mutex::default(),
            player,
            sound: Mutex::default(),
        }))
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    pub async fn restore(&self) {
        let _operation = self.0.lifecycle.lock().await;
        if let Err(error) = self.0.store.recover() {
            log::warn!("Could not recover plugin packages: {error:#}");
            return;
        }
        self.load_installed(&[]).await;
    }

    pub async fn restore_selected(&self, ids: &[&str]) {
        let _operation = self.0.lifecycle.lock().await;
        self.load_installed(ids).await;
    }

    async fn load_installed(&self, ids: &[&str]) {
        let Ok(installed) = fs::read_dir(self.0.store.installed()) else {
            return;
        };
        for item in installed.flatten() {
            let Some(id) = item
                .file_name()
                .into_string()
                .ok()
                .filter(|id| package::name(id))
            else {
                continue;
            };
            if !ids.is_empty()
                && id != "vpn"
                && !ids.iter().any(|provider| {
                    provider
                        .split_once(':')
                        .map_or(*provider, |(plugin, _)| plugin)
                        == id
                })
            {
                continue;
            }
            match self.0.store.unresolved(&id) {
                Ok(false) => {}
                Ok(true) => {
                    log::warn!("Plugin {id} is awaiting package recovery");
                    continue;
                }
                Err(error) => {
                    log::warn!("Could not inspect plugin {id} recovery: {error:#}");
                    continue;
                }
            }
            let manifest = Manifest::read(&item.path().join("package"));
            match manifest {
                Ok(manifest) if manifest.id == id => {
                    self.0.activate(manifest, None).await;
                }
                Ok(_) => log::warn!("Plugin {id} has a mismatched manifest identity"),
                Err(error) => log::warn!("Could not read plugin {id}: {error}"),
            }
        }
    }

    pub(crate) fn request_shutdown(&self) {
        for entry in self.0.registry.read().values() {
            if let Some(active) = &entry.active {
                active.process.request_stop();
            }
        }
    }

    pub async fn shutdown(&self) {
        let _operation = self.0.lifecycle.lock().await;
        let entries = self.0.registry.take();
        for entry in entries.into_values() {
            if let Some(active) = entry.active {
                active.process.stop().await;
            }
        }
    }

    pub fn catalog(&self) -> Vec<View> {
        self.0.registry.catalog()
    }

    pub(crate) fn catalog_changes(&self) -> watch::Receiver<()> {
        self.0.registry.changes.subscribe()
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn route_via_vpn(&self, id: &str, selected: bool) -> Result<(), Failure> {
        let _operation = self.0.lifecycle.lock().await;
        let manifest = {
            let entries = self.0.registry.read();
            let entry = entries.get(id).ok_or(Failure::NotFound)?;
            if selected && (!entry.manifest.vpn_capable() || !entries.contains_key("vpn")) {
                return Err(Failure::InvalidSettings);
            }
            if !entry.manifest.vpn_capable() && entry.vpn != Some(true) {
                return Err(Failure::InvalidSettings);
            }
            if entry.vpn == Some(selected) && entry.live().is_some() {
                return Ok(());
            }
            entry.manifest.clone()
        };
        self.0.withdraw(id, Status::Updating).await?;
        let path = self.0.vpn_selection(id);
        let saved = goodwin_plugin::save_json(&path, &selected).map_err(storage);
        let started = self.0.activate(manifest, None).await;
        saved?;
        started.then_some(()).ok_or(Failure::StartFailed)
    }

    fn settings_fields(&self, id: &str) -> Result<Vec<Field>, Failure> {
        let registry = self.0.registry.read();
        let manifest = &registry.get(id).ok_or(Failure::NotFound)?.manifest;
        if manifest.settings.is_empty() || manifest.provider_connections {
            return Err(Failure::NotFound);
        }
        Ok(manifest.settings.clone())
    }

    pub(crate) async fn settings(&self, id: &str) -> Result<settings::View, Failure> {
        let _operation = self.0.lifecycle.lock().await;
        let fields = self.settings_fields(id)?;
        let directory = self.0.store.slot(id).join("data");
        tokio::task::spawn_blocking(move || {
            let data = SettingsData::read(&directory).map_err(storage)?;
            Ok(settings::View::new(fields, data))
        })
        .await
        .map_err(storage)?
    }

    pub(crate) async fn update_settings(
        &self,
        id: &str,
        patch: Map<String, Value>,
    ) -> Result<settings::View, Failure> {
        let _operation = self.0.lifecycle.lock().await;
        let fields = self.settings_fields(id)?;
        let directory = self.0.store.slot(id).join("data");
        let manager = self.0.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            manager.registry.update_settings(&id, || {
                let data = SettingsData::read(&directory).map_err(storage)?;
                let data = settings::apply(&fields, data, patch)?;
                data.save(&directory).map_err(storage)?;
                Ok(settings::View::new(fields, data))
            })
        })
        .await
        .map_err(storage)?
    }

    pub(crate) fn provider(&self, id: &str, capability: Capability) -> Option<(Process, bool)> {
        let registry = self.0.registry.read();
        let RegisteredProvider {
            active, connection, ..
        } = registered_provider(&registry, id, capability)?;
        let process = match connection {
            Some(connection) => active
                .process
                .for_connection(&connection.id, connection.speech_streaming),
            None => active.process.clone(),
        };
        Some((process, active.registration.model_catalog))
    }

    pub(crate) fn bind_voice(
        &self,
        pipeline: &crate::assistant::PipelineSettings,
    ) -> std::result::Result<
        crate::voice_core::VoiceProviders<crate::providers::Provider>,
        (crate::app::Stage, crate::app::Failure),
    > {
        use crate::app::Failure;
        let registry = self.0.registry.read();
        #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
        let mut configs = BTreeMap::<&str, super::ProviderConfig>::new();
        crate::voice_core::VoiceProviders::resolve(|capability| {
            let settings = pipeline.get(capability);
            let Some(RegisteredProvider {
                entry,
                active,
                connection,
            }) = registered_provider(&registry, &settings.provider, capability)
            else {
                return Err(Failure::ProviderUnavailable);
            };
            let process = active.process.clone();
            #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
            let process = if entry.manifest.is_wasm() {
                use std::collections::btree_map::Entry;
                let config = match configs.entry(settings.provider.as_str()) {
                    Entry::Occupied(config) => config.get().clone(),
                    Entry::Vacant(config) => {
                        let data = self.0.store.slot(&entry.manifest.id).join("data");
                        config
                            .insert(
                                super::ProviderConfig::read(
                                    &data,
                                    connection.map(|view| view.id.as_str()),
                                )
                                .map_err(|error| {
                                    log::warn!(
                                        "Read voice provider configuration id={}: {error:#}",
                                        entry.manifest.id
                                    );
                                    Failure::ProviderUnavailable
                                })?,
                            )
                            .clone()
                    }
                };
                process.with_provider_config(config)
            } else {
                process
            };
            let _ = (entry, connection);
            crate::providers::Provider::from_process(
                settings,
                capability,
                process,
                active.registration.model_catalog,
            )
            .map(|provider| provider.pin_package(active.voice.clone()))
        })
    }

    pub(crate) fn provider_revisions<const N: usize>(
        &self,
        stages: [(&str, Capability); N],
    ) -> [Option<ProviderRevision>; N] {
        let registry = self.0.registry.read();
        stages.map(|(id, capability)| {
            let RegisteredProvider {
                entry,
                active,
                connection,
            } = registered_provider(&registry, id, capability)?;
            Some(ProviderRevision {
                package: active.revision.clone(),
                settings: entry.settings_revision,
                connection: connection.map(|connection| connection.request_revision),
            })
        })
    }

    pub(crate) async fn connections(&self, id: &str) -> Result<Vec<connections::View>, Failure> {
        self.edit_connections(id, |_| Ok(()))
            .await
            .map(|(_, views)| views)
    }

    pub(crate) async fn save_connection(
        &self,
        id: &str,
        update: connections::Update,
    ) -> Result<(String, Vec<connections::View>), Failure> {
        self.edit_connections(id, move |store| {
            store.update(update, || {
                Ok(format!(
                    "p-{:016x}",
                    u64::from_be_bytes(random_bytes::<8>()?)
                ))
            })
        })
        .await
    }

    pub(crate) async fn remove_connection(
        &self,
        id: &str,
        connection: String,
    ) -> Result<Vec<connections::View>, Failure> {
        self.edit_connections(id, move |store| store.remove(&connection))
            .await
            .map(|(_, views)| views)
    }

    async fn edit_connections<T: Send + 'static>(
        &self,
        id: &str,
        change: impl FnOnce(&mut connections::Store) -> Result<T, Failure> + Send + 'static,
    ) -> Result<(T, Vec<connections::View>), Failure> {
        let _operation = self.0.lifecycle.lock().await;
        if !self
            .0
            .registry
            .read()
            .get(id)
            .is_some_and(|entry| entry.manifest.provider_connections)
        {
            return Err(Failure::NotFound);
        }
        let directory = self.0.store.slot(id).join("data");
        let plugin = id.to_owned();
        let (result, views, changed) = tokio::task::spawn_blocking(move || {
            let mut store = connections::Store::initialize(&plugin, &directory).map_err(storage)?;
            let old_revision = store.revision();
            let result = change(&mut store)?;
            let changed = store.revision() != old_revision;
            if changed {
                store.save(&directory).map_err(storage)?;
            }
            Ok::<_, Failure>((result, store.view(&plugin), changed))
        })
        .await
        .map_err(storage)??;
        if changed {
            self.0.registry.set_connections(id, views.clone());
        }
        Ok((result, views))
    }

    #[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
    pub(crate) fn observe_wake(&self, make_event: impl FnOnce() -> Event) {
        let registry = self.0.registry.read();
        let mut make_event = Some(make_event);
        let mut event = None;
        for (id, entry) in registry.iter() {
            let Some(active) = entry.live() else { continue };
            let Some(subscription) = active.process.wake_subscription() else {
                continue;
            };
            let event = event.get_or_insert_with(|| make_event.take().expect("first observer")());
            if let Err(error) = active.process.observe_wake(subscription, event) {
                log::warn!("Could not enqueue wake event for plugin {id}: {error}");
            }
        }
    }

    pub(crate) async fn models(
        &self,
        id: &str,
        capability: Capability,
    ) -> Result<ModelCatalog, Failure> {
        let (process, supported) = self.provider(id, capability).ok_or(Failure::Unavailable)?;
        if !supported {
            return Err(Failure::NotFound);
        }
        process
            .models(capability)
            .await
            .map_err(|_| Failure::Unavailable)
    }

    #[cfg(test)]
    pub(crate) fn mock_provider(&self, id: &str, process: Process) {
        self.mock_plugin(id, process, json!({
            "apiVersion": 1, "providers": ["transcription", "chat", "speech"], "modelCatalog": true,
        }));
    }

    #[cfg(test)]
    pub(crate) fn mock_plugin(&self, id: &str, process: Process, registration: Value) {
        let manifest = serde_json::from_value(json!({
            "id": id, "version": "1.0.0", "apiVersion": 1,
            "title": {"en": "Fixture provider", "ru": "Тестовый провайдер"},
            "target": "fixture", "executable": "bin/fixture-provider",
        }))
        .unwrap();
        let registration = serde_json::from_value(registration).unwrap();
        self.0.publish(
            manifest,
            Some(Active {
                voice: Arc::default(),
                process,
                registration,
                revision: "fixture".into(),
                connections: Vec::new(),
            }),
            Status::Ready,
            None,
        );
    }

    pub fn voice_tools(&self, turn: u64) -> VoiceTools {
        let entries = self.0.registry.read();
        let mut voice = VoiceTools {
            tools: Default::default(),
            context_tools: Default::default(),
            turn,
        };
        for (id, entry) in entries.iter() {
            let Some(active) = entry.live() else {
                continue;
            };
            if let Some(name) = &active.registration.voice_context_tool {
                voice
                    .context_tools
                    .insert(id.clone(), format!("{id}__{name}"));
            }
            for tool in &active.registration.tools {
                voice.tools.insert(
                    format!("{id}__{}", tool.name),
                    (active.process.clone(), tool.clone()),
                );
            }
        }
        voice
    }

    pub fn asset(&self, id: &str, revision: &str, relative: &str) -> Option<Vec<u8>> {
        let entries = self.0.registry.read();
        let entry = entries.get(id)?;
        let active = entry.live()?;
        if active.revision != revision || entry.manifest.web.is_none() {
            return None;
        }
        let file = package::web_file(&self.0.store.slot(id).join("package"), relative)?;
        fs::read(file).ok()
    }

    pub async fn http(
        &self,
        id: &str,
        request: HttpRequest,
        password_session: bool,
    ) -> Result<HttpResponse, Failure> {
        let process = {
            let entries = self.0.registry.read();
            let entry = entries.get(id).ok_or(Failure::NotFound)?;
            let active = entry.live().ok_or(Failure::Unavailable)?;
            let mut routes = active
                .registration
                .routes
                .iter()
                .filter(|route| route.accepts(&request.method, &request.path));
            let route = routes.next().ok_or(Failure::NotFound)?;
            let private = route.password_required || routes.any(|route| route.password_required);
            if private && !password_session {
                return Err(Failure::PasswordRequired);
            }
            active.process.clone()
        };
        let result = process
            .call(
                "http",
                serde_json::to_value(request).map_err(|_| Failure::Unavailable)?,
            )
            .await
            .map_err(|_| Failure::Unavailable)?;
        serde_json::from_value(result).map_err(|_| Failure::Unavailable)
    }

    pub async fn upload(&self, bytes: axum::body::Bytes) -> Result<(), Failure> {
        // Once received, an upload must finish even if the HTTP caller leaves.
        let plugins = self.clone();
        tokio::spawn(async move { plugins.install(bytes).await })
            .await
            .map_err(storage)?
    }

    async fn install(&self, bytes: axum::body::Bytes) -> Result<(), Failure> {
        let _operation = self.0.lifecycle.lock().await;
        self.0.store.dirs().map_err(storage)?;
        let work = tempfile::tempdir_in(self.0.store.work()).map_err(storage)?;
        let staged = work.path().join("slot");
        fs::create_dir(&staged).map_err(storage)?;
        let package = staged.join("package");
        let manifest = tokio::task::spawn_blocking(move || package::extract(&bytes, &package))
            .await
            .map_err(storage)?
            .map_err(|error| {
                log::warn!("Could not read plugin upload: {error}");
                Failure::InvalidPackage
            })?;
        let id = &manifest.id;
        let update = Arc::new(self.0.store.update(id, work.path()).map_err(storage)?);
        let previous = self.0.store.slot(id);
        let old = previous
            .try_exists()
            .map_err(storage)?
            .then(|| Manifest::read(&previous.join("package")).map_err(storage))
            .transpose()?;
        if old.is_some() {
            self.0.withdraw(id, Status::Updating).await?;
        }

        let staged = {
            let update = update.clone();
            tokio::task::spawn_blocking(move || update.prepare().and_then(|()| update.publish()))
                .await
                .map_err(storage)
                .and_then(|result| result.map_err(storage))
        };
        let failure = match staged {
            Ok(()) => match self.0.start(&manifest).await {
                Ok(active) => {
                    let commit = {
                        let update = update.clone();
                        tokio::task::spawn_blocking(move || update.commit())
                            .await
                            .map_err(storage)
                            .and_then(|result| result.map_err(storage))
                    };
                    if let Err(error) = commit {
                        active.process.stop().await;
                        error
                    } else {
                        self.0.publish(manifest, Some(active), Status::Ready, None);
                        return Ok(());
                    }
                }
                Err(error) => {
                    log::warn!("Could not start plugin {id}: {error:#}");
                    if old.is_some() {
                        Failure::RolledBack
                    } else {
                        Failure::StartFailed
                    }
                }
            },
            Err(error) => error,
        };

        let restored = tokio::task::spawn_blocking(move || update.rollback())
            .await
            .map_err(storage)
            .and_then(|result| result.map_err(storage));
        if let Some(old) = old {
            if let Err(error) = restored {
                self.0.publish(old, None, Status::Error, Some(error.code()));
                return Err(error);
            }
            if !self.0.activate(old, Some(failure.code())).await {
                return Err(Failure::StartFailed);
            }
        } else {
            restored?;
        }
        Err(failure)
    }

    pub async fn remove(&self, id: &str) -> Result<(), Failure> {
        if !package::name(id) {
            return Err(Failure::NotFound);
        }
        let _operation = self.0.lifecycle.lock().await;
        let slot = self.0.store.slot(id);
        if !slot.is_dir() {
            return Err(Failure::NotFound);
        }
        if self.0.store.unresolved(id).map_err(storage)? {
            return Err(Failure::Storage);
        }
        let work = tempfile::tempdir_in(self.0.store.work()).map_err(storage)?;
        self.0.withdraw(id, Status::Removing).await?;
        if let Err(error) = fs::rename(&slot, work.path().join("removed")) {
            if let Ok(manifest) = Manifest::read(&slot.join("package")) {
                self.0
                    .activate(manifest, Some(Failure::Storage.code()))
                    .await;
            }
            return Err(storage(error));
        }
        self.0.registry.remove(id);
        tokio::task::spawn_blocking(move || work.close())
            .await
            .map_err(storage)?
            .map_err(storage)
    }
}

impl Manager {
    fn vpn_selection(&self, id: &str) -> std::path::PathBuf {
        self.store.slot(id).join("data/.goodwin-vpn.json")
    }

    fn publish(
        &self,
        manifest: Manifest,
        active: Option<Active>,
        status: Status,
        error: Option<&'static str>,
    ) {
        let selected = goodwin_plugin::read_json(&self.vpn_selection(&manifest.id)).unwrap_or(true);
        let vpn = (manifest.vpn_capable() || selected).then_some(selected);
        self.registry.insert(manifest, active, status, error, vpn);
    }

    async fn withdraw(&self, id: &str, status: Status) -> Result<(), Failure> {
        if let Some(active) = self.registry.take_active(id, status)? {
            active.process.stop().await;
        }
        Ok(())
    }

    async fn start(&self, manifest: &Manifest) -> Result<Active> {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let revision = URL_SAFE_NO_PAD.encode(random_bytes::<12>()?);
        let slot = self.store.slot(&manifest.id);
        let sound = {
            let mut sound = self.sound.lock().await;
            sound
                .get_or_insert_with(|| {
                    let (send, mut receive) = mpsc::channel::<Signal>(64);
                    let player = self.player.clone();
                    tokio::spawn(async move {
                        let signal = timer_signal();
                        while let Some(alarm) = receive.recv().await {
                            if alarm.cancelled() {
                                continue;
                            }
                            let played = player.play(&signal).await;
                            if let Err(error) = &played {
                                log::warn!("Could not play timer signal: {error:#}");
                            }
                            alarm.finish(played.is_ok());
                        }
                    });
                    send
                })
                .clone()
        };
        let connections = if manifest.provider_connections {
            let id = manifest.id.clone();
            let directory = slot.join("data");
            tokio::task::spawn_blocking(move || {
                Ok::<_, goodwin_error::Error>(
                    connections::Store::initialize(&id, &directory)?.view(&id),
                )
            })
            .await??
        } else {
            Vec::new()
        };
        let selected: bool = goodwin_plugin::read_json(&self.vpn_selection(&manifest.id))?;
        goodwin_error::ensure!(
            !selected || manifest.vpn_capable(),
            "Plugin cannot use its assigned VPN"
        );
        #[cfg(target_os = "linux")]
        let network = if selected || manifest.id == "vpn" {
            let gateway = self.vpn.get_or_try_init(super::vpn::Gateway::open).await?;
            super::process::Network {
                proxy: selected.then_some(gateway.url.as_str()),
                gateway: (manifest.id == "vpn").then_some(gateway),
            }
        } else {
            Default::default()
        };
        #[cfg(not(target_os = "linux"))]
        let network = Default::default();
        let (process, registration) = Process::start(
            &slot.join("package"),
            &slot.join("data"),
            manifest,
            sound,
            self.registry.changes.clone(),
            network,
        )
        .await?;
        Ok(Active {
            voice: Arc::default(),
            process,
            registration,
            revision,
            connections,
        })
    }

    async fn activate(&self, manifest: Manifest, notice: Option<&'static str>) -> bool {
        self.publish(manifest.clone(), None, Status::Starting, notice);
        match self.start(&manifest).await {
            Ok(active) => {
                log::info!(
                    "Plugin ready id={} version={}",
                    manifest.id,
                    manifest.version
                );
                self.publish(manifest, Some(active), Status::Ready, notice);
                true
            }
            Err(error) => {
                log::error!("Could not activate plugin {}: {error}", manifest.id);
                self.publish(
                    manifest,
                    None,
                    Status::Error,
                    Some(Failure::StartFailed.code()),
                );
                false
            }
        }
    }
}
