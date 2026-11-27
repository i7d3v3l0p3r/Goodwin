use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::Instant,
};

use crate::plugin_wasm::Guest;
#[cfg(test)]
use crate::plugin_wasm::Reply;

use super::*;
#[cfg(test)]
mod integration;
mod services;

const MEMORY_LIMIT: usize = 512 * 1024;
const INSTANCE_LIMIT: usize = 16;
const STATE_LIMIT: usize = 64 * 1024;

#[derive(Default)]
struct CallGuards<'a> {
    stopping: Option<&'a mut watch::Receiver<bool>>,
    response: Option<&'a mut CallResponse>,
}

pub(super) async fn start(
    root: &Path,
    data: &Path,
    manifest: &Manifest,
    sound: mpsc::Sender<Signal>,
    catalog_changes: watch::Sender<()>,
    proxy: Option<&str>,
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
            .then(|| services::Host::new(data.to_owned(), manifest, proxy))
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
        let request = plugin_rpc::Request {
            jsonrpc: "2.0",
            id: 0,
            method: "initialize",
            params: json!({ "apiVersion": API_VERSION, "state": state, "timerAlarms": true }),
            now: Some(now_ms()),
            timer_clock: Some(goodwin_plugin::timer::clock_sample()),
        };
        let (mut reply, _) = instance
            .invoke::<WireReply>(
                request,
                None,
                CallLimits {
                    timeout: START_TIMEOUT,
                    ..CallLimits::STANDARD
                },
                None,
                CallGuards::default(),
            )
            .await?
            .context("WASM plugin stopped during initialization")?;
        let effects = Effects::take(&mut reply);
        let registration: Registration = serde_json::from_str(decode_reply(reply, 0)??.get())?;
        registration.validate_wasm(manifest)?;
        Ok::<_, goodwin_error::Error>((instance, registration, effects))
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
    let timer_alarms = registration.timer_alarms;
    tokio::spawn(async move {
        if let Err(error) = serve(
            &instance,
            &mut incoming,
            &mut stopping,
            &sound,
            state_path.as_deref(),
            (deadline, timer_alarms),
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
    sound: &mpsc::Sender<Signal>,
    state_path: Option<&Path>,
    timing: (Option<u64>, bool),
    wake_subscription: Option<&AtomicU64>,
) -> Result<()> {
    let (mut deadline, timer_alarms) = timing;
    let mut timer = Delivery::new(timer_alarms);
    let mut id = 0u64;
    loop {
        let mut call = tokio::select! {
            biased;
            _ = stopping.changed() => return Ok(()),
            poll = timer.wait(), if timer_alarms => Call::timer(poll)?,
            _ = wait_for_deadline(deadline), if !timer_alarms => Call {
                method: "tick",
                params: RawValue::from_string("{}".to_owned())?,
                connection: None,
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
        let request = plugin_rpc::Request {
            jsonrpc: "2.0",
            id,
            method: call.method,
            params: call.params,
            now: Some(now_ms()),
            timer_clock: timer_alarms.then(goodwin_plugin::timer::clock_sample),
        };
        let Some((mut reply, signals)) = instance
            .invoke::<WireReply>(
                request,
                call.connection,
                call.limits,
                call.chunks,
                CallGuards {
                    stopping: Some(stopping),
                    response: call.response.as_mut(),
                },
            )
            .await?
        else {
            if *stopping.borrow() {
                return Ok(());
            }
            continue;
        };
        let effects = Effects::take(&mut reply);
        let subscription = reply.wake_subscription;
        let result = decode_reply(reply, id)?;
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
                if sound.try_send(Signal::default()).is_err() {
                    log::warn!("Timer signal queue is full or unavailable");
                    break;
                }
            }
        } else if call.method == "tick" {
            // Retry scheduled work after a recoverable guest error.
            deadline = Some(now_ms().saturating_add(1000));
        }
        if call.method == POLL_METHOD {
            timer.update(
                result.and_then(|reply| Ok(serde_json::from_str(reply.get())?)),
                sound,
            );
        } else if call.method == "tick" {
            if let Err(error) = result {
                log::warn!("WASM plugin scheduled call failed: {error}");
            }
        } else {
            timer.changed();
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
    goodwin_plugin::timer::countdown_time_ms()
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
    fn take(reply: &mut WireReply) -> Self {
        Self {
            deadline: reply.wake_at.take(),
            state: reply.state.take(),
        }
    }

    async fn apply(self, state_path: Option<&Path>, deadline: &mut Option<u64>) -> Result<()> {
        if let Some(state) = self.state {
            let path = state_path
                .context("WASM plugin did not declare a state file")?
                .to_owned();
            tokio::task::spawn_blocking(move || {
                ensure!(
                    goodwin_json::to_vec(&state)?.len() <= STATE_LIMIT,
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
    guest: Guest<services::Host>,
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
            let guest = Guest::open(&path, host, memory_limit).context("Load WASM plugin")?;
            Ok(Self(Arc::new(Mutex::new(Active {
                guest,
                _permit: permit,
            }))))
        })
        .await?
    }

    #[cfg(test)]
    async fn call(&self, request: Value) -> Result<Reply> {
        self.invoke(
            plugin_rpc::Request {
                jsonrpc: "2.0",
                id: request["id"].as_u64().unwrap(),
                method: request["method"].as_str().unwrap(),
                params: &request["params"],
                now: Some(now_ms()),
                timer_clock: Some(goodwin_plugin::timer::clock_sample()),
            },
            None,
            CallLimits::STANDARD,
            None,
            CallGuards::default(),
        )
        .await?
        .context("WASM plugin stopped")
    }

    async fn invoke<T: DeserializeOwned + Send + 'static>(
        &self,
        request: plugin_rpc::Request<'_, impl Serialize>,
        connection: Option<ProviderTarget>,
        limits: CallLimits,
        chunks: Option<StreamChunks>,
        CallGuards { stopping, response }: CallGuards<'_>,
    ) -> Result<Option<(T, u32)>> {
        let active = self.0.clone();
        let (cancel, shutdown) = watch::channel(false);
        let deadline = Instant::now() + limits.timeout;
        let id = request.id;
        let frame = serde_json::to_vec(&request)?;
        drop(request);
        let mut work = tokio::task::spawn_blocking(move || {
            ensure!(
                frame.len() <= limits.request,
                "WASM plugin request is too large"
            );
            let mut active = active.lock().map_err(|_| report!("WASM worker failed"))?;
            if let Some(host) = active.guest.host_mut()
                && host.select_config(connection).is_err()
            {
                let reply = serde_json::to_vec(&json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32000, "message": "Provider connection is unavailable"},
                }))?;
                return Ok((serde_json::from_slice(&reply)?, 0));
            }
            if let Some(host) = active.guest.host_mut() {
                host.deadline = deadline;
                host.chunks = chunks;
                host.shutdown = Some(shutdown);
            }
            let result = active.guest.call_as(&frame, limits.response);
            if let Some(host) = active.guest.host_mut() {
                host.finish();
            }
            result
        });
        let result = tokio::select! {
            biased;
            result = timeout(limits.timeout, &mut work) => Some(result),
            () = async {
                if let Some(stopping) = stopping { let _ = stopping.changed().await; }
                else { std::future::pending::<()>().await; }
            } => None,
            () = response_closed(response) => None,
        };
        if let Some(Ok(reply)) = result {
            return Ok(Some(reply??));
        }
        // A running blocking job can still have a file operation in flight.
        // Finish it before the manager replaces or copies the plugin data.
        cancel.send_replace(true);
        work.abort();
        let _ = work.await;
        if result.is_some() {
            bail!("WASM plugin RPC timed out");
        }
        Ok(None)
    }
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
            Default::default(),
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
        let signal = timeout(Duration::from_secs(3), audio.recv())
            .await
            .unwrap()
            .unwrap();
        signal.finish(false);
        let in_flight = timeout(Duration::from_secs(3), audio.recv())
            .await
            .unwrap()
            .unwrap();
        plugin.stop().await;
        drop(in_flight);
        let saved: Value = goodwin_plugin::read_json(&state_file).unwrap();
        assert_eq!(saved["next_id"], 42);
        assert_eq!(saved["timers"][1]["completed"], true);
        assert_eq!(saved["pending_alarms"], json!([42]));
        let (restarted, _) = Process::start(
            dir.path(),
            dir.path(),
            &manifest,
            sound.clone(),
            changes.clone(),
            Default::default(),
        )
        .await
        .unwrap();
        let result = restarted
            .call("http", json!({"method":"GET","path":"/timers","body":null}))
            .await
            .unwrap();
        assert_eq!(result["body"]["timers"][0]["name"], "Чай");
        assert_eq!(result["body"]["timers"][1]["completed"], true);
        let replay = timeout(Duration::from_secs(3), audio.recv())
            .await
            .unwrap()
            .unwrap();
        replay.finish(true);
        timeout(Duration::from_secs(3), async {
            loop {
                let saved: Value = goodwin_plugin::read_json(&state_file).unwrap();
                if saved["pending_alarms"].is_null() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        restarted.stop().await;
        let (restarted, _) = Process::start(
            dir.path(),
            dir.path(),
            &manifest,
            sound,
            changes,
            Default::default(),
        )
        .await
        .unwrap();
        assert!(
            timeout(Duration::from_millis(300), audio.recv())
                .await
                .is_err()
        );
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
    async fn wasm_timers_reject_id_exhaustion_without_changing_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.wasm");
        fs::write(&path, TIMERS).unwrap();
        let instance = Instance::open(path).await.unwrap();
        let (reply, _) = instance
            .call(json!({
                "id": 0, "method": "initialize",
                "params": { "state": { "next_id": u64::MAX, "timers": [] } }
            }))
            .await
            .unwrap();
        response(reply, 0).unwrap().unwrap();

        let (reply, _) = instance
            .call(json!({
                "id": 1, "method": "tool",
                "params": { "name": "create", "voiceTurn": 1, "arguments": { "seconds": 60 } }
            }))
            .await
            .unwrap();
        assert_eq!(
            reply["error"],
            json!({ "code": -32000, "message": "Timer request failed" })
        );
        assert!(reply.get("state").is_none());

        let (reply, _) = instance
            .call(json!({
                "id": 2, "method": "http",
                "params": { "method": "GET", "path": "/timers", "body": null }
            }))
            .await
            .unwrap();
        assert_eq!(
            response(reply, 2).unwrap().unwrap()["body"]["timers"],
            json!([])
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

    #[test]
    fn wasm_instruction_limit_traps_a_guest_loop() {
        let mut guest = Guest::<services::Host>::load(LIMITS, None, MEMORY_LIMIT).unwrap();
        let fuel = 10_000;
        let (reply, _) = guest
            .exchange(br#"{"id":1,"method":"grow"}"#, FRAME_LIMIT, fuel)
            .unwrap();
        assert_eq!(response(reply, 1).unwrap().unwrap()["status"], 200);
        let error = guest
            .exchange(br#"{"id":1,"method":"loop"}"#, FRAME_LIMIT, fuel)
            .unwrap_err();
        assert!(
            error.to_string().contains("instruction limit exceeded"),
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
        let Err(error) = Guest::<services::Host>::load(&module, None, MEMORY_LIMIT) else {
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
            let Err(error) = Guest::<services::Host>::load(&module, None, MEMORY_LIMIT) else {
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
        let guest = Guest::<services::Host>::load(module, None, MEMORY_LIMIT).unwrap();
        assert_eq!(guest.memory_size(), FRAME_LIMIT);
    }
}
