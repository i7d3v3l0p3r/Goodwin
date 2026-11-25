# Repository Guidelines

## Project Structure

Goodwin is a Rust voice assistant for Yandex Station Mini 3 (`bergamot`) with an embedded SolidJS 2 panel.

- `src/main.rs` selects the Linux or ESP-IDF platform. `src/platform/linux/mod.rs` handles the native CLI and service startup. `src/config.rs` owns persisted Station settings. `src/app.rs` and `src/builtin_assistant.rs` manage request state. Both use the voice pipeline in `src/voice_core.rs` and the panel contract in `src/panel_core.rs`.
- `src/assistant.rs` defines the assistant engine and its stage settings. The cascade (STT → LLM → TTS) resolves a provider connection and model independently for each stage through `src/providers.rs`. Connection settings belong to the provider plugin. Model selection and stage parameters such as TTS `voice` and `speed` belong to the assistant. Text chat uses the LLM stage.
- `src/station/` handles microphone capture, gain, and playback. `src/wake.rs` and `src/wake/` handle LiveKit Standard and Micro ONNX inference and the optional Micro INT8 TensorFlow Lite Micro backend. The panel and installer share catalog runtime handling in `shared/wake_catalog.rs`. Match a catalog model's runtime against the capabilities of the target build. Preserve raw audio until the wake and recognition paths apply their separate gains. An unset recognition gain uses automatic adjustment.
- `src/station/playback.rs` plays complete WAV replies and streams PCM from providers that support it. The cascade acts on `speech_streaming` in the assistant settings. Use buffered playback when the setting is off or the provider lacks the negotiated capability.
- `src/model/` selects the equipment and capabilities for each build. The Mini 3 indicator driver in `src/model/mini3/indicators/hardware.rs` uses the logo framebuffer and LED clock segments. Restore a settled clock frame after an interaction and give newer firmware frames priority.
- `src/panel.rs` registers HTTP APIs, `src/panel/auth.rs` handles first-run setup and sessions, and `src/panel/events.rs` delivers live updates. `src/panel/assets.rs` serves the frontend precompressed by `web/compress.mjs` during the build. Register core page URLs in both `web/src/router.ts` and `src/panel/assets.rs` so direct links work.
- `src/panel/diagnostics.rs` reports request and system diagnostics. `src/journal.rs` owns the rotating Goodwin journal. `src/core_update/` verifies and installs release binaries for managed Stations through the same deployment script used by the desktop installer. Keep journal access and update actions behind password authorization.
- Keep page-specific components under their page directory in `web/src/pages/` and shared UI in `web/src/components/`.
- Linux hosts native plugin processes. Linux and ESP-IDF share WASM validation and execution in `src/plugin_wasm.rs` and the package contract in `src/plugins/`. The root Cargo workspace includes the SDK, Timers, OpenAI-compatible, Wake-word dataset, and legacy OpenRouter packages. The provider contract lives in `plugins/sdk/src/provider.rs`. Negotiate PCM streaming during `initialize` and keep the buffered speech RPC available. Use the host HTTP and storage services for WASM plugins and preserve their cancellation and data-directory boundaries. See [the plugin development guide](plugins/README.md) for the RPC contract, package format, and UI kit.
- Declare ordinary plugin settings in the package manifest. Goodwin renders them at `/plugins/:id/settings`, and plugins read them through `goodwin_plugin::settings::read_settings`. The unified OpenAI-compatible WASM package uses `providerConnections` for named connections, including OpenRouter. Goodwin owns connection storage and applies API keys in the HTTP host. Keep provider IDs stable during migration and expose only whether a secret is configured to the browser. Plugin packages can also register their own pages and routes.
- `installer/` is a separate Cargo workspace. `installer/src/cli.rs` handles headless arguments and `installer/src/window.rs` contains the desktop UI. Both use `installer/src/station.rs` for ADB deployment and `installer/install-station.sh` for the on-device install and update steps.

Use [the glossary](GLOSSARY.md) for shared product terminology.

## Build and Run

Run commands from the repository root. Use Node.js 22.12+ or 24 LTS and build the panel before compiling or testing the root Rust workspace because Goodwin embeds the gzip files in `web/dist/`.

- `npm ci --prefix web` installs locked frontend dependencies.
- `npm run build --prefix web` type-checks, bundles, and compresses the panel.
- `cargo build --locked --release` builds the native binary.
- `python scripts/build-esp32s3.py` builds and packages the ESP32-S3 N16R8 firmware after activating the ESP-IDF and Xtensa Rust toolchains. Pass `--emulator` for the ESP-EMU build. See the [ESP32-S3 setup and test guide](firmware/esp32s3/README.md).
- `cargo run --locked -- setup goodwin-data` initializes a new data directory without prompting for a password. Run `cargo run --locked -- serve goodwin-data` and open `http://127.0.0.1:8787` to create the password and sign in. `npm run dev --prefix web` starts Vite and proxies API and plugin asset requests to that address.
- `goodwin setup <data-dir> open` explicitly enables password-free access for a new data directory. Plugin package management requires a signed-in password session. To set or reset the password from the CLI, stop Goodwin, run `goodwin set-password <data-dir>`, then restart it.
- `./scripts/build-station.ps1` builds the `aarch64-unknown-linux-musl` service with `wake-tflite,wasm-plugins` and packages native and WASM plugins on Windows. `python scripts/package-station.py` creates `target/goodwin-mini3-aarch64.tar.gz`. The installer and panel download wake models separately from [Goodwin-wakeword](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/latest). See [the README](README.md#build) for prerequisites and Linux cross-build commands.
- `cargo build --locked --release --manifest-path installer/Cargo.toml` builds the desktop installer without needing the panel build. Use `goodwin-installer install <address[:port]>` for headless installation. Add `--no-default-features` to the build command for a binary without desktop dependencies. See [headless installation](README.md#headless-installation) for the package, ADB, and wake-word model options.

On a fresh Station installation, complete password setup from the panel on a trusted network. For a manual deployment, initialize the data directory and start Goodwin first. The first visitor who completes setup sets the password, and protected APIs stay locked until then. Reuse the existing data directory on upgrades to retain the password, settings, selected model, and plugin data. Install `openai-compatible-wasm32.tar.gz` and configure a connection in Plugins before testing the assistant.

## Style and Testing

Use four-space indentation in Rust and run `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings`. Use `snake_case` for Rust functions and modules. Follow the existing two-space TypeScript and TSX style, with PascalCase components and camelCase functions. Keep `*.module.css` beside its page or component and use `web/src/style.css` for shared variables and base styles. Update both languages in `web/src/helpers/words.ts` when changing UI copy. Use English for repository documentation.

Run `cargo test --locked --workspace` after building the panel. Keep tests with the module they exercise and share loopback server fixtures through `src/test_support.rs`. Use behavior-based names such as `numbered_cancellation_uses_the_original_timer_ids`. Cover changed behavior with Rust unit tests or Tokio tests. Use loopback fixtures for cloud and microphone tests. Report physical Station checks separately.

On Linux x86-64, run `python scripts/build-wake-tflite.py` and `python scripts/build-wasm-plugins.py` before running `cargo test --locked --workspace --features goodwin/wake-tflite,goodwin/wasm-plugins` to cover the optional runtimes. The WASM tests use the actual guest builds and local HTTP fixtures.

Run installer tests separately with `cargo test --locked --release --manifest-path installer/Cargo.toml`. Also test with `--no-default-features` when changing headless behavior. On Linux, run `sh installer/tests/station-install.sh` when changing the on-device script.

## Commits and Pull Requests

Use short, imperative subjects such as `Preserve wake audio pre-roll`. Describe the change and verification in the PR, link relevant issues, and include screenshots for visible UI changes. Keep API keys, recordings, generated builds, and local configuration out of commits.

Record user-facing changes in the English and Russian sections under `[Unreleased]` in [the changelog](CHANGELOG.md). For a release, move those notes to a dated version entry, update the Goodwin and installer manifests and lockfiles, and push the matching `v<version>` tag. Confirm that the GitHub release workflow publishes all packages and `SHA256SUMS`. See [the release instructions](README.md#publish-a-release).
