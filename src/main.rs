mod app;
mod config;
mod openrouter;
mod panel;
mod plugins;
mod station;
#[cfg(test)]
mod test_support;
mod wake;
mod wav;

use std::{env, fs, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    app::{App, VOICE_INSTRUCTIONS},
    config::{Access, Config, Settings, hash_password},
    openrouter::{Models, OpenRouter},
    station::{MicrophoneOptions, Player},
};

const USAGE: &str = "Usage:\n  goodwin setup <data-dir> [open|password]\n  goodwin set-password <data-dir>\n  goodwin serve <data-dir> [addr:port]\n  goodwin wake-check <model.onnx>\n  goodwin wav <input.wav> <output.wav>";

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        Some("setup") if (2..=3).contains(&args.len()) => {
            let dir = PathBuf::from(&args[1]);
            let mode = match args.get(2) {
                Some(value) => value.to_string_lossy().into_owned(),
                None => rpassword::prompt_password("Panel access (open/password): ")?,
            };
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
            let address = args
                .get(2)
                .map(|arg| arg.to_string_lossy())
                .unwrap_or("127.0.0.1:8787".into());
            serve(
                PathBuf::from(&args[1]),
                address.parse().context("Invalid panel address")?,
            )
            .await
        }
        Some("wav") if args.len() == 3 => {
            answer_wav(PathBuf::from(&args[1]), PathBuf::from(&args[2])).await
        }
        Some("wake-check") if args.len() == 2 => {
            let mut model = wake::WakeModel::load(std::path::Path::new(&args[1]))?;
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
        Some(_) if args.len() == 2 => {
            answer_wav(PathBuf::from(&args[0]), PathBuf::from(&args[1])).await
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
        "password" => prompt_password()?,
        _ => bail!("Choose panel access: open or password"),
    };
    Config {
        access,
        settings: Settings::default(),
        key: String::new(),
    }
    .save(&dir, true)?;
    println!("Panel access configured.");
    Ok(())
}

fn prompt_password() -> Result<Access> {
    let password = rpassword::prompt_password("Panel password: ")?;
    let confirm = rpassword::prompt_password("Confirm password: ")?;
    ensure!(password == confirm, "Passwords do not match");
    Ok(Access::Password {
        hash: hash_password(&password)?,
    })
}

async fn serve(dir: PathBuf, address: SocketAddr) -> Result<()> {
    let config = Config::load(&dir)?;
    let microphone: SocketAddr = env::var("GOODWIN_MICROPHONE")
        .unwrap_or_else(|_| "127.0.0.1:9850".into())
        .parse()
        .context("Invalid microphone address")?;
    ensure!(
        microphone.ip().is_loopback(),
        "The microphone address must be loopback"
    );
    let gain: f32 = env::var("GOODWIN_MIC_GAIN")
        .unwrap_or_else(|_| "4".into())
        .parse()
        .context("Invalid microphone gain")?;
    ensure!(
        (0.1..=64.0).contains(&gain),
        "Microphone gain must be between 0.1 and 64"
    );
    let app = App::new(dir, config, OpenRouter::new()?, Player::new());
    let listener = tokio::net::TcpListener::bind(address).await?;
    app.plugins.restore().await;
    println!(
        "Goodwin panel listening on http://{}",
        listener.local_addr()?
    );
    let capture = tokio::spawn(station::run_microphone(
        app.clone(),
        MicrophoneOptions {
            address: microphone,
            gain,
        },
    ));
    let result = axum::serve(listener, panel::router(app.clone()))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    capture.abort();
    app.plugins.shutdown().await;
    result.context("Serve Goodwin panel")
}

async fn answer_wav(input: PathBuf, output: PathBuf) -> Result<()> {
    let key = env::var("OPENROUTER_API_KEY").context("Set OPENROUTER_API_KEY")?;
    ensure!(
        fs::metadata(&input)?.len() <= 4_000_000,
        "Input WAV is too large"
    );
    let recording = fs::read(input)?;
    wav::check_recording(&recording)?;
    let provider = OpenRouter::new()?;
    let models = Models::for_file_request();
    let text = provider
        .transcribe(&key, &models, &recording)
        .await
        .context("Transcribe recording")?;
    ensure!(!text.is_empty(), "No speech was transcribed");
    let reply = provider
        .reply(&key, &models, &text, VOICE_INSTRUCTIONS, [])
        .await
        .context("Generate an answer")?;
    let audio = provider
        .synthesize(&key, &models, &reply)
        .await
        .context("Synthesize the answer")?;
    audio.write_wav(fs::File::create(output)?)?;
    Ok(())
}
