use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use log::{Level, LevelFilter, Log, Metadata, Record};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::journal_format;

pub(crate) const TAIL_BYTES: usize = 64 * 1024;
const TAIL_LINES: usize = 200;
const FILE_LIMIT: u64 = 256 * 1024;
const FILE_NAME: &str = "goodwin.log";
static FILE_ACCESS: Mutex<()> = Mutex::new(());

struct Journal {
    path: PathBuf,
}

pub(crate) fn init(dir: &Path) {
    log::set_boxed_logger(Box::new(Journal { path: path(dir) }))
        .expect("one Goodwin journal per process");
    log::set_max_level(LevelFilter::Info);
}

fn path(dir: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    if dir == Path::new("/data/opt/etc/goodwin") {
        return Path::new("/tmp").join(FILE_NAME);
    }
    dir.join(FILE_NAME)
}

impl Log for Journal {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info
            && (metadata.target() == "goodwin" || metadata.target().starts_with("goodwin::"))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();
        let line = journal_format::line(
            format_args!("{timestamp} {} {} ", record.level(), record.target()),
            *record.args(),
        );
        let _ = io::stderr().write_all(line.as_bytes());
        if let Err(error) = append(&self.path, &line) {
            let _ = writeln!(io::stderr(), "Could not write the Goodwin journal: {error}");
        }
    }

    fn flush(&self) {}
}

fn append(path: &Path, line: &str) -> io::Result<()> {
    let _guard = FILE_ACCESS.lock().expect("journal file lock");
    let mut file = open(path, true)?;
    if file.metadata()?.len() + line.len() as u64 > FILE_LIMIT {
        drop(file);
        fs::rename(path, path.with_extension("log.1"))?;
        file = open(path, true)?;
    }
    file.write_all(line.as_bytes())
}

fn open(path: &Path, append: bool) -> io::Result<File> {
    #[cfg(target_os = "linux")]
    let file = {
        use rustix::fs::{Mode, OFlags};

        let access = if append {
            OFlags::WRONLY | OFlags::CREATE | OFlags::APPEND
        } else {
            OFlags::RDONLY
        };
        let options = access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        File::from(rustix::fs::open(path, options, Mode::RUSR | Mode::WUSR)?)
    };
    #[cfg(not(target_os = "linux"))]
    let file = fs::OpenOptions::new()
        .read(!append)
        .append(append)
        .create(append)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("The journal must be a regular file"));
    }
    Ok(file)
}

pub(crate) fn recent(dir: &Path) -> Option<String> {
    let path = path(dir);
    let _guard = FILE_ACCESS.lock().expect("journal file lock");
    let current = read_tail(&path);
    if current
        .as_ref()
        .is_some_and(|text| text.lines().count() >= TAIL_LINES)
    {
        return current;
    }
    let previous = read_tail(&path.with_extension("log.1"));
    match (previous, current) {
        (Some(previous), Some(current)) => {
            let text = format!("{previous}\n{current}");
            Some(clip_tail(text.trim_matches('\n').as_bytes(), false))
        }
        (previous, current) => current.or(previous),
    }
}

fn read_tail(path: &Path) -> Option<String> {
    let mut file = open(path, false).ok()?;
    let offset = file
        .metadata()
        .ok()?
        .len()
        .saturating_sub(TAIL_BYTES as u64);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES as u64).read_to_end(&mut bytes).ok()?;
    Some(clip_tail(&bytes, offset != 0))
}

pub(crate) fn clip_tail(bytes: &[u8], truncated: bool) -> String {
    let start = bytes.len().saturating_sub(TAIL_BYTES);
    let mut bytes = &bytes[start..];
    if (truncated || start != 0)
        && let Some(newline) = bytes.iter().position(|byte| *byte == b'\n')
        && newline + 1 < bytes.len()
    {
        bytes = &bytes[newline + 1..];
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches(['\r', '\n']);
    let start = text
        .rmatch_indices('\n')
        .nth(TAIL_LINES - 1)
        .map_or(0, |(index, _)| index + 1);
    text[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_the_journal_keeps_recent_entries_and_bounds_storage() {
        let dir = tempfile::tempdir().unwrap();
        let log = path(dir.path());
        for i in 0..520 {
            append(&log, &format!("entry {i:03} {}\n", "x".repeat(1024))).unwrap();
        }
        let text = recent(dir.path()).unwrap();
        assert_eq!(text.lines().count(), 63);
        assert!(text.starts_with("entry 457 "));
        assert!(text.lines().last().unwrap().starts_with("entry 519 "));
        assert!(fs::metadata(&log).unwrap().len() <= FILE_LIMIT);
        assert!(fs::metadata(log.with_extension("log.1")).unwrap().len() <= FILE_LIMIT);
    }

    #[test]
    fn tails_keep_only_the_newest_lines_within_the_byte_limit() {
        let mut log = vec![b'x'; 2 * TAIL_BYTES];
        for line in 0..=TAIL_LINES {
            log.extend_from_slice(format!("\nline {line}").as_bytes());
        }
        let tail = clip_tail(&log, false);
        assert_eq!(tail.lines().count(), TAIL_LINES);
        assert_eq!(tail.lines().next(), Some("line 1"));
        assert!(tail.ends_with(&format!("line {TAIL_LINES}")));
        assert!(clip_tail(&vec![b'x'; 2 * TAIL_BYTES], false).len() <= TAIL_BYTES);
    }
}
