use std::{collections::VecDeque, sync::Mutex};

use esp_idf_svc::{
    log::{EspIdfLogFilter, EspLogger},
    sys,
};
use log::{Level, Log, Metadata, Record};

use crate::journal_format;

const BYTES_LIMIT: usize = 24 * 1024;
const LINE_LIMIT: usize = 200;
static LOGGER: Journal = Journal {
    serial: EspLogger::new(EspIdfLogFilter::new()),
    entries: Mutex::new(Entries {
        lines: VecDeque::new(),
        bytes: 0,
    }),
};

struct Journal {
    serial: EspLogger,
    entries: Mutex<Entries>,
}

struct Entries {
    lines: VecDeque<Box<str>>,
    bytes: usize,
}

pub(super) fn init() -> goodwin_error::Result<()> {
    log::set_logger(&LOGGER)?;
    LOGGER.serial.filter().initialize();
    Ok(())
}

pub(super) fn recent() -> String {
    let entries = LOGGER.entries.lock().expect("diagnostic journal");
    let mut result = String::with_capacity(entries.bytes);
    for line in &entries.lines {
        result.push_str(line);
    }
    result
}

impl Log for Journal {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.serial.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        self.serial.log(record);
        let target = record.target();
        if record.level() > Level::Info || !(target == "goodwin" || target.starts_with("goodwin::"))
        {
            return;
        }
        // SAFETY: The ESP-IDF timer is initialized before the Rust application starts.
        let milliseconds = unsafe { sys::esp_timer_get_time() } / 1_000;
        let line = journal_format::line(
            format_args!("{milliseconds}ms {} {target} ", record.level()),
            *record.args(),
        )
        .into_boxed_str();
        let mut entries = self.entries.lock().expect("diagnostic journal");
        while entries.lines.len() >= LINE_LIMIT || entries.bytes + line.len() > BYTES_LIMIT {
            if let Some(old) = entries.lines.pop_front() {
                entries.bytes -= old.len();
            } else {
                break;
            }
        }
        entries.bytes += line.len();
        entries.lines.push_back(line);
    }

    fn flush(&self) {
        self.serial.flush();
    }
}
