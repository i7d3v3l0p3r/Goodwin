use std::{env, fs, net::SocketAddr, path::PathBuf};

use goodwin_error::{Context, Result, bail, ensure};
use goodwin_plugin::provider::{Capability, MAX_RECORDING_BYTES};

use crate::{
    app::App,
    config::{Access, Config, DEFAULT_MICROPHONE_GAIN, Settings, valid_microphone_gain},
    journal, model, panel, password,
    plugins::Plugins,
    station::Player,
    wake, wake_catalog, wav,
};

const USAGE: &str = "Usage:\n  goodwin setup <data-dir> [open|password]\n  goodwin set-password <data-dir>\n  goodwin serve <data-dir> [addr:port]\n  goodwin device-capabilities\n  goodwin wake-runtimes\n  goodwin wake-check <model>\n  goodwin wake-check --installed-inference <data-dir>\n  goodwin wav <input.wav> <output.wav> [data-dir]";

#[cfg(all(target_os = "linux", feature = "wake-bench"))]
mod wake_bench;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(
    feature = "hotpath-alloc",
    hotpath::main(report = "functions-alloc,threads")
)]
pub(crate) async fn run() -> Result<()> {
    #[cfg(all(feature = "wasm-plugins", target_os = "linux"))]
    std::hint::black_box("goodwin.plugins.wamr.v1");
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        #[cfg(all(target_os = "linux", feature = "wake-bench"))]
        Some("wake-bench") => wake_bench::run(&args[1..]),
        Some("--version" | "-V") if args.len() == 1 => {
            println!("{version}", version = env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        Some("core-update-worker") if args.len() == 2 => {
            let version = args[1].to_str().context("Invalid update version")?;
            tokio::task::block_in_place(|| crate::core_update::run_worker(version))
        }
        Some("setup") if (2..=3).contains(&args.len()) => {
            let dir = PathBuf::from(&args[1]);
            let mode = args.get(2).map(|value| value.to_string_lossy());
            let mode = mode.as_deref().unwrap_or("password");
            setup(dir, mode.trim())
        }
        Some("set-password") if args.len() == 2 => {
            let dir = PathBuf::from(&args[1]);
            let mut config = Config::load(&dir)?;
            config.access = prompt_password()?;
            config.save(&dir, false)?;
            println!("Panel password saved. Restart Goodwin to apply it.");
            Ok(())
        }
        Some("serve") if (2..=3).contains(&args.len()) => {
            let dir = PathBuf::from(&args[1]);
            journal::init(&dir);
            let address = args
                .get(2)
                .map(|arg| arg.to_string_lossy())
                .unwrap_or("127.0.0.1:8787".into());
            let result = match address.parse().context("Invalid panel address") {
                Ok(address) => serve(dir, address).await,
                Err(error) => Err(error),
            };
            if let Err(error) = &result {
                log::error!("Goodwin service failed: {error}");
            }
            result
        }
        Some("wav") if (3..=4).contains(&args.len()) => {
            let dir = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(default_data_dir);
            answer_wav(dir, PathBuf::from(&args[1]), PathBuf::from(&args[2])).await
        }
        Some("device-capabilities") if args.len() == 1 => {
            println!("{}", goodwin_json::to_string(&model::capabilities())?);
            Ok(())
        }
        Some("wake-runtimes") if args.len() == 1 => {
            println!("{}", goodwin_json::to_string(wake::runtimes())?);
            Ok(())
        }
        Some("wake-check") if args.len() == 3 && args[1] == "--installed" => {
            wake::WakeModels::check_installed(std::path::Path::new(&args[2]))
        }
        Some("wake-check") if args.len() == 3 && args[1] == "--installed-inference" => {
            wake::WakeModels::check_installed_inference(std::path::Path::new(&args[2]))
        }
        Some("wake-check") if (2..=3).contains(&args.len()) => {
            let path = std::path::Path::new(&args[1]);
            let mut model = if let Some(manifest) = args.get(2) {
                let selection: wake_catalog::Selection =
                    goodwin_json::from_reader(fs::File::open(manifest)?)?;
                selection.validate()?;
                wake::WakeModel::load_selected(path, &selection)?
            } else {
                wake::WakeModel::load(path)?
            };
            let score = model.predict(&[0; wake::FRAME_SAMPLES])?;
            println!(
                "{} wake model loaded. Silence score {score:.6}",
                model.kind()
            );
            Ok(())
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

fn setup(dir: PathBuf, mode: &str) -> Result<()> {
    ensure!(
        !Config::path(&dir).exists(),
        "This data directory already has a configuration"
    );
    let access = match mode {
        "open" => Access::Open,
        "password" => Access::PendingPassword,
        _ => bail!("Choose panel access: open or password"),
    };
    Config {
        access,
        settings: Settings::default(),
        system_prompt: String::new(),
    }
    .save(&dir, true)?;
    if mode == "password" {
        println!("Open the web panel after starting Goodwin to choose how to sign in.");
    } else {
        println!("Open panel access configured.");
    }
    Ok(())
}

fn prompt_password() -> Result<Access> {
    let password = rpassword::prompt_password("Panel password: ")?;
    let confirm = rpassword::prompt_password("Confirm password: ")?;
    ensure!(password == confirm, "Passwords do not match");
    Ok(Access::Password {
        hash: password::hash(&password)?,
    })
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
async fn serve(dir: PathBuf, address: SocketAddr) -> Result<()> {
    log::info!("Starting Goodwin {}", env!("CARGO_PKG_VERSION"));
    let config = Config::load(&dir)?;
    let microphone = model::microphone()?;
    let gain = env::var("GOODWIN_MIC_GAIN")
        .ok()
        .map(|value| value.parse::<f32>())
        .transpose()
        .context("Invalid microphone gain")?
        .unwrap_or(DEFAULT_MICROPHONE_GAIN);
    ensure!(
        valid_microphone_gain(gain),
        "Microphone gain must be between 0.1 and 64"
    );
    let app = App::new(dir, config, Player::new(), gain);
    let listener = tokio::net::TcpListener::bind(address).await?;
    app.plugins.restore().await;
    let address = listener.local_addr()?;
    println!("Goodwin panel listening on http://{address}");
    log::info!("Panel listening on http://{address}");
    let capture = microphone.start(app.clone());
    let shutdown_app = app.clone();
    let result = axum::serve(listener, panel::router(app.clone()))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_app.stop_panel();
            shutdown_app.indicators.stop();
            shutdown_app.plugins.request_shutdown();
        })
        .await;
    capture.abort();
    app.plugins.shutdown().await;
    if result.is_ok() {
        log::info!("Goodwin stopped");
    }
    result.context("Serve Goodwin panel")
}

async fn shutdown_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn default_data_dir() -> PathBuf {
    env::var_os("GOODWIN_DATA_DIR").map_or_else(|| PathBuf::from("goodwin-data"), PathBuf::from)
}

async fn answer_wav(dir: PathBuf, input: PathBuf, output: PathBuf) -> Result<()> {
    ensure!(
        fs::metadata(&input)?.len() <= MAX_RECORDING_BYTES as u64,
        "Input WAV is too large"
    );
    let recording = fs::read(input)?;
    wav::check_recording(&recording)?;
    let config = Config::load(&dir)?;
    let engine = &config.settings.assistant;
    let pipeline = &engine.pipeline;
    let plugins = Plugins::new(&dir, Player::new());
    plugins
        .restore_selected(&[
            &pipeline.transcription.provider,
            &pipeline.chat.provider,
            &pipeline.speech.provider,
        ])
        .await;
    let result = async {
        let resolve = |capability| {
            engine.resolve(&plugins, capability).map_err(|_| {
                goodwin_error::report!(
                    "Provider {} is unavailable",
                    pipeline.get(capability).provider
                )
            })
        };
        let transcription = resolve(Capability::Transcription)?;
        let chat = resolve(Capability::Chat)?;
        let speech = resolve(Capability::Speech)?;
        let text = transcription
            .transcribe(&recording, config.settings.languages.transcription.as_str())
            .await
            .context("Transcribe recording")?;
        ensure!(!text.is_empty(), "No speech was transcribed");
        let reply = chat
            .reply(
                &text,
                &config
                    .settings
                    .languages
                    .voice_instructions(&config.system_prompt),
                Vec::new(),
                None,
            )
            .await
            .context("Generate an answer")?;
        let wav = speech
            .synthesize_wav(&reply)
            .await
            .context("Synthesize the answer")?;
        fs::write(output, wav)?;
        Ok(())
    }
    .await;
    plugins.shutdown().await;
    result
}
pub(crate) mod audio;
