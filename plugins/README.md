# Goodwin plugins

Goodwin starts trusted native plugins as independent processes. The probe plugin has a page and a persistent counter that exercise installation, navigation and HTTP calls. The timers plugin adds a local scheduler and assistant tools. Both use the Rust SDK in [`sdk/`](sdk/).

## Build a package

For a Windows development instance, run these commands from the repository root.

```powershell
cargo build --locked -p goodwin-probe --release
python scripts/package-plugin.py plugins/probe target/release/goodwin-probe.exe windows-x86_64 target/plugins/probe-windows-x86_64.tar.gz
```

Use `./scripts/build-station.ps1` on Windows to produce separate Linux ARM64 packages for the probe and timers. Open **Plugins** in a password-protected panel to upload an archive. The manifest ID identifies the plugin across updates.

## Package and process contract

A `.tar.gz` package contains `manifest.json`, an executable under `bin/`, and optional prebuilt files under `ui/`. Use [the probe manifest](probe/manifest.json) as a starting point. The packaging script adds `target` and `executable`. Keep the upload under 16 MiB and the expanded files under 48 MiB. Package paths use lowercase ASCII and forward slashes. Use regular files and include no explicit directory entries.

Goodwin runs the executable in its data directory and sets `GOODWIN_PLUGIN_DATA` to that directory. Keep persistent state there. Treat stdout as newline-delimited JSON-RPC 2.0 and send logs to stderr. The host sends `initialize` with `apiVersion: 1`. Reply with the same ID and a result containing `apiVersion` and any `routes`, `pages`, or `tools`. Publish readiness after the process can handle requests. The host waits up to five seconds for this response and closes stdin to stop the process.

- HTTP routes declare a method and a path such as `/timers/:id`. Goodwin serves them below `/api/plugins/{id}/`. The `http` request includes `method`, `path`, `query`, and the parsed JSON `body`. Reply with `status` and an optional JSON `body`.
- Pages declare an `id`, a relative `path`, an English and Russian `title`, and an optional `navigation` flag. An empty path maps to `/plugins/{id}`. The host publishes registered endpoints and pages together after readiness.
- Tools declare a lowercase `name`, a `description` and an object `parameters` schema. Specify `properties`, `required`, and `additionalProperties: false`. Properties may be strings, integers, numbers or booleans. Validate domain constraints in the plugin handler. The `tool` request includes `name`, `arguments` and `voiceTurn`, a counter for successive voice requests during the service run.
- Optionally set `voiceContextTool` to a registered read-only tool that accepts `{}`. Goodwin may call it before each voice request and send its result to the configured model as `pluginContext`, under the plugin ID. Return a small JSON object with current state. Keep credentials and private data out of this result. Provide the same validation in the action tools as for calls made without a snapshot.
- Send `play_timer_signal` as a JSON-RPC notification to request the host's short local timer sound.

## Web entry point

Export `mount(container, context)` from your packaged `.mjs` entry. Return a cleanup function that removes event listeners, pending work and timers. The context provides `pageId`, `language`, `request(path, method, body)` for your API, and `navigate(pagePath)` for declared pages.

The panel provides `gw-button` with `variant="primary"` and `disabled`, `gw-card`, the `.gw-row` and `.gw-stack` utilities, and `--gw-color-*`, `--gw-space-*`, and `--gw-radius` CSS variables. Scope your styles to your page. The web module runs on the panel's origin with the current session, and the executable runs with the Goodwin service account's OS permissions.
