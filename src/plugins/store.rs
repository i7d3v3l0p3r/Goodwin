use std::{
    fs,
    path::{Path, PathBuf},
};

use goodwin_error::{Context, Result, ensure};

use super::{Failure, package};

#[cfg(any(test, target_os = "espidf"))]
pub(crate) mod fat_files;

#[cfg(all(
    feature = "wasm-plugins",
    any(target_os = "linux", target_os = "espidf")
))]
pub(crate) mod quota;

#[cfg(target_os = "espidf")]
pub(crate) const DATA_BYTES: u64 = 2 * 1024 * 1024;
#[cfg(not(target_os = "espidf"))]
pub(crate) const DATA_BYTES: u64 = 132 * 1024 * 1024;
const DATA_ENTRIES: usize = 8192;
const DATA_DEPTH: usize = 16;

#[derive(Clone)]
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
    fn pending_installs(&self) -> PathBuf {
        self.root.join("pending-installs")
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
        for path in [
            self.installed(),
            self.backups(),
            self.pending_installs(),
            self.work(),
        ] {
            fs::create_dir_all(path)?;
        }
        sync_directory(&self.root)?;
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
        for marker in fs::read_dir(self.pending_installs())? {
            let marker = marker?;
            if let Some(id) = marker.file_name().to_str().filter(|id| package::name(id)) {
                discard_install(&self.slot(id), &marker.path())?;
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

    pub(crate) fn unresolved(&self, id: &str) -> Result<bool> {
        Ok(self.backup(id).try_exists()? || self.pending_installs().join(id).try_exists()?)
    }

    pub(crate) fn update(&self, id: &str, work: &Path) -> Result<Update> {
        ensure!(!self.unresolved(id)?, "Plugin update requires recovery");
        let slot = self.slot(id);
        Ok(Update {
            existed: slot.try_exists()?,
            slot,
            staged: work.join("slot"),
            backup: self.backup(id),
            pending: self.pending_installs().join(id),
            retired: work.join("retired"),
        })
    }

    fn restore_backup(&self, id: &str) -> Result<()> {
        restore_backup(&self.slot(id), &self.backup(id))
    }
}

/// The recovery record stays outside the work directory until a successful start.
/// Callers must quiesce the current plugin for the lifetime of this operation.
pub(crate) struct Update {
    existed: bool,
    slot: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    pending: PathBuf,
    retired: PathBuf,
}

impl Update {
    pub(crate) fn prepare(&self) -> Result<()> {
        let data = self.staged.join("data");
        fs::create_dir(&data)?;
        if self.existed {
            let mut budget = DataBudget {
                bytes: DATA_BYTES,
                entries: DATA_ENTRIES,
            };
            copy_data(&self.slot.join("data"), &data, &mut budget, DATA_DEPTH)?;
        }
        sync_tree(&self.staged)
    }

    fn record(&self) -> &Path {
        if self.existed {
            &self.backup
        } else {
            &self.pending
        }
    }

    fn begin(&self) -> Result<()> {
        if self.existed {
            rename(&self.slot, &self.backup).context("Save previous plugin package and data")
        } else {
            fs::create_dir(&self.pending)?;
            sync_directory(parent(&self.pending))
        }
    }

    pub(crate) fn publish(&self) -> Result<()> {
        self.begin()?;
        rename(&self.staged, &self.slot).context("Publish candidate plugin package")
    }

    pub(crate) fn commit(&self) -> Result<()> {
        fs::rename(self.record(), &self.retired).context("Commit plugin update")?;
        // Both paths refer to the same filesystem. After the rename the old
        // package is no longer a recovery record, even if syncing fails.
        for directory in [parent(self.record()), parent(&self.retired)] {
            if let Err(error) = sync_directory(directory) {
                log::warn!("Synchronize committed plugin update: {error:#}");
            }
        }
        Ok(())
    }

    pub(crate) fn rollback(&self) -> Result<()> {
        if self.existed {
            if self.backup.try_exists()? {
                restore_backup(&self.slot, &self.backup)?;
            }
        } else if self.pending.try_exists()? {
            discard_install(&self.slot, &self.pending)?;
        }
        Ok(())
    }
}

fn discard_install(slot: &Path, marker: &Path) -> Result<()> {
    remove_tree(slot)?;
    sync_directory(parent(slot))?;
    fs::remove_dir(marker)?;
    sync_directory(parent(marker))
}

fn restore_backup(slot: &Path, backup: &Path) -> Result<()> {
    remove_tree(slot)?;
    rename(backup, slot).context("Restore previous plugin package and data")
}

fn remove_tree(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn parent(path: &Path) -> &Path {
    path.parent().expect("plugin store path has a parent")
}

fn rename(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to)?;
    sync_directory(parent(to))?;
    sync_directory(parent(from))
}

fn sync_directory(_path: &Path) -> Result<()> {
    // ESP-IDF's FAT VFS exposes file sync but no directory file handles.
    #[cfg(not(target_os = "espidf"))]
    crate::persistence::sync_directory(_path)?;
    Ok(())
}

fn sync_tree(path: &Path) -> Result<()> {
    #[cfg(not(target_os = "espidf"))]
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_tree(&entry.path())?;
        }
    }
    sync_directory(path)
}

pub(crate) fn storage(error: impl std::fmt::Display) -> Failure {
    log::error!("Plugin storage: {error}");
    Failure::Storage
}

struct DataBudget {
    bytes: u64,
    entries: usize,
}

fn copy_data(
    source: &Path,
    destination: &Path,
    budget: &mut DataBudget,
    depth: usize,
) -> Result<()> {
    ensure!(depth > 0, "Plugin data exceeds maximum directory depth");
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        budget.entries = budget
            .entries
            .checked_sub(1)
            .context("Too many plugin data entries")?;
        let kind = entry.file_type()?;
        let to = destination.join(entry.file_name());
        if kind.is_dir() {
            fs::create_dir(&to)?;
            copy_data(&entry.path(), &to, budget, depth - 1)?;
        } else {
            ensure!(
                kind.is_file(),
                "Plugin data must consist of regular files and directories"
            );
            let size = entry.metadata()?.len();
            budget.bytes = budget
                .bytes
                .checked_sub(size)
                .context("Plugin data exceeds update limit")?;
            ensure!(
                fs::copy(entry.path(), &to)? == size,
                "Plugin data changed while being copied"
            );
            fs::OpenOptions::new().write(true).open(to)?.sync_all()?;
        }
    }
    Ok(())
}

#[cfg(all(test, not(target_os = "espidf")))]
mod tests {
    use super::*;

    const ID: &str = "fixture";

    fn stage(store: &Store, existing: bool) -> Update {
        store.dirs().unwrap();
        if existing {
            let slot = store.slot(ID);
            fs::create_dir_all(slot.join("data")).unwrap();
            fs::write(slot.join("data/state"), "previous data").unwrap();
            fs::write(slot.join("version"), "previous version").unwrap();
        }
        let work = store.work().join("incoming");
        fs::create_dir_all(work.join("slot")).unwrap();
        fs::write(work.join("slot/version"), "candidate version").unwrap();
        let update = store.update(ID, &work).unwrap();
        update.prepare().unwrap();
        update
    }

    #[test]
    fn restart_recovers_each_unconfirmed_phase_for_new_and_existing_plugins() {
        for existing in [true, false] {
            for phase in 0..4 {
                let dir = tempfile::tempdir().unwrap();
                let store = Store::new(dir.path());
                let update = stage(&store, existing);
                if phase == 1 {
                    update.begin().unwrap();
                }
                if phase >= 2 {
                    update.publish().unwrap();
                    fs::write(update.slot.join("data/state"), "candidate data").unwrap();
                }
                if phase == 3 {
                    update.commit().unwrap();
                }
                store.recover().unwrap();
                store.recover().unwrap();
                assert!(!store.unresolved(ID).unwrap());
                assert!(fs::read_dir(store.work()).unwrap().next().is_none());
                let slot = store.slot(ID);
                let expected = match (existing, phase) {
                    (_, 3) => ("candidate version", "candidate data"),
                    (true, _) => ("previous version", "previous data"),
                    (false, _) => {
                        assert!(!slot.exists());
                        continue;
                    }
                };
                assert_eq!(
                    fs::read_to_string(slot.join("version")).unwrap(),
                    expected.0
                );
                assert_eq!(
                    fs::read_to_string(slot.join("data/state")).unwrap(),
                    expected.1
                );
            }
        }
    }

    #[test]
    fn failed_publication_and_commit_restore_the_original_data() {
        for publish_fails in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::new(dir.path());
            let update = stage(&store, true);
            if publish_fails {
                fs::remove_dir_all(&update.staged).unwrap();
                assert!(update.publish().is_err());
            } else {
                update.publish().unwrap();
                fs::write(update.slot.join("data/state"), "failed migration").unwrap();
                fs::create_dir(&update.retired).unwrap();
                fs::write(update.retired.join("conflict"), "blocked").unwrap();
                assert!(update.commit().is_err());
            }
            update.rollback().unwrap();
            assert!(!store.unresolved(ID).unwrap());
            assert_eq!(
                fs::read_to_string(update.slot.join("data/state")).unwrap(),
                "previous data"
            );
        }
    }

    #[test]
    fn data_snapshot_enforces_total_size_and_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("one"), "1234").unwrap();
        fs::write(source.join("two"), "5678").unwrap();
        for (name, bytes, entries) in [("size", 7, 2), ("entries", 8, 1)] {
            let destination = dir.path().join(name);
            fs::create_dir(&destination).unwrap();
            let mut budget = DataBudget { bytes, entries };
            assert!(copy_data(&source, &destination, &mut budget, DATA_DEPTH).is_err());
            assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
        }
    }
}
