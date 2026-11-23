use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{RwLock, RwLockReadGuard},
};

use goodwin_plugin::provider::Capability;
use serde::Serialize;

use super::{
    Failure,
    package::{Label, Manifest, Web},
    process::{Page, Process, Registration},
};

#[derive(Default)]
pub(super) struct Registry(RwLock<BTreeMap<String, Entry>>);

impl Registry {
    pub(super) fn read(&self) -> RwLockReadGuard<'_, BTreeMap<String, Entry>> {
        self.0.read().expect("plugin registry")
    }

    pub(super) fn catalog(&self) -> Vec<View> {
        self.read().values().map(Entry::view).collect()
    }

    pub(super) fn remove(&self, id: &str) {
        self.0.write().expect("plugin registry").remove(id);
    }

    pub(super) fn take(&self) -> BTreeMap<String, Entry> {
        std::mem::take(&mut *self.0.write().expect("plugin registry"))
    }

    pub(super) fn insert(
        &self,
        manifest: Manifest,
        active: Option<Active>,
        status: Status,
        error: Option<&'static str>,
    ) {
        self.0.write().expect("plugin registry").insert(
            manifest.id.clone(),
            Entry {
                manifest,
                active,
                status,
                error,
            },
        );
    }

    pub(super) fn take_active(&self, id: &str, status: Status) -> Option<Active> {
        self.0
            .write()
            .expect("plugin registry")
            .get_mut(id)
            .and_then(|entry| {
                entry.status = status;
                entry.error = None;
                entry.active.take()
            })
    }
}

pub(super) struct Entry {
    pub(super) manifest: Manifest,
    pub(super) active: Option<Active>,
    status: Status,
    error: Option<&'static str>,
}

#[derive(Clone)]
pub(super) struct Active {
    pub(super) process: Process,
    pub(super) registration: Registration,
    pub(super) revision: String,
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
    web: Option<Web>,
    pages: Vec<Page>,
    providers: BTreeSet<Capability>,
    model_catalog: bool,
    speech_streaming: bool,
    has_settings: bool,
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
            web: active.and(self.manifest.web.clone()),
            pages: active.map_or_else(Vec::new, |active| active.registration.pages.clone()),
            providers: active.map_or_else(BTreeSet::new, |active| {
                active.registration.providers.clone()
            }),
            model_catalog: active.is_some_and(|active| active.registration.model_catalog),
            speech_streaming: active.is_some_and(|active| active.registration.speech_streaming),
            has_settings: !self.manifest.settings.is_empty(),
        }
    }
}
