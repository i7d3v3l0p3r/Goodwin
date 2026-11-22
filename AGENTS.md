# Repository Guidelines

## Project Structure

Goodwin is a Rust voice assistant for Yandex Station Mini 3 (`bergamot`) with an embedded SolidJS 2 panel.

- `src/main.rs` handles CLI commands and service startup. `src/config.rs` owns persisted settings and the panel access mode. `src/app.rs` manages request state, `src/app/voice.rs` runs the voice pipeline, and `src/openrouter.rs` calls OpenRouter.
- `src/station/` handles microphone capture and playback. `src/wake.rs` and `src/wake/` handle LiveKit Standard and Micro ONNX inference.
- `src/panel.rs` registers HTTP APIs, `src/panel/auth.rs` handles first-run setup and sessions, and `src/panel/assets.rs` serves the embedded frontend. Register core page URLs in both `web/src/router.ts` and `src/panel/assets.rs` so direct links work.
- Keep page-specific components under their page directory in `web/src/pages/` and shared UI in `web/src/components/`.
- `src/plugins/` hosts native plugin processes. The Cargo workspace includes the SDK and timers in `plugins/`. Plugin packages declare their own routes and pages. See [the plugin development guide](plugins/README.md) for the process protocol, package format, and UI kit.

## Build and Run

Run commands from the repository root. Use Node.js 22.12+ or 24 LTS and build the panel before compiling or testing Rust because the binary embeds `web/dist/`.

- `npm ci --prefix web` installs locked frontend dependencies.
- `npm run build --prefix web` type-checks and bundles the panel.
- `cargo build --locked --release` builds the native binary.
- `cargo run --locked -- setup goodwin-data` initializes a new data directory without prompting for a password. Run `cargo run --locked -- serve goodwin-data` and open `http://127.0.0.1:8787` to create the password and sign in. `npm run dev --prefix web` starts Vite and proxies API and plugin asset requests to that address.
- `goodwin setup <data-dir> open` explicitly enables password-free access for a new data directory. Plugin package management requires a signed-in password session. To set or reset the password from the CLI, stop Goodwin, run `goodwin set-password <data-dir>`, then restart it.
- `./scripts/build-station.ps1` builds the `aarch64-unknown-linux-musl` service and separate plugin archives on Windows. With the trained models and encoder attribution files available, `python scripts/package-station.py` creates `target/goodwin-mini3-aarch64.tar.gz`. See [the README](README.md) for prerequisites and installation commands.

On a fresh Station installation, initialize the data directory, start Goodwin, and complete password setup from the panel on a trusted network. The first visitor who completes setup sets the password, and protected APIs stay locked until then. Reuse the existing data directory on upgrades to retain the password, settings, and plugin data.

## Style and Testing

Use four-space indentation in Rust and run `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings`. Use `snake_case` for Rust functions and modules. Follow the existing two-space TypeScript and TSX style, with PascalCase components and camelCase functions. Keep `*.module.css` beside its page or component and use `web/src/style.css` for shared variables and base styles. Update both languages in `web/src/helpers/words.ts` when changing UI copy. Use English for repository documentation.

Run `cargo test --locked --workspace` after building the panel. Keep tests with the module they exercise and share loopback server fixtures through `src/test_support.rs`. Use behavior-based names such as `numbered_cancellation_uses_the_original_timer_ids`. Cover changed behavior with Rust unit tests or Tokio tests. Use loopback fixtures for cloud and microphone tests. Report physical Station checks separately.

## Commits and Pull Requests

Use short, imperative subjects such as `Preserve wake audio pre-roll`. Describe the change and verification in the PR, link relevant issues, and include screenshots for visible UI changes. Keep API keys, recordings, generated builds, and local configuration out of commits.
