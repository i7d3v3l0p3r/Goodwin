# Repository Guidelines

## Project Structure

Goodwin is a Rust voice assistant for Yandex Station Mini 3 with an embedded SolidJS 2 panel. The CLI is `src/main.rs`. Request state lives in `src/app.rs`, OpenRouter calls in `src/openrouter.rs`, and the Station audio, HTTP panel, and plugin host use `src/station/`, `src/panel/`, and `src/plugins/`. In `web/src/`, put page-specific components inside their page directory and shared UI in `components/`. Register core URLs in `router.ts` and serve their direct links from `src/panel/assets.rs`. Plugin packages declare their own routes and UI from `plugins/`.

## Build and Run

Run commands from the repository root. Build the panel before compiling or testing Rust because the binary embeds `web/dist/`.

- `npm ci --prefix web` installs locked frontend dependencies.
- `npm run build --prefix web` type-checks and bundles the panel.
- `cargo build --locked --release` builds the native binary.
- `cargo run --locked -- setup goodwin-data open` initializes local access once. Then run `cargo run --locked -- serve goodwin-data` to serve on `127.0.0.1:8787`.
- `npm run dev --prefix web` starts Vite and proxies API requests to the running Rust service.
- `./scripts/build-station.ps1` builds the Linux ARM64 service and separate plugin archives on Windows. See `README.md` for toolchain prerequisites.

## Style and Testing

Use four-space indentation in Rust and run `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings`. Use `snake_case` for Rust functions and modules. Follow the existing two-space TypeScript and TSX style, with PascalCase components and camelCase functions. Keep `*.module.css` beside its page or component and use `web/src/style.css` for shared variables and base styles. Update both languages in `web/src/helpers/words.ts` when changing UI copy. Use English for repository documentation.

Run `cargo test --locked --workspace` after building the panel. Keep tests with the module they exercise and share loopback server fixtures through `src/test_support.rs`. Use behavior-based names such as `numbered_cancellation_uses_the_original_timer_ids`. Cover changed behavior with Rust unit tests or Tokio tests. Use loopback fixtures for cloud and microphone tests. Report physical Station checks separately.

## Commits and Pull Requests

Git history contains only the initial commit. Use short, imperative subjects such as `Preserve wake audio pre-roll`. Describe the change and verification in the PR, link relevant issues, and include screenshots for visible UI changes. Keep API keys, recordings, generated builds, and local configuration out of commits.
