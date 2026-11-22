use std::{fs, path::PathBuf, thread, time::Duration};

use anyhow::{Result, bail};
use goodwin_plugin::{HttpRequest, HttpResponse, Plugin, data_dir, read_json, save_json};
use serde_json::{Value, json};

struct Probe {
    state: PathBuf,
    count: u32,
}

impl Plugin for Probe {
    fn registration(&self) -> Value {
        json!({
            "apiVersion": 1,
            "pages": [{ "id": "main", "path": "", "title": {"en": "Plugin check", "ru": "Проверка плагина"}, "navigation": true }],
            "routes": [{ "method": "GET", "path": "/counter" }, { "method": "POST", "path": "/counter" }]
        })
    }

    fn http(&mut self, request: HttpRequest) -> Result<HttpResponse> {
        if request.method == "POST" {
            let count = self.count.saturating_add(1);
            save_json(&self.state, &count)?;
            self.count = count;
        }
        Ok(HttpResponse::json(200, json!({"count": self.count})))
    }
}

fn main() -> Result<()> {
    let state = data_dir()?.join("count.json");
    let executable = std::env::current_exe()?;
    let mode = executable.with_file_name("probe-mode");
    if let Ok(mode) = fs::read_to_string(mode) {
        match mode.as_str() {
            "delay" => thread::sleep(Duration::from_millis(1500)),
            "fail" | "timeout" => {
                save_json(&state, &999_u32)?;
                if mode == "timeout" {
                    thread::sleep(Duration::from_secs(60));
                }
                bail!("Intentional readiness failure");
            }
            _ => bail!("Unknown probe mode"),
        }
    }
    let count = read_json(&state)?;
    goodwin_plugin::run(Probe { state, count })
}
