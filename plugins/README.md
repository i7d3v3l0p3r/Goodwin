# Goodwin plugins

Goodwin starts trusted native plugins as independent processes and runs WASM plugins through WAMR Fast. The [Timers plugin](timers/) uses the Rust SDK in [`sdk/`](sdk/) and provides a Solid page, a local scheduler, and assistant tools.

## Build a package

For a Windows development instance, run these commands from the repository root.

```powershell
npm ci --prefix web
npm run build --prefix web
cargo build --locked -p goodwin-timers --release
python scripts/package-plugin.py plugins/timers target/release/goodwin-timers.exe windows-x86_64 target/plugins/timers-windows-x86_64.tar.gz
```

Use `./scripts/build-station.ps1` on Windows to produce the separate Station packages for Timers, the [OpenAI-compatible provider](openai-compatible/) and [Wake-word dataset](wake-dataset/). Open **Plugins** in a password-protected panel to upload an archive. The manifest ID identifies the plugin across updates.

## Package and process contract

A `.tar.gz` package contains `manifest.json`, an executable under `bin/`, and optional prebuilt files under `ui/`. Use [the Timers manifest](timers/manifest.json) as a starting point. The packaging script adds `target` and `executable`. Keep the upload under 16 MiB and the expanded files under 48 MiB. Package paths use lowercase ASCII and forward slashes. Use regular files and include no explicit directory entries.

Goodwin runs the executable in its data directory and sets `GOODWIN_PLUGIN_DATA` to that directory. Keep persistent state there. Treat stdout as newline-delimited JSON-RPC 2.0 and send logs to stderr. The host sends `initialize` with `apiVersion: 1`. Reply with the same ID and a result containing `apiVersion` and any `routes`, `pages`, or `tools`. Publish readiness after the process can handle requests. The host waits up to five seconds for this response and closes stdin to stop the process.

- HTTP routes declare a method and a path such as `/timers/:id`. Goodwin serves them below `/api/plugins/{id}/`. Set `password_required: true` on private routes to require a signed-in session. The `http` request includes `method`, `path`, `query`, and the parsed JSON `body`. Reply with `status` and an optional JSON `body`.
- Pages declare an `id`, a relative `path`, an English and Russian `title`, and an optional `navigation` flag. An empty path maps to `/plugins/{id}`. The host publishes registered endpoints and pages together after readiness. Set `navigation` to `true` to list a page in the sidebar and mobile menu. Use the optional `icon` field on that page, such as `"icon": "timer"`.
- Tools declare a lowercase `name`, a `description` and an object `parameters` schema. Specify `properties`, `required`, and `additionalProperties: false`. Properties may be strings, integers, numbers or booleans. Validate domain constraints in the plugin handler. The `tool` request includes `name`, `arguments` and `voiceTurn`, a counter for successive voice requests during the service run.
- Optionally set `voiceContextTool` to a registered read-only tool that accepts `{}`. Goodwin may call it before each voice request and send its result to the configured model as `pluginContext`, under the plugin ID. Return a small JSON object with current state. Keep credentials and private data out of this result. Provide the same validation in the action tools as for calls made without a snapshot.
- Send `play_timer_signal` as a JSON-RPC notification to request the host's short local timer sound.

Supported Lucide names are `activity`, `audio-lines`, `calendar-days`, `database`, `download`, `headphones`, `house`, `list-checks`, `message-square-text`, `mic`, `monitor`, `plug`, `settings`, `sparkles`, and `timer`. Goodwin uses the plug icon when the name is omitted or unknown.

## WASM plugins

Station and ESP32-S3 builds use the same WAMR Fast adapter and guest contract. To use it on a Linux development host, install Git, CMake 3.23 or newer, and Ninja, build the panel, and enable the Cargo feature `wasm-plugins`. The first build fetches the pinned WAMR sources. See the [ESP32-S3 guide](../firmware/esp32s3/README.md) for that target's flash, HTTP and memory limits. To build the WASM plugins and the test fixture, install Zig and the Rust target `wasm32-unknown-unknown`, then run `python scripts/build-wasm-plugins.py`.

The Timers WASM build uses `no_std` with `alloc` and enables full LTO in the build script.

Use `wasm32` as the package target. The Station build packages Timers, OpenAI-compatible and Wake-word dataset as `target/plugins/<id>-wasm32.tar.gz`. To package Timers manually, run the following command and upload the archive from a password-protected panel.

```sh
python scripts/package-plugin.py plugins/timers target/wasm32-unknown-unknown/release/goodwin-timers.wasm \
  wasm32 target/plugins/timers-wasm32.tar.gz --wasm-state-file timers.json
```

The guest is a core WebAssembly module with one 32-bit linear memory. Export these two functions.

- `goodwin_buffer() -> i32` returns the offset of a writable 128 KiB exchange buffer in linear memory.
- `goodwin_call(length: i32) -> i32` reads a JSON-RPC 2.0 request in the buffer, writes its reply at the same offset, and returns the reply's byte length. JSON uses UTF-8.

Goodwin sends `initialize`, `http` and `tool` requests with the same parameters and result shapes as native plugins. Each request includes `nowMs` with the wall clock in Unix milliseconds. A WASM plugin can register pages, HTTP routes and assistant tools.

- Set `wasmStateFile` to a JSON filename in the plugin data directory to receive its saved value in `initialize.params.state`. When a successful reply includes a top-level `state` field, Goodwin atomically saves that value before confirming the call. The saved state limit is 64 KiB. Timers uses the same `timers.json` as the native package.
- Return a top-level `wakeAt` Unix millisecond timestamp to schedule a one-shot `tick` request with empty parameters. Return `null` to cancel a scheduled request or omit the field to retain a schedule. A delivered tick consumes the schedule.
- The optional import `goodwin.play_timer_signal() -> ()` queues a local signal after a successful reply and any requested state save.

Use exports for initialization and request handling. The host validates each module before instantiation. Calls have an instruction budget and a deadline. Traps stop the affected plugin. The module limit is 512 KiB. The host applies its normal panel authorization to WASM routes. A plugin's browser UI has the same origin privileges as native plugin UI.

For plugins that use host services, enable the SDK's `wasm-guest` feature for the WASM target and use `goodwin_plugin::wasm_plugin!`. The SDK exports `goodwin_input(length)` to allocate an exchange buffer on demand. `goodwin_buffer()` returns the reply offset after each call. Each plugin runs its calls in order. An upstream HTTP request can wait for network I/O while other plugins run.

- `wasmHttp` enables provider calls. Set `baseUrl` to a fixed HTTPS API base or `baseUrlSetting` to a URL field. Set `keySetting` to the secret field used for bearer authentication. `requireKey` requires a key for POST requests. Goodwin sends HTTP and streams response chunks through its shared client with redirects disabled. The guest receives public settings and parses provider responses. Goodwin applies the bearer key in the host and cancels pending HTTP operations when the plugin stops.
- `providerConnections` with `wasmHttp` enables named connections. Goodwin resolves the selected connection before invoking the guest and keeps its API URL and key in the host. In this mode the guest reads the selected protocol through `read_settings`. Named provider IDs have the form `<plugin-id>:<connection-id>`. A queued call for a removed connection fails independently, and other connections continue serving requests.
- `wasmStorage` enables reads and writes relative to the plugin's data directory and lets the guest register for wake events. Wake-word dataset uses the same `.clip` format and `capture.json` as the native plugin.
- Linear memory can grow up to 512 KiB for base plugins, 8 MiB for storage plugins and 64 MiB for HTTP providers. The provider limit accommodates the existing buffered recording and speech payloads.

## Declarative settings

Declare a `settings` array in `manifest.json` for fields that Goodwin should render and store. A plugin with only a settings form can omit `web` entirely. Plugins with their own pages link to the same settings form from their page headers. Settings also open from the Plugins list and can be edited while the plugin process is unavailable. Plugins using declarative settings reserve the `settings` page path for the panel.

```json
{
  "settings": [
    {
      "id": "api_url",
      "type": "url",
      "label": { "en": "API base URL", "ru": "Базовый URL API" },
      "required": true
    },
    {
      "id": "token",
      "type": "secret",
      "label": { "en": "API key", "ru": "Ключ API" }
    }
  ]
}
```

Use lowercase snake_case IDs and keep them stable across releases. Supported types are `text`, `url` (an HTTP(S) base URL without credentials, query or fragment), `secret`, `boolean` and `integer` (a JavaScript safe integer). Fields may have a localized `description`. String fields may set a `maxLength` in UTF-8 bytes. Unset optional fields are omitted from the data. For each secret the panel API reports whether it is configured.

Read the current values with `goodwin_plugin::settings::read_settings::<T>()`, where `T` is your deserializable struct. Use Serde defaults for values the plugin can use before setup and read settings again at the start of a request if edits should take effect immediately. Goodwin saves atomically in the plugin's data directory and retains those values during package updates.

## Providers

Declare a `providers` array in the initialize result with any of `transcription`, `chat` and `speech`. The assistant's cascade engine selects one provider and one model for each stage. A model provider owns the connection to the upstream service. The default assignment uses the `openai-compatible:openrouter` connection.

| Capability | JSON-RPC method | Request | Result |
| --- | --- | --- | --- |
| `transcription` | `provider.transcribe` | `{wavBase64, language}` with a standard Base64-encoded WAV and a language code | `{text}` |
| `chat` | `provider.complete` | `{messages, max_completion_tokens}` with optional `tools` and `tool_choice`, following the Chat Completions schema | `{message}` for the first completion, with any tool calls and provider-specific fields preserved |
| `speech` | `provider.synthesize` | `{text}` | `{pcmBase64, sampleRate, channels}` with standard Base64-encoded signed 16-bit little-endian PCM |

Advertise `modelCatalog: true` and implement `provider.models` to accept model selections. The catalog request is `{capability}` and returns `{models, default, available}`. Each model has an `id`, an optional `name`, and optional `voices`. When an upstream catalog is absent or temporarily unreachable, set `available` to false. The panel still accepts a model ID manually.

On inference calls, the optional `selection` object carries a model ID and provider-specific `parameters`, such as `{"model":"vendor/speech","parameters":{"voice":"Russian"}}`. The plugin supplies its default model when the selection omits the ID. Wire fields such as `model`, `messages`, `input_audio`, `response_format` and `stream` are reserved and cannot be overridden through `parameters`.

Use the types in `goodwin_plugin::provider` and implement the corresponding SDK trait methods. The host sends each synthesis request as one text string. PCM may use 8–96 kHz and one or two channels. WAV input is capped at 4,000,000 bytes, PCM at 8,000,000 bytes, and chat RPC frames at 512 KiB. Provider calls have a 60-second execution deadline. Each plugin process handles requests serially.

For incremental PCM playback, implement `supports_speech_streaming` and `synthesize_stream`. The SDK advertises `speechStreaming: true` when the host sends the same flag in `initialize`. The streaming call uses `provider.synthesize_stream` with the usual speech request. Each emitted `SpeechAudio` becomes a `provider.speech_chunk` notification with `{requestId, audio}`. Emit complete PCM frames in chunks up to 16 KiB with a consistent sample rate and channel count, followed by the normal RPC result or error. The host reads each stream frame with a 60-second timeout.

To stream plain text replies to the panel, implement `supports_chat_streaming` and `complete_stream`. The SDK advertises `chatStreaming: true` when the host sends that flag in `initialize`. The host calls `provider.complete_stream` with the chat request and no tools. Emit nonempty UTF-8 text chunks of up to 16 KiB and no more than 256,000 bytes total. The SDK sends each chunk as a `provider.chat_chunk` notification with `{requestId, text}`, followed by a null result or an RPC error. Keep `provider.complete` available for the voice pipeline and hosts without streaming support.

## Wake-word events

Goodwin offers `wakeEvents` during initialization. Implement `supports_wake_events`, `wake_subscription`, and `wake_event` in the Rust SDK to observe the wake detector. Return zero from `wake_subscription` to disable delivery and a fresh nonzero token to subscribe. The SDK reports the initial token in the initialize result and any changes on subsequent RPC responses. Observations include that token so a plugin can discard queued events from a previous subscription. Require a signed-in session on panel routes that control a subscription or expose microphone data.

Each `wake.event` request describes either a scored frame or the end of a detector stream. A frame has a stream ID, a frame index within that stream, raw mono PCM encoded as standard Base64, the model identity, the gain used to score the frame, the model's score, and the configured threshold. A stream ends whenever the detector resets or stops listening. Event delivery has a bounded queue and uses sequence numbers so plugins can detect gaps and decide how to handle them.

## Web entry point

For a Solid page, set `web.solidRuntimeVersion` to `1` and export the page component as the default export of `ui/index.tsx`. With `web.entry` set to `ui/index.mjs`, the packaging script compiles the TSX into that module. List CSS files separately in `web.styles`. Imports declared by the [shared runtime](../web/src/helpers/plugin-runtime.ts) resolve to the Solid instance used by the panel.

The component receives `pageId`, `language`, `request(path, method, body)` for your API, and `navigate(pagePath)` for declared pages as props. Use `onCleanup` to release timers and other resources when the panel unmounts the page. A prebuilt `.mjs` entry can also export `mount(container, context)` and return its own cleanup function.

The panel also supplies `protectDraft(dirty)`. Register when a form becomes dirty, pass a function that returns whether its draft is unsaved, and call the returned cleanup after saving, discarding, or unmounting the draft.

The panel provides `gw-button` with `variant="primary"` and `disabled`, `gw-card`, the `.gw-row` and `.gw-stack` utilities, and `--gw-color-*`, `--gw-space-*`, and `--gw-radius` CSS variables.

Use `.gw-button` on a native button and set `data-variant="primary"` for the primary action or `data-variant="quiet"` for a low-emphasis action. Set `type="submit"` on the native button for forms. The `gw-button` element continues to act as a regular action button.

Use `.gw-field` to group a label and control, `.gw-field-label` for the label text, and `.gw-control` on a text or number input, select, textarea, or range. Use `.gw-field-hint` and `.gw-field-error` for text linked by `aria-describedby` and mark invalid controls with `aria-invalid="true"`. For an on/off control, use a native `button type="button"` with `.gw-switch`, `role="switch"`, an accessible name, and `aria-checked`. Add a decorative `.gw-switch-track` span inside the button.

Use `.gw-status` with `data-tone="pending"`, `"success"`, or `"error"` and a readable status label. Use `.gw-inline-error` for error text and `.gw-empty-state` for a confirmed empty result. To announce changes, put loading or empty text in a persistent `role="status"` region and use `role="alert"` for newly reported errors.

Use `.gw-disclosure` on a native `details` element with a `summary` to group collapsible content. Use `.gw-save-actions` around the native submit button and a `.gw-save-status` element with `role="status"` for progress and save results.

Plugin pages can use `--gw-page-content-width` with a fallback for their own centered content. The host gives plugin pages the full available width.

Scope your plugin styles to your page. The web module runs on the panel's origin with the current session, and the executable runs with the Goodwin service account's OS permissions.
