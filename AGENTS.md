# Repository Guidelines

## Project Structure

Goodwin is a Rust voice assistant for Yandex Station Mini 3 (`bergamot`) with an embedded SolidJS 2 panel.

- `src/main.rs` handles CLI commands and service startup. `src/config.rs` owns persisted settings and the panel access mode. `src/app.rs` manages request state and `src/app/voice.rs` runs the voice pipeline.
- `src/assistant.rs` defines the assistant engine and its stage settings. The cascade (STT → LLM → TTS) resolves a provider and model independently for each stage through `src/providers.rs`. Provider connection settings belong to the plugin. Model selection and stage parameters such as TTS `voice` and `speed` belong to the assistant. Text chat uses the LLM stage.
- `src/station/` handles microphone capture, gain, and playback. `src/wake.rs` and `src/wake/` handle LiveKit Standard and Micro ONNX inference. Preserve raw audio until the wake and recognition paths apply their separate gains. An unset recognition gain uses automatic adjustment.
- `src/station/playback.rs` plays complete WAV replies and streams PCM from providers that support it. The cascade acts on `speech_streaming` in the assistant settings. Use buffered playback when the setting is off or the provider lacks the negotiated capability.
- `src/station/indicators.rs` drives the eyes, ring, and audio-reactive mouth during voice turns. The Station Mini 3 backend in `src/station/indicators/hardware.rs` uses the logo framebuffer and LED clock segments. Restore a settled clock frame after an interaction and give newer firmware frames priority.
- `src/panel.rs` registers HTTP APIs, `src/panel/auth.rs` handles first-run setup and sessions, and `src/panel/assets.rs` serves the embedded frontend. Register core page URLs in both `web/src/router.ts` and `src/panel/assets.rs` so direct links work.
- Keep page-specific components under their page directory in `web/src/pages/` and shared UI in `web/src/components/`.
- `src/plugins/` hosts native plugin processes. The root Cargo workspace includes the SDK, Timers, OpenRouter, and OpenAI-compatible plugins in `plugins/`. The model provider contract lives in `plugins/sdk/src/provider.rs`. Negotiate PCM streaming during `initialize` and keep the buffered speech RPC available. See [the plugin development guide](plugins/README.md) for the process protocol, package format, and UI kit.
- Declare plugin settings in the package manifest. Goodwin renders the form at `/plugins/:id/settings`, stores values per plugin, and exposes only the presence of saved secrets to the browser. Plugins read values through `goodwin_plugin::settings::read_settings`. Plugin packages can also register their own pages and routes.
- `installer/` is a separate Cargo workspace. `installer/src/cli.rs` handles headless arguments and `installer/src/window.rs` contains the desktop UI. Both use `installer/src/station.rs` for ADB deployment and `installer/install-station.sh` for the on-device install and update steps.

Use [the glossary](GLOSSARY.md) for shared product terminology.

## Build and Run

Run commands from the repository root. Use Node.js 22.12+ or 24 LTS and build the panel before compiling or testing the root Rust workspace because the Goodwin binary embeds `web/dist/`.

- `npm ci --prefix web` installs locked frontend dependencies.
- `npm run build --prefix web` type-checks and bundles the panel.
- `cargo build --locked --release` builds the native binary.
- `cargo run --locked -- setup goodwin-data` initializes a new data directory without prompting for a password. Run `cargo run --locked -- serve goodwin-data` and open `http://127.0.0.1:8787` to create the password and sign in. `npm run dev --prefix web` starts Vite and proxies API and plugin asset requests to that address.
- `goodwin setup <data-dir> open` explicitly enables password-free access for a new data directory. Plugin package management requires a signed-in password session. To set or reset the password from the CLI, stop Goodwin, run `goodwin set-password <data-dir>`, then restart it.
- `./scripts/build-station.ps1` builds the `aarch64-unknown-linux-musl` service and separate plugin archives on Windows. `python scripts/package-station.py` creates `target/goodwin-mini3-aarch64.tar.gz`. The installer and panel download wake models separately from [Goodwin-wakeword](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/latest). See [the README](README.md) for prerequisites and installation commands.
- `cargo build --locked --release --manifest-path installer/Cargo.toml` builds the desktop installer without needing the panel build. Use `goodwin-installer install <address[:port]>` for headless installation. Add `--no-default-features` to the build command for a binary without desktop dependencies. See [headless installation](README.md#headless-installation) for the package, ADB, and wake-word model options.

On a fresh Station installation, complete password setup from the panel on a trusted network. For a manual deployment, initialize the data directory and start Goodwin first. The first visitor who completes setup sets the password, and protected APIs stay locked until then. Reuse the existing data directory on upgrades to retain the password, settings, and plugin data. Install a provider package and configure its connection in Plugins before testing the assistant.

## Style and Testing

Use four-space indentation in Rust and run `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings`. Use `snake_case` for Rust functions and modules. Follow the existing two-space TypeScript and TSX style, with PascalCase components and camelCase functions. Keep `*.module.css` beside its page or component and use `web/src/style.css` for shared variables and base styles. Update both languages in `web/src/helpers/words.ts` when changing UI copy. Use English for repository documentation.

Run `cargo test --locked --workspace` after building the panel. Keep tests with the module they exercise and share loopback server fixtures through `src/test_support.rs`. Use behavior-based names such as `numbered_cancellation_uses_the_original_timer_ids`. Cover changed behavior with Rust unit tests or Tokio tests. Use loopback fixtures for cloud and microphone tests. Report physical Station checks separately.

Run installer tests separately with `cargo test --locked --release --manifest-path installer/Cargo.toml`. Also test with `--no-default-features` when changing headless behavior. On Linux, run `sh installer/tests/station-install.sh` when changing the on-device script.

## Commits and Pull Requests

Use short, imperative subjects such as `Preserve wake audio pre-roll`. Describe the change and verification in the PR, link relevant issues, and include screenshots for visible UI changes. Keep API keys, recordings, generated builds, and local configuration out of commits.

Record user-facing changes in the English and Russian sections under `[Unreleased]` in [the changelog](CHANGELOG.md).
