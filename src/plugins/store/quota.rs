use std::{fs, io::ErrorKind, path::Path};

use goodwin_error::{Context, Result, ensure};

use super::{DATA_BYTES, DATA_DEPTH, DATA_ENTRIES};

/// A guest is serialized by its runtime. Recount after a rename or an
/// incomplete write and confirm the on-disk size near the storage cap.
#[derive(Default)]
pub(crate) struct Quota(Option<u64>);

impl Quota {
    pub(crate) fn projected(&self, root: &Path, old: u64, new: u64) -> Result<u64> {
        const RECOUNT_MARGIN: u64 = 256 * 1024;
        let projected = self
            .0
            .and_then(|used| used.checked_sub(old)?.checked_add(new));
        let next = match projected {
            Some(next) if next < DATA_BYTES - RECOUNT_MARGIN => next,
            _ => {
                let mut entries = DATA_ENTRIES;
                measure(root, DATA_DEPTH, &mut entries)?
                    .checked_sub(old)
                    .and_then(|used| used.checked_add(new))
                    .context("Plugin data usage changed during the write")?
            }
        };
        ensure!(
            new <= old || next <= DATA_BYTES,
            "Plugin data storage limit reached"
        );
        Ok(next)
    }

    pub(crate) fn saved(&mut self, next: u64) {
        self.0 = Some(next);
    }

    pub(crate) fn removed(&mut self, bytes: u64) {
        self.0 = self.0.and_then(|size| size.checked_sub(bytes));
    }

    pub(crate) fn invalidate(&mut self) {
        self.0 = None;
    }
}

pub(crate) fn file_size(path: &Path) -> Result<u64> {
    #[cfg(target_os = "espidf")]
    let metadata = fs::metadata(path);
    #[cfg(not(target_os = "espidf"))]
    let metadata = fs::symlink_metadata(path);
    match metadata {
        Ok(entry) => {
            ensure!(entry.is_file(), "Plugin data must be a regular file");
            Ok(entry.len())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn measure(path: &Path, depth: usize, entries: &mut usize) -> Result<u64> {
    ensure!(depth > 0, "Plugin data exceeds maximum directory depth");
    let mut bytes = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        *entries = entries
            .checked_sub(1)
            .context("Too many plugin data entries")?;
        let kind = entry.file_type()?;
        let size = if kind.is_dir() {
            measure(&entry.path(), depth - 1, entries)?
        } else {
            ensure!(kind.is_file(), "Plugin data must be a regular file");
            entry.metadata()?.len()
        };
        bytes = bytes
            .checked_add(size)
            .context("Plugin data is too large")?;
    }
    Ok(bytes)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn replaces_and_removes_data_at_the_total_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing");
        let file = fs::File::create(&path).unwrap();
        file.set_len(DATA_BYTES).unwrap();
        let mut quota = Quota::default();
        assert!(quota.projected(dir.path(), 0, 1).is_err());
        let next = quota.projected(dir.path(), DATA_BYTES, 3).unwrap();
        file.set_len(3).unwrap();
        quota.saved(next);
        fs::remove_file(&path).unwrap();
        quota.removed(3);
        assert_eq!(quota.projected(dir.path(), 0, 2).unwrap(), 2);
    }
}
