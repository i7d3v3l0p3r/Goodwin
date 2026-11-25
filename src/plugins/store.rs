use std::{
    fs,
    path::{Path, PathBuf},
};

use goodwin_error::{Context, Result, ensure};

use super::{Failure, package};

pub(crate) struct Store {
    root: PathBuf,
}

impl Store {
    pub(crate) fn new(dir: &Path) -> Self {
        Self {
            root: dir.join("plugins"),
        }
    }

    pub(crate) fn installed(&self) -> PathBuf {
        self.root.join("installed")
    }
    fn backups(&self) -> PathBuf {
        self.root.join("rollback")
    }
    pub(crate) fn work(&self) -> PathBuf {
        self.root.join("work")
    }
    pub(crate) fn slot(&self, id: &str) -> PathBuf {
        self.installed().join(id)
    }
    pub(crate) fn backup(&self, id: &str) -> PathBuf {
        self.backups().join(id)
    }

    pub(crate) fn dirs(&self) -> Result<()> {
        for path in [self.installed(), self.backups(), self.work()] {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }

    pub(crate) fn recover(&self) -> Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        self.dirs()?;
        for backup in fs::read_dir(self.backups())? {
            let backup = backup?;
            if let Some(id) = backup.file_name().to_str().filter(|id| package::name(id)) {
                self.restore_backup(id)?;
            }
        }
        for item in fs::read_dir(self.work())? {
            let item = item?;
            if item.file_type()?.is_dir() {
                fs::remove_dir_all(item.path())?;
            }
        }
        Ok(())
    }

    pub(crate) fn restore_backup(&self, id: &str) -> Result<()> {
        let slot = self.slot(id);
        if slot.exists() {
            fs::remove_dir_all(&slot)?;
        }
        fs::rename(self.backup(id), slot).context("Restore previous plugin package and data")
    }
}

pub(crate) fn storage(error: impl std::fmt::Display) -> Failure {
    log::error!("Plugin storage: {error}");
    Failure::Storage
}

pub(crate) fn copy_data(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let to = destination.join(entry.file_name());
        if kind.is_dir() {
            fs::create_dir(&to)?;
            copy_data(&entry.path(), &to)?;
        } else {
            ensure!(
                kind.is_file(),
                "Plugin data must consist of regular files and directories"
            );
            fs::copy(entry.path(), &to)?;
            fs::OpenOptions::new().write(true).open(to)?.sync_all()?;
        }
    }
    Ok(())
}
