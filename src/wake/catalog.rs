use std::{
    fs::{self, File},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};

use goodwin_error::{Context, Result, ensure};
use tempfile::NamedTempFile;
use tokio::sync::{Notify, oneshot};

use crate::wake_catalog::{Catalog, Runtime, Selection};

use super::{FRAME_SAMPLES, WakeModel, runtimes};

const MANIFEST: &str = "wake-model.json";

pub(crate) struct WakeModels {
    dir: PathBuf,
    state: Mutex<State>,
    installer: Mutex<()>,
    pub(crate) changed: Notify,
}

struct State {
    selected: Option<Selection>,
    pending: Option<PendingModel>,
}

pub(crate) struct PendingModel {
    pub selection: Selection,
    pub model: WakeModel,
    pub activated: oneshot::Sender<()>,
}

pub(crate) enum InstallError {
    Busy,
    Unavailable(goodwin_error::Error),
    CatalogChanged,
    Unknown,
    Failed(goodwin_error::Error),
}

impl WakeModels {
    pub(crate) fn new(dir: &Path) -> Self {
        let selected = read_selection(dir).unwrap_or_else(|error| {
            log::warn!("Could not read wake model selection: {error}");
            None
        });
        Self {
            dir: dir.to_owned(),
            state: Mutex::new(State {
                selected,
                pending: None,
            }),
            installer: Mutex::new(()),
            changed: Notify::new(),
        }
    }

    fn selected(&self) -> Option<Selection> {
        self.state
            .lock()
            .expect("wake model state")
            .selected
            .clone()
    }

    pub(crate) fn take_pending(&self) -> Option<PendingModel> {
        self.state.lock().expect("wake model state").pending.take()
    }

    pub(crate) fn initial_model(&self) -> Result<(WakeModel, Option<Selection>)> {
        let selected = self.selected();
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

    pub(crate) fn install(
        &self,
        tag: &str,
        id: &str,
    ) -> Result<(Selection, oneshot::Receiver<()>), InstallError> {
        let _installer = self.installer.try_lock().map_err(|_| InstallError::Busy)?;
        if self
            .state
            .lock()
            .expect("wake model state")
            .pending
            .is_some()
        {
            return Err(InstallError::Busy);
        }
        let catalog = Catalog::latest(runtimes()).map_err(InstallError::Unavailable)?;
        if catalog.tag != tag {
            return Err(InstallError::CatalogChanged);
        }
        if !catalog.models.iter().any(|model| model.selection.id == id) {
            return Err(InstallError::Unknown);
        }
        self.install_from(&catalog, id)
            .map_err(InstallError::Failed)
    }

    fn install_from(
        &self,
        catalog: &Catalog,
        id: &str,
    ) -> Result<(Selection, oneshot::Receiver<()>)> {
        let models = self.dir.join("wake-models");
        fs::create_dir_all(&models)?;
        let mut model_file = NamedTempFile::new_in(&models)?;
        let selected = catalog.download(id, model_file.as_file_mut())?;
        let mut model = WakeModel::load_selected(model_file.path(), selected)?;
        model.predict(&[0; FRAME_SAMPLES])?;
        model.reset()?;

        let previous = self.selected();
        let target = model_path(&self.dir, selected);
        let mut manifest = NamedTempFile::new_in(&self.dir)?;
        goodwin_json::to_writer(manifest.as_file_mut(), selected)?;
        manifest.as_file().sync_all()?;
        model_file.as_file().sync_all()?;
        model_file.persist(&target)?;
        if let Err(error) = manifest.persist(self.dir.join(MANIFEST)) {
            if previous
                .as_ref()
                .is_none_or(|old| model_path(&self.dir, old) != target)
            {
                let _ = fs::remove_file(&target);
            }
            return Err(error.into());
        }

        let activated = self.queue_model(selected.clone(), model);
        if let Some(old) = previous
            .map(|old| model_path(&self.dir, &old))
            .filter(|old| *old != target)
            && let Err(error) = fs::remove_file(old)
            && error.kind() != ErrorKind::NotFound
        {
            log::warn!("Could not remove the previous wake model: {error}");
        }
        Ok((selected.clone(), activated))
    }

    pub(crate) fn queue_model(
        &self,
        selection: Selection,
        model: WakeModel,
    ) -> oneshot::Receiver<()> {
        let (activated, confirmation) = oneshot::channel();
        {
            let mut state = self.state.lock().expect("wake model state");
            state.selected = Some(selection.clone());
            state.pending = Some(PendingModel {
                selection,
                model,
                activated,
            });
        }
        self.changed.notify_one();
        confirmation
    }
}

fn model_path(dir: &Path, selected: &Selection) -> PathBuf {
    dir.join("wake-models")
        .join(format!("{}.{}", selected.sha256, selected.runtime.format()))
}

fn read_selection(dir: &Path) -> Result<Option<Selection>> {
    let file = match File::open(dir.join(MANIFEST)) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let selection: Selection =
        goodwin_json::from_reader(file.take(4096)).context("Read wake model manifest")?;
    selection.validate()?;
    Ok(Some(selection))
}

fn local_model_path(dir: &Path) -> PathBuf {
    std::env::var_os("GOODWIN_WAKE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("goodwin.onnx"))
}
