use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

pub trait Plugin {
    fn registration(&self) -> Value;

    fn ready(&mut self, _host: Host) {}

    fn http(&mut self, _request: HttpRequest) -> Result<HttpResponse> {
        Ok(HttpResponse::json(404, json!({"error": "not_found"})))
    }

    fn tool(&mut self, _call: ToolCall) -> Result<Value> {
        Ok(json!({"error": "unknown_tool"}))
    }
}

#[derive(Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub body: Value,
}

#[derive(Serialize)]
pub struct HttpResponse {
    status: u16,
    body: Option<Value>,
}

impl HttpResponse {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: Some(body),
        }
    }

    pub fn empty(status: u16) -> Self {
        Self { status, body: None }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    pub voice_turn: u64,
}

#[derive(Clone)]
pub struct Host(Arc<Mutex<Output>>);

type SendFrame = dyn FnMut(&Value) -> Result<()> + Send;

struct Output {
    send: Box<SendFrame>,
    pending_signals: Option<usize>,
}

impl Host {
    pub fn play_timer_signal(&self) -> Result<()> {
        let mut output = self.0.lock().expect("plugin output");
        if let Some(count) = &mut output.pending_signals {
            *count += 1;
            return Ok(());
        }
        output.send_timer_signal()
    }

    fn confirm_ready(&self, reply: &Value) -> Result<()> {
        let mut output = self.0.lock().expect("plugin output");
        (output.send)(reply)?;
        for _ in 0..output.pending_signals.take().expect("startup signals") {
            output.send_timer_signal()?;
        }
        Ok(())
    }
}

impl Output {
    fn send_timer_signal(&mut self) -> Result<()> {
        (self.send)(&json!({"jsonrpc": "2.0", "method": "play_timer_signal"}))
    }
}

#[derive(Deserialize)]
struct Request {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Value,
}

pub fn run(plugin: impl Plugin) -> Result<()> {
    run_protocol(plugin, io::stdin().lock(), send)
}

fn run_protocol(
    mut plugin: impl Plugin,
    input: impl BufRead,
    send: impl FnMut(&Value) -> Result<()> + Send + 'static,
) -> Result<()> {
    let mut input = input.lines();
    let line = input
        .next()
        .context("Expected the Goodwin initialize request")??;
    let init: Request = serde_json::from_str(&line)?;
    ensure!(
        init.jsonrpc == "2.0" && init.method == "initialize" && init.params["apiVersion"] == 1,
        "Unsupported Goodwin protocol"
    );
    let host = Host(Arc::new(Mutex::new(Output {
        send: Box::new(send),
        pending_signals: Some(0),
    })));
    plugin.ready(host.clone());
    let registration = plugin.registration();
    host.confirm_ready(&json!({"jsonrpc": "2.0", "id": init.id, "result": registration}))?;

    for line in input {
        let request: Request = serde_json::from_str(&line?)?;
        ensure!(request.jsonrpc == "2.0", "Unexpected RPC version");
        let result = match request.method.as_str() {
            "http" => serde_json::from_value(request.params)
                .map_err(Into::into)
                .and_then(|request| plugin.http(request))
                .and_then(|reply| serde_json::to_value(reply).map_err(Into::into)),
            "tool" => serde_json::from_value(request.params)
                .map_err(Into::into)
                .and_then(|call| plugin.tool(call)),
            _ => Err(anyhow::anyhow!("Unknown RPC method")),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": request.id, "result": result}),
            Err(error) => {
                eprintln!("Plugin request: {error:#}");
                json!({"jsonrpc": "2.0", "id": request.id, "error": {"code": -32603, "message": "Plugin request failed"}})
            }
        };
        (host.0.lock().expect("plugin output").send)(&response)?;
    }
    Ok(())
}

fn send(message: &Value) -> Result<()> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, message)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

pub fn data_dir() -> Result<PathBuf> {
    std::env::var_os("GOODWIN_PLUGIN_DATA")
        .map(PathBuf::from)
        .context("GOODWIN_PLUGIN_DATA is missing")
}

pub fn read_json<T: Default + DeserializeOwned>(path: &Path) -> Result<T> {
    match fs::File::open(path) {
        Ok(file) => Ok(serde_json::from_reader(file)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let directory = path
        .parent()
        .context("Data file must be inside a directory")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut file, value)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIALIZE: &[u8] =
        br#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"apiVersion":1}}"#;

    struct StartupPlugin {
        initialized: bool,
        fail: bool,
    }

    impl Plugin for StartupPlugin {
        fn registration(&self) -> Value {
            assert!(self.initialized);
            json!({"apiVersion": 1})
        }

        fn ready(&mut self, host: Host) {
            host.play_timer_signal().unwrap();
            assert!(!self.fail, "startup failed");
            self.initialized = true;
        }
    }

    #[test]
    fn startup_signal_follows_the_successful_initialize_response() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&frames);
        let plugin = StartupPlugin {
            initialized: false,
            fail: false,
        };
        run_protocol(plugin, INITIALIZE, move |frame| {
            captured.lock().unwrap().push(frame.clone());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            *frames.lock().unwrap(),
            [
                json!({"jsonrpc": "2.0", "id": 0, "result": {"apiVersion": 1}}),
                json!({"jsonrpc": "2.0", "method": "play_timer_signal"}),
            ]
        );
    }

    #[test]
    fn failed_initialization_sends_neither_readiness_nor_a_startup_signal() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&frames);
        let plugin = StartupPlugin {
            initialized: false,
            fail: true,
        };
        let failure = std::panic::catch_unwind(|| {
            run_protocol(plugin, INITIALIZE, move |frame| {
                captured.lock().unwrap().push(frame.clone());
                Ok(())
            })
        });

        assert!(failure.is_err());
        assert!(frames.lock().unwrap().is_empty());
    }
}
