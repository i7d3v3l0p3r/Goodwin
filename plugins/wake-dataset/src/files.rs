#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use std::{
        fs::{self, File},
        io::{self, Read, Seek, SeekFrom, Write},
        path::Path,
    };

    use goodwin_error::{Context, Result};

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

    pub fn list(path: &Path) -> Result<Vec<String>> {
        let mut data = vec![0; 128 * 1024];
        let count = guest::call(
            Request::List {
                path: path.to_string_lossy().into_owned(),
            },
            &[],
            &mut data,
        )?;
        Ok(goodwin_plugin::json_codec::from_slice(&data[..count])?)
    }

    pub fn read(path: &Path, offset: u64, length: usize, optional: bool) -> Result<Vec<u8>> {
        let mut data = vec![0; length];
        let count = guest::call(
            Request::Read {
                path: path.to_string_lossy().into_owned(),
                offset,
                length,
                optional,
            },
            &[],
            &mut data,
        )?;
        data.truncate(count);
        Ok(data)
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
