use std::{fs, path::Path, sync::Arc};

use anyhow::{Context, Result};
use goodwin_plugin::{
    provider::{Capability, ModelCatalog},
    settings::SettingsData,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, mpsc};

use crate::{
    config::random_bytes,
    station::{Player, timer_signal},
};

use super::{
    Failure, VoiceTools,
    package::{self, Manifest},
    process::Process,
    registry::{Active, Registry, Status, View},
    settings::{self, Field},
    store::{Store, copy_data, storage},
};

#[derive(Clone)]
pub struct Plugins(Arc<Manager>);

struct Manager {
    store: Store,
    registry: Registry,
    lifecycle: Mutex<()>,
    player: Player,
    sound: Mutex<Option<mpsc::Sender<()>>>,
}

#[derive(Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Option<Value>,
}

impl Plugins {
    pub fn new(dir: &Path, player: Player) -> Self {
        Self(Arc::new(Manager {
            store: Store::new(dir),
            registry: Registry::default(),
            lifecycle: Mutex::default(),
            player,
            sound: Mutex::default(),
        }))
    }

    pub async fn restore(&self) {
        let _operation = self.0.lifecycle.lock().await;
        if let Err(error) = self.0.store.recover() {
            eprintln!("Recover plugin packages: {error:#}");
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
            if !ids.is_empty() && !ids.contains(&id.as_str()) {
                continue;
            }
            let manifest = Manifest::read(&item.path().join("package"));
            match manifest {
                Ok(manifest) if manifest.id == id => {
                    self.0.activate(manifest, None).await;
                }
                Ok(_) => eprintln!("Plugin {id} has a mismatched manifest identity"),
                Err(error) => eprintln!("Read plugin {id}: {error:#}"),
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

    fn settings_fields(&self, id: &str) -> Result<Vec<Field>, Failure> {
        let registry = self.0.registry.read();
        let fields = &registry.get(id).ok_or(Failure::NotFound)?.manifest.settings;
        if fields.is_empty() {
            return Err(Failure::NotFound);
        }
        Ok(fields.clone())
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
        tokio::task::spawn_blocking(move || {
            let data = SettingsData::read(&directory).map_err(storage)?;
            let data = settings::apply(&fields, data, patch)?;
            data.save(&directory).map_err(storage)?;
            Ok(settings::View::new(fields, data))
        })
        .await
        .map_err(storage)?
    }

    pub(crate) fn provider(&self, id: &str, capability: Capability) -> Option<(Process, bool)> {
        let registry = self.0.registry.read();
        let active = registry.get(id)?.live()?;
        active
            .registration
            .providers
            .contains(&capability)
            .then(|| (active.process.clone(), active.registration.model_catalog))
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
        let manifest = serde_json::from_value(json!({
            "id": id, "version": "1.0.0", "apiVersion": 1,
            "title": {"en": "Fixture provider", "ru": "Тестовый провайдер"},
            "target": "fixture", "executable": "bin/fixture-provider",
        }))
        .unwrap();
        let registration = serde_json::from_value(json!({
            "apiVersion": 1, "providers": ["transcription", "chat", "speech"], "modelCatalog": true,
        }))
        .unwrap();
        self.0.registry.insert(
            manifest,
            Some(Active {
                process,
                registration,
                revision: "fixture".into(),
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
        method: &str,
        path: &str,
        query: Option<&str>,
        body: Value,
    ) -> Result<HttpResponse, Failure> {
        let process = {
            let entries = self.0.registry.read();
            let entry = entries.get(id).ok_or(Failure::NotFound)?;
            let active = entry.live().ok_or(Failure::Unavailable)?;
            if !active
                .registration
                .routes
                .iter()
                .any(|route| route.accepts(method, path))
            {
                return Err(Failure::NotFound);
            }
            active.process.clone()
        };
        let result = process
            .call(
                "http",
                json!({ "method": method, "path": path, "query": query, "body": body }),
            )
            .await
            .map_err(|_| Failure::Unavailable)?;
        serde_json::from_value(result).map_err(|_| Failure::Unavailable)
    }

    pub async fn upload(&self, bytes: axum::body::Bytes) -> Result<(), Failure> {
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
                eprintln!("Read plugin upload: {error:#}");
                Failure::InvalidPackage
            })?;
        fs::create_dir(staged.join("data")).map_err(storage)?;
        let id = &manifest.id;
        let previous = self.0.store.slot(id);
        if !previous.exists() {
            fs::rename(&staged, &previous).map_err(storage)?;
            return if self.0.activate(manifest, None).await {
                Ok(())
            } else {
                Err(Failure::StartFailed)
            };
        }

        let backup = self.0.store.backup(id);
        if backup.exists() {
            return Err(Failure::Storage);
        }
        let old = Manifest::read(&previous.join("package")).map_err(storage)?;
        self.0.withdraw(id, Status::Updating).await;
        let source = previous.join("data");
        let destination = staged.join("data");
        let prepared = tokio::task::spawn_blocking(move || {
            copy_data(&source, &destination).context("Copy plugin data for update")
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|result| result)
        .and_then(|()| fs::rename(&previous, &backup).context("Save the previous plugin package"));
        if let Err(error) = prepared {
            self.0.activate(old, Some(Failure::Storage.code())).await;
            return Err(storage(error));
        }
        let ready = match fs::rename(&staged, &previous) {
            Ok(()) => self.0.start(&manifest).await,
            Err(error) => Err(error.into()),
        };
        match ready {
            Ok(active) => match fs::rename(&backup, work.path().join("retired")) {
                Ok(()) => {
                    self.0
                        .registry
                        .insert(manifest, Some(active), Status::Ready, None);
                    return Ok(());
                }
                Err(error) => {
                    active.process.stop().await;
                    eprintln!("Commit plugin {id} update: {error}");
                }
            },
            Err(error) => eprintln!("Start updated plugin {id}: {error:#}"),
        }
        if let Err(error) = self.0.store.restore_backup(id) {
            self.0
                .registry
                .insert(old, None, Status::Error, Some(Failure::Storage.code()));
            return Err(storage(error));
        }
        if self.0.activate(old, Some(Failure::RolledBack.code())).await {
            Err(Failure::RolledBack)
        } else {
            Err(Failure::StartFailed)
        }
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
        if self.0.store.backup(id).exists() {
            return Err(Failure::Storage);
        }
        let work = tempfile::tempdir_in(self.0.store.work()).map_err(storage)?;
        self.0.withdraw(id, Status::Removing).await;
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
    async fn withdraw(&self, id: &str, status: Status) {
        if let Some(active) = self.registry.take_active(id, status) {
            active.process.stop().await;
        }
    }

    async fn start(&self, manifest: &Manifest) -> Result<Active> {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let revision = URL_SAFE_NO_PAD.encode(random_bytes::<12>()?);
        let slot = self.store.slot(&manifest.id);
        let sound = {
            let mut sound = self.sound.lock().await;
            sound
                .get_or_insert_with(|| {
                    let (send, mut receive) = mpsc::channel(64);
                    let player = self.player.clone();
                    tokio::spawn(async move {
                        let signal = timer_signal();
                        while receive.recv().await.is_some() {
                            if let Err(error) = player.play(&signal).await {
                                eprintln!("Play timer signal: {error:#}");
                            }
                        }
                    });
                    send
                })
                .clone()
        };
        let (process, registration) =
            Process::start(&slot.join("package"), &slot.join("data"), manifest, sound).await?;
        Ok(Active {
            process,
            registration,
            revision,
        })
    }

    async fn activate(&self, manifest: Manifest, notice: Option<&'static str>) -> bool {
        self.registry
            .insert(manifest.clone(), None, Status::Starting, notice);
        match self.start(&manifest).await {
            Ok(active) => {
                self.registry
                    .insert(manifest, Some(active), Status::Ready, notice);
                true
            }
            Err(error) => {
                eprintln!("Activate plugin {}: {error:#}", manifest.id);
                self.registry.insert(
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
