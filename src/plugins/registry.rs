use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{RwLock, RwLockReadGuard},
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
        entries
            .values()
            .map(|entry| entry.view(Self::connection_owner(&entries, &entry.manifest.id)))
            .collect()
    }

    pub(super) fn connection_owner(
        entries: &BTreeMap<String, Entry>,
        provider: &str,
    ) -> Option<&'static str> {
        const UNIFIED: &str = "openai-compatible";
        (provider == "openrouter"
            && entries
                .get(UNIFIED)
                .is_some_and(|entry| entry.manifest.provider_connections))
        .then_some(UNIFIED)
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
    ) {
        self.entries.write().expect("plugin registry").insert(
            manifest.id.clone(),
            Entry {
                manifest,
                active,
                status,
                error,
                settings_revision: 0,
            },
        );
        self.changes.send_replace(());
    }

    pub(super) fn take_active(&self, id: &str, status: Status) -> Option<Active> {
        let mut entries = self.entries.write().expect("plugin registry");
        let entry = entries.get_mut(id)?;
        entry.status = status;
        entry.error = None;
        let active = entry.active.take();
        self.changes.send_replace(());
        active
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

    pub(super) fn settings_changed(&self, id: &str) {
        if let Some(entry) = self.entries.write().expect("plugin registry").get_mut(id) {
            entry.settings_revision += 1;
            self.changes.send_replace(());
        }
    }
}

pub(super) struct Entry {
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
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Starting,
    Ready,
    Updating,
    Removing,
    Error,
}

#[derive(Serialize)]
pub struct View {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_redirect: Option<&'static str>,
    connection_settings: bool,
    connections: Vec<connections::View>,
}

impl Entry {
    pub(super) fn live(&self) -> Option<&Active> {
        self.active
            .as_ref()
            .filter(|active| active.process.running())
    }

    fn view(&self, settings_redirect: Option<&'static str>) -> View {
        let active = self.live();
        let provider = active.filter(|_| settings_redirect.is_none());
        let stopped = self.active.is_some() && active.is_none();
        View {
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
            providers: provider.map_or_else(BTreeSet::new, |active| {
                active.registration.providers.clone()
            }),
            model_catalog: provider.is_some_and(|active| active.registration.model_catalog),
            speech_streaming: provider.is_some_and(|active| active.registration.speech_streaming),
            has_settings: settings_redirect.is_some()
                || self.manifest.provider_connections
                || !self.manifest.settings.is_empty(),
            settings_redirect,
            connection_settings: self.manifest.provider_connections,
            connections: active.map_or_else(Vec::new, |active| active.connections.clone()),
        }
    }
}
