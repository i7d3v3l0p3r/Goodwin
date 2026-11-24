use std::{
    borrow::Cow,
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use wasmi::{
    Caller, CompilationMode, Config, EnforcedLimits, Engine, ExternType, Linker, Memory, Module,
    Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};

use super::*;
#[cfg(test)]
mod integration;
mod services;

const MODULE_LIMIT: usize = 512 * 1024;
const MEMORY_LIMIT: usize = 512 * 1024;
const INSTANCE_LIMIT: usize = 16;
const STATE_LIMIT: usize = 64 * 1024;
const CALL_FUEL: u64 = 32_000_000;
const MAX_SIGNALS: u32 = 32;

pub(super) async fn start(
    root: &Path,
    data: &Path,
    manifest: &Manifest,
    sound: mpsc::Sender<()>,
    catalog_changes: watch::Sender<()>,
) -> Result<(Process, Registration)> {
    let state_path = manifest
        .wasm_state_file
        .as_ref()
        .map(|name| data.join(name));
    let state = if let Some(path) = state_path.clone() {
        tokio::task::spawn_blocking(move || read_state(&path)).await??
    } else {
        Value::Null
    };
    let ready = async {
        let host = (manifest.wasm_http.is_some() || manifest.wasm_storage)
            .then(|| services::Host::new(data.to_owned(), manifest))
            .transpose()?;
        let memory_limit = if manifest.wasm_http.is_some() {
            64 * 1024 * 1024
        } else if manifest.wasm_storage {
            8 * 1024 * 1024
        } else {
            MEMORY_LIMIT
        };
        let instance = timeout(
            START_TIMEOUT,
            Instance::open_with_host(root.join(&manifest.executable), host, memory_limit),
        )
        .await
        .context("WASM module loading timed out")??;
        let request = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": { "apiVersion": API_VERSION, "pageIcons": true, "state": state },
        });
        let (mut reply, _) = instance
            .invoke(
                request,
                CallLimits {
                    timeout: START_TIMEOUT,
                    ..CallLimits::STANDARD
                },
                None,
                None,
            )
            .await?
            .context("WASM plugin stopped during initialization")?;
        let effects = Effects::take(&mut reply)?;
        let registration: Registration = serde_json::from_value(response(reply, 0)??)?;
        registration.validate(manifest)?;
        ensure!(
            (manifest.wasm_http.is_some()
                || (registration.providers.is_empty()
                    && !registration.model_catalog
                    && !registration.chat_streaming
                    && !registration.speech_streaming))
                && (manifest.wasm_storage || !registration.wake_events),
            "WASM plugin registration requires host services"
        );
        Ok::<_, anyhow::Error>((instance, registration, effects))
    };
    let (instance, registration, effects) = ready.await?;
    let mut deadline = None;
    effects.apply(state_path.as_deref(), &mut deadline).await?;
    let (requests, mut incoming) = mpsc::channel(8);
    let (stop, mut stopping) = watch::channel(false);
    let id = manifest.id.clone();
    let chat_streaming = registration.chat_streaming;
    let speech_streaming = registration.speech_streaming;
    let wake_subscription = registration
        .wake_events
        .then(|| Arc::new(AtomicU64::new(registration.wake_subscription)));
    let subscription = wake_subscription.clone();
    tokio::spawn(async move {
        if let Err(error) = serve(
            &instance,
            &mut incoming,
            &mut stopping,
            &sound,
            state_path.as_deref(),
            deadline,
            subscription.as_deref(),
        )
        .await
        {
            log::warn!("WASM plugin {id} stopped: {error}");
        }
        drop(instance);
        drop(incoming);
        catalog_changes.send_replace(());
    });
    Ok((
        Process {
            requests,
            stop,
            chat_streaming,
            speech_streaming,
            wake_subscription,
            provider_connection: None,
        },
        registration,
    ))
}

async fn serve(
    instance: &Instance,
    incoming: &mut mpsc::Receiver<Call>,
    stopping: &mut watch::Receiver<bool>,
    sound: &mpsc::Sender<()>,
    state_path: Option<&Path>,
    mut deadline: Option<u64>,
    wake_subscription: Option<&AtomicU64>,
) -> Result<()> {
    let mut id = 0u64;
    loop {
        let call = tokio::select! {
            biased;
            _ = stopping.changed() => return Ok(()),
            _ = wait_for_deadline(deadline) => Call {
                method: "tick",
                params: json!({}),
                response: None,
                chunks: None,
                limits: CallLimits::STANDARD,
            },
            call = incoming.recv() => {
                let Some(call) = call else { return Ok(()); };
                call
            }
        };
        if call
            .response
            .as_ref()
            .is_some_and(oneshot::Sender::is_closed)
        {
            continue;
        }
        id += 1;
        let request =
            json!({ "jsonrpc": "2.0", "id": id, "method": call.method, "params": call.params });
        let Some((mut reply, signals)) = instance
            .invoke(request, call.limits, call.chunks, Some(stopping))
            .await?
        else {
            return Ok(());
        };
        let effects = Effects::take(&mut reply)?;
        let subscription = reply["wakeSubscription"].as_u64();
        let result = response(reply, id)?;
        if let (Some(state), Some(subscription)) = (wake_subscription, subscription) {
            state.store(subscription, Ordering::Release);
        }
        if result.is_ok() {
            if call.method == "tick" {
                deadline = None;
            }
            effects.apply(state_path, &mut deadline).await?;
            if call.method == "tick" {
                deadline = deadline.map(|wake_at| wake_at.max(now_ms().saturating_add(10)));
            }
            for _ in 0..signals {
                if sound.try_send(()).is_err() {
                    log::warn!("Timer signal queue is full or unavailable");
                    break;
                }
            }
        } else if call.method == "tick" {
            // Retry scheduled work after a recoverable guest error.
            deadline = Some(now_ms().saturating_add(1000));
        }
        if call.method == "tick" {
            if let Err(error) = result {
                log::warn!("WASM plugin scheduled call failed: {error}");
            }
        } else {
            if call.method == EVENT_METHOD
                && let Err(error) = &result
            {
                log::warn!("WASM plugin wake event failed: {error}");
            }
            complete(call.response, result);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn wait_for_deadline(deadline: Option<u64>) {
    let Some(deadline) = deadline else {
        return std::future::pending().await;
    };
    while let Some(delay) = deadline.checked_sub(now_ms()).filter(|&delay| delay != 0) {
        tokio::time::sleep(Duration::from_millis(delay.min(5000))).await;
    }
}

fn read_state(path: &Path) -> Result<Value> {
    use rustix::fs::{Mode, OFlags, open};
    let file = match open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(file) => File::from(file),
        Err(rustix::io::Errno::NOENT) => return Ok(Value::Null),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.is_file(),
        "WASM state must be a regular file"
    );
    let mut bytes = Vec::new();
    file.take((STATE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= STATE_LIMIT, "WASM plugin state is too large");
    Ok(serde_json::from_slice(&bytes)?)
}

struct Effects {
    deadline: Option<Option<u64>>,
    state: Option<Value>,
}

impl Effects {
    fn take(reply: &mut Value) -> Result<Self> {
        let reply = reply
            .as_object_mut()
            .context("Expected WASM response object")?;
        let deadline = reply
            .remove("wakeAt")
            .map(serde_json::from_value)
            .transpose()?;
        Ok(Self {
            deadline,
            state: reply.remove("state"),
        })
    }

    async fn apply(self, state_path: Option<&Path>, deadline: &mut Option<u64>) -> Result<()> {
        if let Some(state) = self.state {
            let path = state_path
                .context("WASM plugin did not declare a state file")?
                .to_owned();
            tokio::task::spawn_blocking(move || {
                ensure!(
                    serde_json::to_vec(&state)?.len() <= STATE_LIMIT,
                    "WASM plugin state is too large"
                );
                goodwin_plugin::save_json(&path, &state)
            })
            .await??;
        }
        if let Some(next) = self.deadline {
            *deadline = next;
        }
        Ok(())
    }
}

struct Instance(Arc<Mutex<Active>>);

struct Active {
    guest: Guest,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Instance {
    #[cfg(test)]
    async fn open(path: PathBuf) -> Result<Self> {
        Self::open_with_host(path, None, MEMORY_LIMIT).await
    }

    async fn open_with_host(
        path: PathBuf,
        host: Option<services::Host>,
        memory_limit: usize,
    ) -> Result<Self> {
        static INSTANCES: LazyLock<Arc<tokio::sync::Semaphore>> =
            LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(INSTANCE_LIMIT)));
        let permit = INSTANCES
            .clone()
            .try_acquire_owned()
            .context("Too many active WASM plugins")?;
        tokio::task::spawn_blocking(move || {
            let guest = Guest::open(path, host, memory_limit).context("Load WASM plugin")?;
            Ok(Self(Arc::new(Mutex::new(Active {
                guest,
                _permit: permit,
            }))))
        })
        .await?
    }

    #[cfg(test)]
    async fn call(&self, request: Value) -> Result<Reply> {
        self.invoke(request, CallLimits::STANDARD, None, None)
            .await?
            .context("WASM plugin stopped")
    }

    async fn invoke(
        &self,
        mut request: Value,
        limits: CallLimits,
        chunks: Option<StreamChunks>,
        stopping: Option<&mut watch::Receiver<bool>>,
    ) -> Result<Option<Reply>> {
        let active = self.0.clone();
        let shutdown = stopping.as_deref().cloned();
        let deadline = Instant::now() + limits.timeout;
        let mut work = tokio::task::spawn_blocking(move || {
            let connection = request
                .get_mut("params")
                .and_then(Value::as_object_mut)
                .and_then(|params| params.remove("providerConnection"));
            request["nowMs"] = now_ms().into();
            let frame = serde_json::to_vec(&request)?;
            ensure!(
                frame.len() <= limits.request,
                "WASM plugin request is too large"
            );
            let mut active = active.lock().map_err(|_| anyhow!("WASM worker failed"))?;
            if let Some(host) = active.guest.store.data_mut().host.as_mut()
                && host
                    .select_connection(connection.as_ref().and_then(Value::as_str))
                    .is_err()
            {
                return Ok((
                    json!({
                        "jsonrpc": "2.0", "id": request["id"],
                        "error": {"code": -32000, "message": "Provider connection is unavailable"},
                    }),
                    0,
                ));
            }
            active
                .guest
                .call(&frame, limits.response, deadline, chunks, shutdown)
        });
        let result = tokio::select! {
            result = timeout(limits.timeout, &mut work) => Some(result),
            () = async {
                if let Some(stopping) = stopping { let _ = stopping.changed().await; }
                else { std::future::pending::<()>().await; }
            } => None,
        };
        if let Some(Ok(reply)) = result {
            return Ok(Some(reply??));
        }
        // A running blocking job can still have a file operation in flight.
        // Finish it before the manager replaces or copies the plugin data.
        work.abort();
        let _ = work.await;
        if result.is_some() {
            bail!("WASM plugin RPC timed out");
        }
        Ok(None)
    }
}

type Reply = (Value, u32);

struct GuestState {
    limits: StoreLimits,
    signals: u32,
    host: Option<services::Host>,
    memory: Option<Memory>,
}

struct Guest {
    store: Store<GuestState>,
    memory: Memory,
    buffer: TypedFunc<(), i32>,
    input: Option<TypedFunc<i32, i32>>,
    dispatch: TypedFunc<i32, i32>,
}

impl Guest {
    fn open(path: PathBuf, host: Option<services::Host>, memory_limit: usize) -> Result<Self> {
        let mut module = Vec::new();
        File::open(path)?
            .take((MODULE_LIMIT + 1) as u64)
            .read_to_end(&mut module)?;
        ensure!(module.len() <= MODULE_LIMIT, "WASM module is too large");
        Self::load(&module, host, memory_limit)
    }

    #[cfg(test)]
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::load(bytes, None, MEMORY_LIMIT)
    }

    fn load(bytes: &[u8], host: Option<services::Host>, memory_limit: usize) -> Result<Self> {
        let mut config = Config::default();
        config
            .consume_fuel(true)
            .compilation_mode(CompilationMode::LazyTranslation)
            .allow_start_fn(false)
            .wasm_multi_memory(false)
            .set_max_stack_height(if host.is_some() {
                256 * 1024
            } else {
                32 * 1024
            })
            .enforced_limits(EnforcedLimits::strict());
        // Wasmi keeps translated functions in the engine. Release them with each guest.
        let engine = Engine::new(&config);
        let bytes = with_memory_export(bytes)?;
        let module = Module::new(&engine, bytes.as_ref())?;
        for name in ["__post_instantiate", "__wasm_call_ctors"] {
            ensure!(
                !matches!(module.get_export(name), Some(ExternType::Func(_))),
                "WASM plugins cannot export a startup hook"
            );
        }
        let mut linker = Linker::<GuestState>::new(&engine);
        linker.func_wrap(
            "goodwin",
            "play_timer_signal",
            |mut caller: Caller<'_, GuestState>| {
                let signals = &mut caller.data_mut().signals;
                *signals = (*signals + 1).min(MAX_SIGNALS);
            },
        )?;
        if host.is_some() {
            linker.func_wrap("goodwin", "host_call", host_call)?;
            linker.func_wrap("goodwin", "host_result", host_result)?;
        }
        let mut store = Store::new(
            &engine,
            GuestState {
                limits: StoreLimitsBuilder::new()
                    .memory_size(memory_limit)
                    .memories(1)
                    .table_elements(4096)
                    .tables(4)
                    .build(),
                signals: 0,
                host,
                memory: None,
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_fuel(CALL_FUEL)?;
        let instance = linker.instantiate_and_start(&mut store, &module)?;
        let memory = instance
            .exports(&store)
            .find_map(wasmi::Export::into_memory)
            .context("WASM plugin is missing linear memory")?;
        store.data_mut().memory = Some(memory);
        let input = if store.data().host.is_some() {
            Some(
                instance
                    .get_typed_func(&store, "goodwin_input")
                    .context("Expected WASM export goodwin_input(i32) -> i32")?,
            )
        } else {
            None
        };
        let buffer = instance
            .get_typed_func(&store, "goodwin_buffer")
            .context("Expected WASM export goodwin_buffer() -> i32")?;
        let dispatch = instance
            .get_typed_func(&store, "goodwin_call")
            .context("Expected WASM export goodwin_call(i32) -> i32")?;
        Ok(Self {
            store,
            memory,
            buffer,
            input,
            dispatch,
        })
    }

    fn call(
        &mut self,
        frame: &[u8],
        response_limit: usize,
        deadline: Instant,
        chunks: Option<StreamChunks>,
        shutdown: Option<watch::Receiver<bool>>,
    ) -> Result<Reply> {
        self.store.data_mut().signals = 0;
        if let Some(host) = self.store.data_mut().host.as_mut() {
            host.deadline = deadline;
            host.chunks = chunks;
            host.shutdown = shutdown;
        }
        let result = self.exchange(frame, response_limit);
        if let Some(host) = self.store.data_mut().host.as_mut() {
            host.finish();
        }
        result
    }

    fn exchange(&mut self, frame: &[u8], response_limit: usize) -> Result<Reply> {
        self.store.set_fuel(if self.input.is_some() {
            1_000_000_000
        } else {
            CALL_FUEL
        })?;
        let offset = if let Some(input) = &self.input {
            input
                .call(&mut self.store, frame.len() as i32)
                .context("WASM plugin trapped in goodwin_input")? as u32 as usize
        } else {
            ensure!(
                frame.len() <= FRAME_LIMIT,
                "WASM plugin request is too large"
            );
            let offset = self
                .buffer
                .call(&mut self.store, ())
                .context("WASM plugin trapped in goodwin_buffer")? as u32
                as usize;
            ensure!(
                offset
                    .checked_add(FRAME_LIMIT)
                    .is_some_and(|end| end <= self.memory.data(&self.store).len()),
                "WASM plugin returned an invalid exchange buffer"
            );
            offset
        };
        self.memory.write(&mut self.store, offset, frame)?;
        let used = self
            .dispatch
            .call(&mut self.store, frame.len() as i32)
            .context("WASM plugin trapped in goodwin_call")?;
        let used = usize::try_from(used).context("WASM plugin returned an invalid frame")?;
        let offset = if self.input.is_some() {
            self.buffer
                .call(&mut self.store, ())
                .context("WASM plugin trapped in goodwin_buffer")? as u32 as usize
        } else {
            offset
        };
        ensure!(
            used <= response_limit && (self.input.is_some() || used <= FRAME_LIMIT),
            "WASM plugin response is too large"
        );
        let response = self
            .memory
            .data(&self.store)
            .get(offset..)
            .and_then(|tail| tail.get(..used))
            .context("WASM plugin returned an invalid frame")?;
        Ok((serde_json::from_slice(response)?, self.store.data().signals))
    }
}

fn host_call(
    mut caller: Caller<'_, GuestState>,
    meta: u32,
    meta_len: u32,
    data: u32,
    data_len: u32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return -1;
    };
    let (memory, state) = memory.data_and_store_mut(&mut caller);
    let Some(host) = state.host.as_mut() else {
        return -1;
    };
    host.output.clear();
    let result = (|| -> Result<Vec<u8>> {
        ensure!(
            meta_len as usize <= services::META_LIMIT && data_len as usize <= services::INPUT_LIMIT,
            "WASM host operation is too large"
        );
        let request = serde_json::from_slice(guest_slice(memory, meta, meta_len)?)?;
        host.execute(request, guest_slice(memory, data, data_len)?)
    })();
    match result {
        Ok(output) => {
            host.output = output;
            host.output.len() as i32
        }
        Err(_) => -1,
    }
}

fn host_result(mut caller: Caller<'_, GuestState>, offset: u32, length: u32) -> i32 {
    let Some(memory) = caller.data().memory else {
        return -1;
    };
    let Some(host) = caller.data_mut().host.as_mut() else {
        return -1;
    };
    let output = std::mem::take(&mut host.output);
    if output.len() != length as usize {
        return -1;
    }
    memory
        .write(&mut caller, offset as usize, &output)
        .map_or(-1, |()| 0)
}

fn guest_slice(memory: &[u8], offset: u32, length: u32) -> Result<&[u8]> {
    if length == 0 {
        return Ok(&[]);
    }
    memory
        .get(offset as usize..)
        .and_then(|tail| tail.get(..length as usize))
        .context("WASM host buffer is out of bounds")
}

fn with_memory_export(module: &[u8]) -> Result<Cow<'_, [u8]>> {
    let mut sections = module
        .strip_prefix(b"\0asm\x01\0\0\0")
        .context("Expected WebAssembly 1 bytecode")?;
    while let Some((&section, rest)) = sections.split_first() {
        let start = module.len() - sections.len();
        sections = rest;
        let mut contents = field(&mut sections)?;
        if section != 7 {
            continue;
        }
        let count = integer(&mut contents)?;
        let exports = contents;
        let mut has_memory = false;
        for _ in 0..count {
            let name = field(&mut contents)?;
            ensure!(!name.contains(&0), "WASM export name contains a null byte");
            let (&kind, rest) = contents.split_first().context("Missing WASM export kind")?;
            contents = rest;
            integer(&mut contents)?;
            has_memory |= kind == 2;
        }
        ensure!(contents.is_empty(), "Trailing bytes in WASM export section");
        if has_memory {
            break;
        }
        // Older packages could keep their single linear memory private. Wasmi accesses it by export.
        let memory_export = b"\0goodwin_memory";
        let mut contents = Vec::new();
        write_integer(&mut contents, count + 1);
        contents.extend_from_slice(exports);
        write_integer(&mut contents, memory_export.len() as u32);
        contents.extend_from_slice(memory_export);
        contents.extend([2, 0]);
        let mut result = Vec::with_capacity(module.len() + 24);
        result.extend_from_slice(&module[..start]);
        result.push(section);
        write_integer(&mut result, contents.len() as u32);
        result.extend_from_slice(&contents);
        result.extend_from_slice(sections);
        return Ok(Cow::Owned(result));
    }
    Ok(Cow::Borrowed(module))
}

fn write_integer(bytes: &mut Vec<u8>, mut value: u32) {
    while value >= 128 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn field<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
    let length = integer(bytes)? as usize;
    let (value, rest) = bytes
        .split_at_checked(length)
        .context("Truncated WASM field")?;
    *bytes = rest;
    Ok(value)
}

fn integer(bytes: &mut &[u8]) -> Result<u32> {
    let mut result = 0;
    for offset in (0..32).step_by(7) {
        let (&byte, rest) = bytes.split_first().context("Truncated WASM integer")?;
        *bytes = rest;
        ensure!(offset != 28 || byte < 16, "WASM integer overflow");
        result |= u32::from(byte & 127) << offset;
        if byte < 128 {
            return Ok(result);
        }
    }
    bail!("WASM integer overflow")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const LIMITS: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/target/wasm-plugins/limits.wasm"
    ));
    const TIMERS: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/target/wasm32-unknown-unknown/release/goodwin-timers.wasm"
    ));

    #[tokio::test]
    async fn wasm_timers_load_native_state_and_persist_scheduled_completions() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("bin")).unwrap();
        fs::create_dir(dir.path().join("ui")).unwrap();
        fs::write(dir.path().join("ui/index.mjs"), "export default null;").unwrap();
        fs::write(dir.path().join("bin/timers.wasm"), TIMERS).unwrap();
        fs::write(
            dir.path().join("manifest.json"),
            json!({
                "id": "timers", "version": "0.1.6", "apiVersion": 1,
                "title": {"en": "Timers", "ru": "Таймеры"},
                "target": "wasm32", "executable": "bin/timers.wasm",
                "web": {"uiKitVersion": 1, "entry": "ui/index.mjs"},
                "wasmStateFile": "timers.json",
            })
            .to_string(),
        )
        .unwrap();
        let state_file = dir.path().join("timers.json");
        goodwin_plugin::save_json(&state_file, &json!({
            "next_id": 41,
            "timers": [{"id":41,"name":"Чай","seconds":60,"expiresAt":now_ms()+60000,"completed":false}],
        })).unwrap();
        let manifest = Manifest::read(dir.path()).unwrap();
        let (sound, mut audio) = mpsc::channel(1);
        let (changes, _) = watch::channel(());
        let (plugin, registration) = Process::start(
            dir.path(),
            dir.path(),
            &manifest,
            sound.clone(),
            changes.clone(),
        )
        .await
        .unwrap();
        assert!(registration.routes[0].accepts("GET", "/timers"));
        assert_eq!(registration.tools[0].name, "create");
        let result = plugin
            .call(
                "tool",
                json!({"name":"create","arguments":{"seconds":1},"voiceTurn":1}),
            )
            .await
            .unwrap();
        assert_eq!(result["created"]["seconds"], 1);
        assert_eq!(
            timeout(Duration::from_secs(3), audio.recv()).await.unwrap(),
            Some(())
        );
        plugin.stop().await;
        let saved: Value = goodwin_plugin::read_json(&state_file).unwrap();
        assert_eq!(saved["next_id"], 42);
        assert_eq!(saved["timers"][1]["completed"], true);
        let (restarted, _) = Process::start(dir.path(), dir.path(), &manifest, sound, changes)
            .await
            .unwrap();
        let result = restarted
            .call("http", json!({"method":"GET","path":"/timers","body":null}))
            .await
            .unwrap();
        assert_eq!(result["body"]["timers"][0]["name"], "Чай");
        assert_eq!(result["body"]["timers"][1]["completed"], true);
        restarted.stop().await;
    }

    #[tokio::test]
    async fn wasm_timers_handle_full_history_and_maximum_length_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.wasm");
        fs::write(&path, TIMERS).unwrap();
        let instance = Instance::open(path).await.unwrap();
        let name = "🫖".repeat(64);
        let future = now_ms() + 60_000;
        let timers: Vec<_> = (1..=95)
            .map(|id| {
                json!({
                    "id":id, "name":name, "seconds":60, "expiresAt":future, "completed":id <= 64
                })
            })
            .collect();
        let (reply, _) = instance
            .call(json!({
                "id":0, "method":"initialize", "params":{"state":{"next_id":95,"timers":timers}}
            }))
            .await
            .unwrap();
        response(reply, 0).unwrap().unwrap();
        let (reply, _) = instance.call(json!({
            "id":1, "method":"tool", "params":{"name":"create","voiceTurn":1,"arguments":{"seconds":60,"name":name}}
        })).await.unwrap();
        assert_eq!(reply["state"]["timers"].as_array().unwrap().len(), 96);
        let (reply, _) = instance
            .call(json!({
                "id":2, "method":"http", "params":{"method":"GET","path":"/timers","body":null}
            }))
            .await
            .unwrap();
        assert_eq!(
            response(reply, 2).unwrap().unwrap()["body"]["timers"]
                .as_array()
                .unwrap()
                .len(),
            96
        );
    }

    #[tokio::test]
    async fn wasm_memory_limit_caps_growth_below_the_guests_declared_maximum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grow.wasm");
        fs::write(&path, LIMITS).unwrap();
        let guest = Instance::open(path).await.unwrap();
        let (reply, _) = guest.call(json!({"id":1,"method":"grow"})).await.unwrap();
        assert_eq!(response(reply, 1).unwrap().unwrap()["status"], 200);
    }

    #[tokio::test]
    async fn wasm_instruction_limit_traps_a_guest_loop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loop.wasm");
        fs::write(&path, LIMITS).unwrap();
        let guest = Instance::open(path).await.unwrap();
        let error = timeout(
            Duration::from_secs(2),
            guest.call(json!({"id":1,"method":"loop"})),
        )
        .await
        .expect("guest loop must finish through the instruction limit")
        .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<wasmi::Error>()
                .and_then(wasmi::Error::as_trap_code),
            Some(wasmi::TrapCode::OutOfFuel),
            "{error:#}"
        );
    }

    #[test]
    fn wasm_startup_hooks_are_rejected_before_instantiation() {
        let module_with_function =
            b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0\x05\x03\x01\0\x02";
        let function_body = b"\x0a\x04\x01\x02\0\x0b";
        let mut module = module_with_function.to_vec();
        module.extend(b"\x08\x01\0");
        module.extend(function_body);
        let Err(error) = Guest::from_bytes(&module) else {
            panic!("WASM start section was accepted");
        };
        assert!(error.to_string().contains("start"), "{error:#}");
        for name in [
            b"__post_instantiate".as_slice(),
            b"__wasm_call_ctors",
            b"__post_instantiate\0alias",
        ] {
            let mut module = module_with_function.to_vec();
            module.extend([7, (name.len() + 4) as u8, 1, name.len() as u8]);
            module.extend(name);
            module.extend([0, 0]);
            module.extend(function_body);
            let Err(error) = Guest::from_bytes(&module) else {
                panic!("WASM startup hook was accepted");
            };
            let error = error.to_string();
            assert!(
                error.contains("startup hook") || error.contains("null byte"),
                "{error}"
            );
        }
    }

    #[test]
    fn wasm_plugins_can_keep_linear_memory_private() {
        let module = b"\0asm\x01\0\0\0\
                       \x01\x0a\x02\x60\0\x01\x7f\x60\x01\x7f\x01\x7f\
                       \x03\x03\x02\0\x01\x05\x03\x01\0\x02\
                       \x07\x21\x02\x0egoodwin_buffer\0\0\x0cgoodwin_call\0\x01\
                       \x0a\x0b\x02\x04\0\x41\0\x0b\x04\0\x41\0\x0b";
        let guest = Guest::from_bytes(module).unwrap();
        assert_eq!(guest.memory.data(&guest.store).len(), FRAME_LIMIT);
    }
}
