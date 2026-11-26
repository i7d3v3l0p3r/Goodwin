use std::{fs, io::ErrorKind, path::Path};

use goodwin_error::{Context, Result, ensure};

use super::{DATA_DEPTH, DATA_ENTRIES, package};

#[cfg(target_os = "espidf")]
pub(crate) fn sidecar(path: &Path, suffix: &str) -> Result<std::path::PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Missing data filename")?;
    Ok(path.with_file_name(format!(".{name}.{suffix}")))
}

/// Recover file replacements before a plugin can list or read its data.
/// FAT cannot rename a file over an existing destination, so the writer
/// moves the old file to .bak before publishing its synced .new file.
pub(crate) fn recover(root: &Path) -> Result<()> {
    let mut directories = vec![(root.to_owned(), DATA_DEPTH)];
    let mut remaining = DATA_ENTRIES;
    while let Some((directory, depth)) = directories.pop() {
        ensure!(depth > 0, "Plugin data exceeds maximum directory depth");
        let children = match fs::read_dir(&directory) {
            Ok(children) => children,
            Err(error) if error.kind() == ErrorKind::NotFound && directory == root => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut interrupted = Vec::new();
        for entry in children {
            let entry = entry?;
            remaining = remaining
                .checked_sub(1)
                .context("Too many plugin data entries")?;
            let kind = entry.file_type()?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if kind.is_dir() && package::path(name) {
                directories.push((entry.path(), depth - 1));
            } else if kind.is_file()
                && let Some((target, suffix)) = name
                    .strip_prefix('.')
                    .and_then(|name| name.rsplit_once('.'))
                && matches!(suffix, "bak" | "new")
                && package::path(target)
            {
                interrupted.push((entry.path(), directory.join(target), suffix == "bak"));
            }
        }
        // Finish enumerating first, since FAT directory positions can change on rename.
        for (sidecar, target, backup) in interrupted {
            if backup {
                match fs::metadata(&target) {
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        fs::rename(sidecar, target)?;
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                    Ok(metadata) => {
                        ensure!(metadata.is_file(), "Plugin data must be a regular file")
                    }
                }
            }
            fs::remove_file(sidecar)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_unpublished_replacements_before_the_guest_reads_or_lists_files() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        recover(&data).unwrap();
        let nested = data.join("nested");
        fs::create_dir_all(&nested).unwrap();

        // The old target was moved away, and the new data was never published.
        fs::write(nested.join(".clip.bak"), b"previous").unwrap();
        fs::write(nested.join(".clip.new"), b"interrupted").unwrap();

        // Publication finished, but deletion of the previous data did not.
        fs::write(data.join("settings.json"), b"active").unwrap();
        fs::write(data.join(".settings.json.bak"), b"superseded").unwrap();

        // No previous file exists for an interrupted new write.
        fs::write(data.join(".first.new"), b"unpublished").unwrap();
        fs::write(data.join(".unrelated"), b"keep").unwrap();

        recover(&data).unwrap();
        recover(&data).unwrap();
        assert_eq!(fs::read(nested.join("clip")).unwrap(), b"previous");
        assert_eq!(fs::read(data.join("settings.json")).unwrap(), b"active");
        assert_eq!(fs::read(data.join(".unrelated")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&nested).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&data).unwrap().count(), 3);
    }
}
