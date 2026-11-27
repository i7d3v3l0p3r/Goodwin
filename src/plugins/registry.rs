use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock, RwLockReadGuard},
};

use goodwin_plugin::provider::Capability;
use serde::Serialize;
use tokio::sync::watch;

use super::{
    Failure, connections,
    package::{Label, Manifest, Web},
    process::{Page, Process, Registration},
};

#[derive(Default)]
pub(super) struct Registry {
    entries: RwLock<BTreeMap<String, Entry>>,
    pub(super) changes: watch::Sender<()>,
}

impl Registry {
    pub(super) fn read(&self) -> RwLockReadGuard<'_, BTreeMap<String, Entry>> {
        self.entries.read().expect("plugin registry")
    }

    pub(super) fn catalog(&self) -> Vec<View> {
        let entries = self.read();
        entries.values().map(Entry::view).collect()
    }

    pub(super) fn remove(&self, id: &str) {
        self.entries.write().expect("plugin registry").remove(id);
        self.changes.send_replace(());
    }

    pub(super) fn take(&self) -> BTreeMap<String, Entry> {
        let entries = std::mem::take(&mut *self.entries.write().expect("plugin registry"));
        self.changes.send_replace(());
        entries
    }

    pub(super) fn insert(
        &self,
        manifest: Manifest,
        active: Option<Active>,
        status: Status,
        error: Option<&'static str>,
        vpn: Option<bool>,
    ) {
        self.entries.write().expect("plugin registry").insert(
            manifest.id.clone(),
            Entry {
                manifest,
                active,
                status,
                error,
                vpn,
                settings_revision: 0,
            },
        );
        self.changes.send_replace(());
    }

    pub(super) fn take_active(&self, id: &str, status: Status) -> Result<Option<Active>, Failure> {
        let mut entries = self.entries.write().expect("plugin registry");
        let Some(entry) = entries.get_mut(id) else {
            return Ok(None);
        };
        if entry.active.as_ref().is_some_and(Active::used_by_voice) {
            return Err(Failure::Busy);
        }
        entry.status = status;
        entry.error = None;
        let active = entry.active.take();
        self.changes.send_replace(());
        Ok(active)
    }

    pub(super) fn set_connections(&self, id: &str, mut views: Vec<connections::View>) {
        let mut entries = self.entries.write().expect("plugin registry");
        if let Some(active) = entries.get_mut(id).and_then(|entry| entry.active.as_mut()) {
            for view in &mut views {
                if let Some(previous) = active.connections.iter().find(|previous| {
                    previous.id == view.id
                        && previous.request_configuration == view.request_configuration
                }) {
                    view.request_revision = previous.request_revision;
                }
            }
            active.connections = views;
            self.changes.send_replace(());
        }
    }

    pub(super) fn update_settings<T>(
        &self,
        id: &str,
        write: impl FnOnce() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let mut entries = self.entries.write().expect("plugin registry");
        let entry = entries.get_mut(id).ok_or(Failure::NotFound)?;
        // Native providers read the settings file themselves. Keep the file
        // stable while a voice request holds the package.
        if !entry.manifest.is_wasm() && entry.active.as_ref().is_some_and(Active::used_by_voice) {
            return Err(Failure::Busy);
        }
        let result = write()?;
        entry.settings_revision += 1;
        self.changes.send_replace(());
        Ok(result)
    }
}

pub(super) struct Entry {
    pub(super) vpn: Option<bool>,
    pub(super) manifest: Manifest,
    pub(super) active: Option<Active>,
    pub(super) settings_revision: u64,
    status: Status,
    error: Option<&'static str>,
}

#[derive(Clone)]
pub(super) struct Active {
    pub(super) process: Process,
    pub(super) registration: Registration,
    pub(super) revision: String,
    pub(super) connections: Vec<connections::View>,
    pub(super) voice: Arc<()>,
}

impl Active {
    fn used_by_voice(&self) -> bool {
        Arc::strong_count(&self.voice) > 1
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Disabled,
    Starting,
    Ready,
    Updating,
    Removing,
    Error,
}

#[derive(Serialize)]
pub struct View {
    #[serde(skip_serializing_if = "Option::is_none")]
    vpn: Option<bool>,
    id: String,
    title: Label,
    version: String,
    status: Status,
    error: Option<&'static str>,
    revision: Option<String>,
    settings_revision: u64,
    web: Option<Web>,
    pages: Vec<Page>,
    providers: BTreeSet<Capability>,
    model_catalog: bool,
    speech_streaming: bool,
    has_settings: bool,
    connection_settings: bool,
    connections: Vec<connections::View>,
}

impl Entry {
    pub(super) fn live(&self) -> Option<&Active> {
        self.active
            .as_ref()
            .filter(|active| active.process.running())
    }

    fn view(&self) -> View {
        let active = self.live();
        let stopped = self.active.is_some() && active.is_none();
        View {
            vpn: self.vpn,
            id: self.manifest.id.clone(),
            title: self.manifest.title.clone(),
            version: self.manifest.version.clone(),
            status: if stopped { Status::Error } else { self.status },
            error: if stopped {
                Some(Failure::Unavailable.code())
            } else {
                self.error
            },
            revision: active.map(|active| active.revision.clone()),
            settings_revision: self.settings_revision,
            web: active.and(self.manifest.web.clone()),
            pages: active.map_or_else(Vec::new, |active| active.registration.pages.clone()),
            providers: active.map_or_else(BTreeSet::new, |active| {
                active.registration.providers.clone()
            }),
            model_catalog: active.is_some_and(|active| active.registration.model_catalog),
            speech_streaming: active.is_some_and(|active| active.registration.speech_streaming),
            has_settings: self.manifest.provider_connections || !self.manifest.settings.is_empty(),
            connection_settings: self.manifest.provider_connections,
            connections: active.map_or_else(Vec::new, |active| active.connections.clone()),
        }
    }
}
