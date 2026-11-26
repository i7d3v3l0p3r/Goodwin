use std::{io, path::Path};

pub(crate) fn sync_directory(_directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(_directory)?.sync_all()?;
    Ok(())
}

/// After rename the new file is already visible, even if its directory sync fails.
pub(crate) fn sync_after_commit(directory: &Path) {
    if let Err(error) = sync_directory(directory) {
        log::warn!(
            "Could not synchronize commit in {}: {error}",
            directory.display()
        );
    }
}
