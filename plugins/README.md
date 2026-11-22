# Goodwin plugins

Goodwin starts trusted native plugins as independent processes. The [Timers plugin](timers/) uses the Rust SDK in [`sdk/`](sdk/) and provides a Solid page, a local scheduler, and assistant tools.

## Build a package

For a Windows development instance, run these commands from the repository root.

```powershell
npm ci --prefix web
npm run build --prefix web
cargo build --locked -p goodwin-timers --release
python scripts/package-plugin.py plugins/timers target/release/goodwin-timers.exe windows-x86_64 target/plugins/timers-windows-x86_64.tar.gz
```

Use `./scripts/build-station.ps1` on Windows to produce the separate Linux ARM64 package for Timers. Open **Plugins** in a password-protected panel to upload an archive. The manifest ID identifies the plugin across updates.

## Package and process contract

A `.tar.gz` package contains `manifest.json`, an executable under `bin/`, and optional prebuilt files under `ui/`. Use [the Timers manifest](timers/manifest.json) as a starting point. The packaging script adds `target` and `executable`. Keep the upload under 16 MiB and the expanded files under 48 MiB. Package paths use lowercase ASCII and forward slashes. Use regular files and include no explicit directory entries.

Goodwin runs the executable in its data directory and sets `GOODWIN_PLUGIN_DATA` to that directory. Keep persistent state there. Treat stdout as newline-delimited JSON-RPC 2.0 and send logs to stderr. The host sends `initialize` with `apiVersion: 1`. Reply with the same ID and a result containing `apiVersion` and any `routes`, `pages`, or `tools`. Publish readiness after the process can handle requests. The host waits up to five seconds for this response and closes stdin to stop the process.

- HTTP routes declare a method and a path such as `/timers/:id`. Goodwin serves them below `/api/plugins/{id}/`. The `http` request includes `method`, `path`, `query`, and the parsed JSON `body`. Reply with `status` and an optional JSON `body`.
- Pages declare an `id`, a relative `path`, an English and Russian `title`, and an optional `navigation` flag. An empty path maps to `/plugins/{id}`. The host publishes registered endpoints and pages together after readiness.
- Tools declare a lowercase `name`, a `description` and an object `parameters` schema. Specify `properties`, `required`, and `additionalProperties: false`. Properties may be strings, integers, numbers or booleans. Validate domain constraints in the plugin handler. The `tool` request includes `name`, `arguments` and `voiceTurn`, a counter for successive voice requests during the service run.
- Optionally set `voiceContextTool` to a registered read-only tool that accepts `{}`. Goodwin may call it before each voice request and send its result to the configured model as `pluginContext`, under the plugin ID. Return a small JSON object with current state. Keep credentials and private data out of this result. Provide the same validation in the action tools as for calls made without a snapshot.
- Send `play_timer_signal` as a JSON-RPC notification to request the host's short local timer sound.

## Web entry point

For a Solid page, set `web.solidRuntimeVersion` to `1` and export the page component as the default export of `ui/index.tsx`. With `web.entry` set to `ui/index.mjs`, the packaging script compiles the TSX into that module. List CSS files separately in `web.styles`. Imports declared by the [shared runtime](../web/src/helpers/plugin-runtime.ts) resolve to the Solid instance used by the panel.

The component receives `pageId`, `language`, `request(path, method, body)` for your API, and `navigate(pagePath)` for declared pages as props. Use `onCleanup` to release timers and other resources when the panel unmounts the page. A prebuilt `.mjs` entry can also export `mount(container, context)` and return its own cleanup function.

The panel provides `gw-button` with `variant="primary"` and `disabled`, `gw-card`, the `.gw-row` and `.gw-stack` utilities, and `--gw-color-*`, `--gw-space-*`, and `--gw-radius` CSS variables. Scope your styles to your page. The web module runs on the panel's origin with the current session, and the executable runs with the Goodwin service account's OS permissions.
