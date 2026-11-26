#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use std::{
        fs::{self, File},
        io::{self, Read, Seek, SeekFrom, Write},
        path::Path,
    };

    use goodwin_error::{Context, Result};

    pub fn storage_limit() -> usize {
        usize::MAX
    }

    pub fn list(path: &Path) -> Result<Vec<String>> {
        fs::create_dir_all(path)?;
        let mut names = Vec::new();
        for entry in fs::read_dir(path)? {
            if let Ok(name) = entry?.file_name().into_string() {
                names.push(name);
            }
        }
        Ok(names)
    }

    pub fn read(path: &Path, offset: u64, length: usize, optional: bool) -> Result<Vec<u8>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if optional && error.kind() == io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error.into()),
        };
        file.seek(SeekFrom::Start(offset))?;
        let mut data = Vec::new();
        file.take(length as u64).read_to_end(&mut data)?;
        Ok(data)
    }

    pub fn write(path: &Path, input: &[u8], replace: bool) -> Result<()> {
        let parent = path.parent().context("Missing wake dataset directory")?;
        let mut file = tempfile::Builder::new()
            .prefix(".capture-")
            .tempfile_in(parent)?;
        file.write_all(input)?;
        file.as_file().sync_all()?;
        if replace {
            file.persist(path)?;
        } else {
            file.persist_noclobber(path)?;
        }
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub fn rename(from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to)?;
        Ok(())
    }

    pub fn remove(path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use std::path::Path;

    use goodwin_error::Result;
    use goodwin_plugin::services::{Request, guest};

    pub fn storage_limit() -> usize {
        // Older hosts do not report a limit. They still enforce it on writes.
        guest::storage_limit()
            .ok()
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(usize::MAX)
    }

    pub fn list(path: &Path) -> Result<Vec<String>> {
        let data = guest::call_vec(
            Request::List {
                path: path.to_string_lossy().into_owned(),
            },
            &[],
            128 * 1024,
        )?;
        Ok(goodwin_plugin::json_codec::from_slice(&data)?)
    }

    pub fn read(path: &Path, offset: u64, length: usize, optional: bool) -> Result<Vec<u8>> {
        guest::call_vec(
            Request::Read {
                path: path.to_string_lossy().into_owned(),
                offset,
                length,
                optional,
            },
            &[],
            length,
        )
    }

    pub fn write(path: &Path, input: &[u8], replace: bool) -> Result<()> {
        guest::call(
            Request::Write {
                path: path.to_string_lossy().into_owned(),
                replace,
            },
            input,
            &mut [],
        )?;
        Ok(())
    }

    pub fn rename(from: &Path, to: &Path) -> Result<()> {
        guest::call(
            Request::Rename {
                from: from.to_string_lossy().into_owned(),
                to: to.to_string_lossy().into_owned(),
            },
            &[],
            &mut [],
        )?;
        Ok(())
    }

    pub fn remove(path: &Path) -> Result<()> {
        guest::call(
            Request::Remove {
                path: path.to_string_lossy().into_owned(),
            },
            &[],
            &mut [],
        )?;
        Ok(())
    }
}

pub use platform::*;
