use std::{
    fs::{self, File},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use goodwin_error::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::sync::Notify;

use crate::wake_catalog::{Catalog, Runtime, Selection};

use super::{FRAME_SAMPLES, WakeModel, runtimes};

const MANIFEST: &str = "wake-model.json";
const OPERATION: &str = "wake-model-operation.json";

#[cfg(test)]
mod tests;

pub(crate) struct WakeModels {
    dir: PathBuf,
    state: Mutex<State>,
    installer: Mutex<()>,
    pub(crate) changed: Notify,
}

struct State {
    operation: Option<Operation>,
    pending: Option<PendingModel>,
    recover: bool,
}

pub(crate) struct PendingModel {
    pub selection: Selection,
    pub model: WakeModel,
    pub manifest: NamedTempFile,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum InstallError {
    Busy,
    Unavailable,
    CatalogChanged,
    Unknown,
    Failed,
    Interrupted,
}

#[derive(Deserialize, Serialize)]
struct Operation {
    id: String,
    phase: Phase,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Prepared {
        selection: Selection,
        previous: Option<Selection>,
    },
    Activated {
        selection: Selection,
    },
    Failed {
        error: InstallError,
    },
}

impl WakeModels {
    pub(crate) fn new(dir: &Path) -> Self {
        let mut operation: Option<Operation> =
            read_optional(dir, OPERATION).unwrap_or_else(|error| {
                log::warn!("Could not read wake model operation: {error:#}");
                None
            });
        if let Some(operation) = &mut operation
            && matches!(operation.phase, Phase::Preparing)
        {
            operation.phase = Phase::Failed {
                error: InstallError::Interrupted,
            };
        }
        let recover = operation
            .as_ref()
            .is_some_and(|operation| matches!(operation.phase, Phase::Prepared { .. }));
        Self {
            dir: dir.to_owned(),
            state: Mutex::new(State {
                operation,
                pending: None,
                recover,
            }),
            installer: Mutex::new(()),
            changed: Notify::new(),
        }
    }

    pub(crate) fn take_pending(&self) -> Option<PendingModel> {
        self.state.lock().expect("wake model state").pending.take()
    }

    pub(crate) fn initial_model(&self) -> Result<(WakeModel, Option<Selection>)> {
        let selected = self.read_active();
        self.resume_install(selected.as_ref());
        let (model, path) = if let Some(selection) = &selected {
            let path = model_path(&self.dir, selection);
            (WakeModel::load_selected(&path, selection)?, path)
        } else {
            let path = local_model_path(&self.dir);
            (WakeModel::load(&path)?, path)
        };
        println!("Loaded {} wake model from {}", model.kind(), path.display());
        Ok((model, selected))
    }

    pub(crate) fn check_installed(dir: &Path) -> Result<()> {
        let selection = read_selection(dir)?.context("No catalog wake model is installed")?;
        let path = model_path(dir, &selection);
        if selection.runtime == Runtime::MicroTflite {
            let mut model = WakeModel::load_selected(&path, &selection)?;
            model.predict(&[0; FRAME_SAMPLES])?;
        } else {
            // The running service may still hold an ONNX model in memory during an update.
            ensure!(path.is_file(), "Missing wake model {}", path.display());
        }
        Ok(())
    }

    pub(crate) fn check_installed_inference(dir: &Path) -> Result<()> {
        let mut model = if let Some(selection) = read_selection(dir)? {
            WakeModel::load_selected(&model_path(dir, &selection), &selection)?
        } else {
            let path = local_model_path(dir);
            if std::env::var_os("GOODWIN_WAKE_MODEL").is_none()
                && matches!(fs::symlink_metadata(&path), Err(error) if error.kind() == ErrorKind::NotFound)
            {
                return Ok(());
            }
            WakeModel::load(&path)?
        };
        model.predict(&[0; FRAME_SAMPLES])?;
        Ok(())
    }

    pub(crate) fn start_install(&self) -> Result<String, InstallError> {
        let _installer = self.installer.try_lock().map_err(|_| InstallError::Busy)?;
        if self
            .state
            .lock()
            .expect("wake model state")
            .operation
            .as_ref()
            .is_some_and(|operation| {
                matches!(operation.phase, Phase::Preparing | Phase::Prepared { .. })
            })
        {
            return Err(InstallError::Busy);
        }
        let id = URL_SAFE_NO_PAD
            .encode(crate::panel_core::random_bytes::<16>().map_err(install_failed)?);
        if !self
            .save_operation(Operation {
                id: id.clone(),
                phase: Phase::Preparing,
            })
            .map_err(install_failed)?
        {
            self.fail_install(InstallError::Failed);
            return Err(InstallError::Failed);
        }
        Ok(id)
    }

    pub(crate) fn install(&self, tag: &str, id: &str) {
        if let Err(error) = self.prepare_install(tag, id) {
            self.fail_install(error);
        }
    }

    pub(crate) fn install_result(
        &self,
        id: &str,
    ) -> Option<Result<Option<Selection>, InstallError>> {
        let state = self.state.lock().expect("wake model state");
        let operation = state
            .operation
            .as_ref()
            .filter(|operation| operation.id == id)?;
        Some(match &operation.phase {
            Phase::Preparing | Phase::Prepared { .. } => Ok(None),
            Phase::Activated { selection } => Ok(Some(selection.clone())),
            Phase::Failed { error } => Err(*error),
        })
    }

    fn prepare_install(&self, tag: &str, id: &str) -> Result<(), InstallError> {
        let catalog = Catalog::latest(runtimes()).map_err(|error| {
            log::warn!("Could not get the wake model catalog: {error:#}");
            InstallError::Unavailable
        })?;
        if catalog.tag != tag {
            return Err(InstallError::CatalogChanged);
        }
        if !catalog.models.iter().any(|model| model.selection.id == id) {
            return Err(InstallError::Unknown);
        }
        self.install_from(&catalog, id).map_err(install_failed)
    }

    fn install_from(&self, catalog: &Catalog, id: &str) -> Result<()> {
        let models = self.dir.join("wake-models");
        fs::create_dir_all(&models)?;
        let mut model_file = NamedTempFile::new_in(&models)?;
        let selected = catalog.download(id, model_file.as_file_mut())?;
        let model = checked_model(model_file.path(), selected)?;
        let previous = self.read_active();
        let target = model_path(&self.dir, selected);
        model_file.as_file().sync_all()?;
        model_file.persist(&target)?;
        crate::persistence::sync_directory(&models)?;
        self.stage_model(selected.clone(), model, previous)
    }

    pub(crate) fn commit_selection(&self, manifest: NamedTempFile) -> Result<()> {
        // The capture task has swapped its detector before reaching the commit point.
        manifest.persist(self.dir.join(MANIFEST))?;
        // Saving the terminal operation synchronizes the directory for both renames.
        Ok(())
    }

    pub(crate) fn finish_install(&self, selection: Selection) {
        let _installer = self.installer.lock().expect("wake model installer");
        let previous = match self.phase() {
            Some(Phase::Prepared { previous, .. }) => previous,
            _ => unreachable!("activation requires a prepared wake model"),
        };
        let target = model_path(&self.dir, &selection);
        if self.complete(Phase::Activated { selection })
            && let Some(previous) = previous
            && let old = model_path(&self.dir, &previous)
            && old != target
            && let Err(error) = fs::remove_file(old)
            && error.kind() != ErrorKind::NotFound
        {
            log::warn!("Could not remove the previous wake model: {error}");
        }
    }

    pub(crate) fn fail_install(&self, error: InstallError) {
        self.complete(Phase::Failed { error });
    }

    fn resume_install(&self, active: Option<&Selection>) {
        if !std::mem::take(&mut self.state.lock().expect("wake model state").recover) {
            return;
        }
        let Some(Phase::Prepared {
            selection,
            previous,
        }) = self.phase()
        else {
            return;
        };
        if active != previous.as_ref() && active != Some(&selection) {
            // An offline installer may have selected a newer model.
            self.fail_install(InstallError::Interrupted);
            return;
        }
        let result = (|| {
            selection.validate()?;
            if let Some(previous) = &previous {
                previous.validate()?;
            }
            let model = checked_model(&model_path(&self.dir, &selection), &selection)?;
            self.queue_model(selection, model)
        })();
        if let Err(error) = result {
            self.fail_install(install_failed(error));
        }
    }

    fn read_active(&self) -> Option<Selection> {
        read_selection(&self.dir).unwrap_or_else(|error| {
            log::warn!("Could not read wake model selection: {error:#}");
            None
        })
    }

    fn stage_model(
        &self,
        selection: Selection,
        model: WakeModel,
        previous: Option<Selection>,
    ) -> Result<()> {
        let candidate = self.candidate(selection, model)?;
        ensure!(
            self.update_phase(Phase::Prepared {
                selection: candidate.selection.clone(),
                previous
            })?,
            "Could not synchronize the staged wake model operation"
        );
        self.enqueue(candidate);
        Ok(())
    }

    fn queue_model(&self, selection: Selection, model: WakeModel) -> Result<()> {
        self.enqueue(self.candidate(selection, model)?);
        Ok(())
    }

    fn candidate(&self, selection: Selection, model: WakeModel) -> Result<PendingModel> {
        let manifest = prepare_json(&self.dir, &selection)?;
        Ok(PendingModel {
            selection,
            model,
            manifest,
        })
    }

    fn enqueue(&self, candidate: PendingModel) {
        self.state.lock().expect("wake model state").pending = Some(candidate);
        self.changed.notify_one();
    }

    fn phase(&self) -> Option<Phase> {
        self.state
            .lock()
            .expect("wake model state")
            .operation
            .as_ref()
            .map(|operation| operation.phase.clone())
    }

    fn update_phase(&self, phase: Phase) -> Result<bool> {
        let id = self
            .state
            .lock()
            .expect("wake model state")
            .operation
            .as_ref()
            .expect("an active wake model operation")
            .id
            .clone();
        self.save_operation(Operation { id, phase })
    }

    fn complete(&self, phase: Phase) -> bool {
        match self.update_phase(phase.clone()) {
            Ok(durable) => durable,
            Err(error) => {
                log::warn!("Could not persist the wake model operation result: {error:#}");
                self.state
                    .lock()
                    .expect("wake model state")
                    .operation
                    .as_mut()
                    .expect("an active wake model operation")
                    .phase = phase;
                false
            }
        }
    }

    fn save_operation(&self, operation: Operation) -> Result<bool> {
        prepare_json(&self.dir, &operation)?.persist(self.dir.join(OPERATION))?;
        let durable = crate::persistence::sync_directory(&self.dir);
        if let Err(error) = &durable {
            log::warn!("Could not synchronize the wake model operation: {error}");
        }
        self.state.lock().expect("wake model state").operation = Some(operation);
        Ok(durable.is_ok())
    }
}

fn install_failed(error: goodwin_error::Error) -> InstallError {
    log::warn!("Could not install the wake model: {error:#}");
    InstallError::Failed
}

fn checked_model(path: &Path, selection: &Selection) -> Result<WakeModel> {
    let mut model = WakeModel::load_selected(path, selection)?;
    model.predict(&[0; FRAME_SAMPLES])?;
    model.reset()?;
    Ok(model)
}

fn prepare_json(dir: &Path, value: &impl Serialize) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new_in(dir)?;
    goodwin_json::to_writer(file.as_file_mut(), value)?;
    file.as_file().sync_all()?;
    Ok(file)
}

fn model_path(dir: &Path, selected: &Selection) -> PathBuf {
    dir.join("wake-models")
        .join(format!("{}.{}", selected.sha256, selected.runtime.format()))
}

fn read_selection(dir: &Path) -> Result<Option<Selection>> {
    let selection: Option<Selection> = read_optional(dir, MANIFEST)?;
    if let Some(selection) = &selection {
        selection.validate()?;
    }
    Ok(selection)
}

fn read_optional<T: serde::de::DeserializeOwned>(dir: &Path, path: &str) -> Result<Option<T>> {
    let file = match File::open(dir.join(path)) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(
        goodwin_json::from_reader(file.take(4096)).with_context(|| format!("Read {path}"))?,
    ))
}

fn local_model_path(dir: &Path) -> PathBuf {
    std::env::var_os("GOODWIN_WAKE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("goodwin.onnx"))
}
