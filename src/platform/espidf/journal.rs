use std::{collections::VecDeque, sync::Mutex};

use esp_idf_svc::{
    log::{EspIdfLogFilter, EspLogger},
    sys,
};
use log::{Level, Log, Metadata, Record};

const BYTES_LIMIT: usize = 24 * 1024;
const LINE_LIMIT: usize = 200;
const MESSAGE_LIMIT: usize = 512;
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
    lines: VecDeque<String>,
    bytes: usize,
}

pub(super) fn init() -> anyhow::Result<()> {
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
        let message: String = record
            .args()
            .to_string()
            .chars()
            .take(MESSAGE_LIMIT)
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        // SAFETY: The ESP-IDF timer is initialized before the Rust application starts.
        let milliseconds = unsafe { sys::esp_timer_get_time() } / 1_000;
        let line = format!("{milliseconds}ms {} {target} {message}\n", record.level());
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
