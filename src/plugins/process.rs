use std::{collections::HashSet, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch},
    time::timeout,
};

use super::package::{self, API_VERSION, Label, Manifest};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_LIMIT: usize = 128 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub method: String,
    pub path: String,
}

impl Route {
    pub fn accepts(&self, method: &str, path: &str) -> bool {
        self.method == method
            && self.path.split('/').count() == path.split('/').count()
            && self
                .path
                .split('/')
                .zip(path.split('/'))
                .all(|(template, part)| {
                    template == part || template.starts_with(':') && !part.is_empty()
                })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub path: String,
    pub title: Label,
    #[serde(default)]
    pub navigation: bool,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl Tool {
    fn valid(&self) -> bool {
        let Some(properties) = self.parameters["properties"].as_object() else {
            return false;
        };
        let Some(required) = self.parameters["required"].as_array() else {
            return false;
        };
        package::name(&self.name)
            && !self.description.trim().is_empty()
            && self.description.len() <= 1000
            && self.parameters["type"] == "object"
            && self.parameters["additionalProperties"] == false
            && properties.len() <= 16
            && required
                .iter()
                .all(|key| key.as_str().is_some_and(|key| properties.contains_key(key)))
            && properties.values().all(|property| {
                matches!(
                    property["type"].as_str(),
                    Some("string" | "integer" | "number" | "boolean")
                )
            })
    }

    pub fn accepts(&self, args: &Value) -> bool {
        let Some(args) = args.as_object() else {
            return false;
        };
        self.parameters["required"]
            .as_array()
            .unwrap()
            .iter()
            .all(|key| args.contains_key(key.as_str().unwrap()))
            && args.iter().all(|(key, value)| {
                match self.parameters["properties"][key]["type"].as_str() {
                    Some("string") => value.as_str().is_some_and(|value| value.len() <= 1000),
                    Some("integer") => value.is_i64() || value.is_u64(),
                    Some("number") => value.is_number(),
                    Some("boolean") => value.is_boolean(),
                    _ => false,
                }
            })
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registration {
    api_version: u32,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub pages: Vec<Page>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub voice_context_tool: Option<String>,
}

impl Registration {
    fn validate(&self, manifest: &Manifest) -> Result<()> {
        ensure!(
            self.api_version == API_VERSION,
            "Plugin RPC version mismatch"
        );
        ensure!(
            self.pages.len() <= 16 && self.routes.len() <= 32 && self.tools.len() <= 16,
            "Too many plugin registrations"
        );
        ensure!(
            self.pages.is_empty() || manifest.web.is_some(),
            "Pages require a web entry module"
        );
        let mut paths = HashSet::new();
        let mut ids = HashSet::new();
        for page in &self.pages {
            ensure!(
                package::name(&page.id)
                    && page.title.valid()
                    && (page.path.is_empty() || page.path.split('/').all(package::name))
                    && page.path.len() <= 128,
                "Invalid plugin page registration"
            );
            ensure!(
                paths.insert(&page.path) && ids.insert(&page.id),
                "Duplicate plugin page"
            );
        }
        let mut routes = HashSet::new();
        for route in &self.routes {
            let path = route
                .path
                .strip_prefix('/')
                .context("Plugin API routes are absolute within the plugin")?;
            ensure!(
                matches!(
                    route.method.as_str(),
                    "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
                ) && path.len() <= 128
                    && path
                        .split('/')
                        .all(|part| package::name(part.strip_prefix(':').unwrap_or(part))),
                "Invalid plugin API registration"
            );
            let shape = path
                .split('/')
                .map(|part| if part.starts_with(':') { ":" } else { part })
                .collect::<Vec<_>>()
                .join("/");
            ensure!(
                routes.insert((&route.method, shape)),
                "Duplicate plugin API route"
            );
        }
        let mut tools = HashSet::new();
        for tool in &self.tools {
            ensure!(
                tool.valid() && tools.insert(&tool.name),
                "Invalid or repeated assistant tool"
            );
        }
        if let Some(name) = &self.voice_context_tool {
            ensure!(
                self.tools
                    .iter()
                    .any(|tool| tool.name == *name && tool.accepts(&json!({}))),
                "Voice context must reference a tool that accepts empty arguments"
            );
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Process {
    requests: mpsc::Sender<Call>,
    stop: watch::Sender<bool>,
}

struct Call {
    method: &'static str,
    params: Value,
    response: oneshot::Sender<Result<Value>>,
}

impl Process {
    #[cfg(test)]
    pub fn mock(handler: impl Fn(Value) -> Value + Send + 'static) -> Self {
        let (requests, mut incoming) = mpsc::channel::<Call>(1);
        let (stop, mut stopping) = watch::channel(false);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopping.changed() => break,
                    call = incoming.recv() => {
                        let Some(call) = call else { break; };
                        assert_eq!(call.method, "tool");
                        let _ = call.response.send(Ok(handler(call.params)));
                    }
                }
            }
        });
        Self { requests, stop }
    }

    pub async fn start(
        root: &Path,
        data: &Path,
        manifest: &Manifest,
        sound: mpsc::Sender<()>,
    ) -> Result<(Self, Registration)> {
        let root = root.canonicalize()?;
        let data = data.canonicalize()?;
        let mut command = Command::new(root.join(&manifest.executable));
        command
            .current_dir(&data)
            .env_clear()
            .env("GOODWIN_PLUGIN_DATA", &data);
        #[cfg(windows)]
        if let Some(value) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("Start plugin process")?;
        let mut input = child.stdin.take().context("Open plugin stdin")?;
        let mut output = Frames {
            reader: BufReader::new(child.stdout.take().context("Open plugin stdout")?),
            partial: Vec::new(),
        };
        let ready = async {
            write(&mut input, &json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "apiVersion": API_VERSION } })).await?;
            let ready: Registration = serde_json::from_value(response(output.read().await?, 0)??)?;
            ready.validate(manifest)?;
            Ok::<_, anyhow::Error>(ready)
        };
        let registration = match timeout(START_TIMEOUT, ready)
            .await
            .context("Plugin readiness timed out")
            .and_then(|ready| ready)
        {
            Ok(registration) => registration,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let (requests, mut incoming) = mpsc::channel(8);
        let (stop, mut stopping) = watch::channel(false);
        let id = manifest.id.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = stopping.changed() => {}
                result = run(&mut input, &mut output, &mut incoming, sound) => {
                    if let Err(error) = result { eprintln!("Plugin {id} stopped: {error:#}"); }
                }
            }
            drop(input);
            if timeout(Duration::from_millis(500), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        });
        Ok((Self { requests, stop }, registration))
    }

    pub fn running(&self) -> bool {
        !self.requests.is_closed() && !*self.stop.borrow()
    }

    pub async fn stop(&self) {
        let _ = self.stop.send(true);
        self.requests.closed().await;
    }

    pub async fn call(&self, method: &'static str, params: Value) -> Result<Value> {
        let (response, result) = oneshot::channel();
        ensure!(self.running(), "Plugin process is unavailable");
        self.requests
            .try_send(Call {
                method,
                params,
                response,
            })
            .map_err(|_| anyhow!("Plugin process is busy or unavailable"))?;
        timeout(CALL_TIMEOUT + CALL_TIMEOUT, result)
            .await
            .context("Plugin request timed out")?
            .context("Plugin process stopped")?
    }
}

async fn run(
    input: &mut ChildStdin,
    output: &mut Frames,
    incoming: &mut mpsc::Receiver<Call>,
    sound: mpsc::Sender<()>,
) -> Result<()> {
    let mut id = 0u64;
    loop {
        tokio::select! {
            frame = output.read() => notify(frame?, &sound)?,
            call = incoming.recv() => {
                let Some(call) = call else { return Ok(()); };
                if call.response.is_closed() { continue; }
                id += 1;
                let result = timeout(CALL_TIMEOUT, async {
                    write(input, &json!({ "jsonrpc": "2.0", "id": id, "method": call.method, "params": call.params })).await?;
                    loop {
                        let message = output.read().await?;
                        if message.get("id").is_some() { break response(message, id); }
                        notify(message, &sound)?;
                    }
                }).await.context("Plugin RPC timed out")??;
                let _ = call.response.send(result);
            }
        }
    }
}

fn response(mut message: Value, id: u64) -> Result<Result<Value>> {
    ensure!(
        message["jsonrpc"] == "2.0" && message["id"] == id,
        "Unexpected plugin RPC response"
    );
    if let Some(result) = message
        .as_object_mut()
        .and_then(|value| value.remove("result"))
    {
        return Ok(Ok(result));
    }
    ensure!(
        message["error"]["code"].is_i64(),
        "Invalid plugin RPC error"
    );
    Ok(Err(anyhow!("Plugin rejected the request")))
}

fn notify(message: Value, sound: &mpsc::Sender<()>) -> Result<()> {
    ensure!(
        message["jsonrpc"] == "2.0"
            && message.get("id").is_none()
            && message["method"] == "play_timer_signal",
        "Unexpected plugin notification"
    );
    if sound.try_send(()).is_err() {
        eprintln!("Timer signal queue is full or unavailable");
    }
    Ok(())
}

async fn write(input: &mut ChildStdin, message: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    ensure!(bytes.len() < FRAME_LIMIT, "Plugin request is too large");
    bytes.push(b'\n');
    input.write_all(&bytes).await?;
    Ok(())
}

struct Frames {
    reader: BufReader<ChildStdout>,
    partial: Vec<u8>,
}

impl Frames {
    async fn read(&mut self) -> Result<Value> {
        loop {
            let buffer = self.reader.fill_buf().await?;
            if buffer.is_empty() {
                bail!("Plugin closed stdout");
            }
            let newline = buffer.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(buffer.len(), |index| index + 1);
            ensure!(
                self.partial.len() + count <= FRAME_LIMIT,
                "Plugin response is too large"
            );
            self.partial.extend_from_slice(&buffer[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                let message = serde_json::from_slice(&self.partial)?;
                self.partial.clear();
                return Ok(message);
            }
        }
    }
}
