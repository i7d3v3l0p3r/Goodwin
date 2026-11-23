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

Use `./scripts/build-station.ps1` on Windows to produce the separate Linux ARM64 packages for Timers, [OpenRouter](openrouter/) and the [OpenAI-compatible provider](openai-compatible/). Open **Plugins** in a password-protected panel to upload an archive. The manifest ID identifies the plugin across updates.

## Package and process contract

A `.tar.gz` package contains `manifest.json`, an executable under `bin/`, and optional prebuilt files under `ui/`. Use [the Timers manifest](timers/manifest.json) as a starting point. The packaging script adds `target` and `executable`. Keep the upload under 16 MiB and the expanded files under 48 MiB. Package paths use lowercase ASCII and forward slashes. Use regular files and include no explicit directory entries.

Goodwin runs the executable in its data directory and sets `GOODWIN_PLUGIN_DATA` to that directory. Keep persistent state there. Treat stdout as newline-delimited JSON-RPC 2.0 and send logs to stderr. The host sends `initialize` with `apiVersion: 1`. Reply with the same ID and a result containing `apiVersion` and any `routes`, `pages`, or `tools`. Publish readiness after the process can handle requests. The host waits up to five seconds for this response and closes stdin to stop the process.

- HTTP routes declare a method and a path such as `/timers/:id`. Goodwin serves them below `/api/plugins/{id}/`. The `http` request includes `method`, `path`, `query`, and the parsed JSON `body`. Reply with `status` and an optional JSON `body`.
- Pages declare an `id`, a relative `path`, an English and Russian `title`, and an optional `navigation` flag. An empty path maps to `/plugins/{id}`. The host publishes registered endpoints and pages together after readiness.
- Tools declare a lowercase `name`, a `description` and an object `parameters` schema. Specify `properties`, `required`, and `additionalProperties: false`. Properties may be strings, integers, numbers or booleans. Validate domain constraints in the plugin handler. The `tool` request includes `name`, `arguments` and `voiceTurn`, a counter for successive voice requests during the service run.
- Optionally set `voiceContextTool` to a registered read-only tool that accepts `{}`. Goodwin may call it before each voice request and send its result to the configured model as `pluginContext`, under the plugin ID. Return a small JSON object with current state. Keep credentials and private data out of this result. Provide the same validation in the action tools as for calls made without a snapshot.
- Send `play_timer_signal` as a JSON-RPC notification to request the host's short local timer sound.

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

Declare a `providers` array in the initialize result with any of `transcription`, `chat` and `speech`. The assistant's cascade engine selects one provider and one model for each stage. A model provider owns the connection to the upstream service. The default assignment uses the OpenRouter plugin ID.

| Capability | JSON-RPC method | Request | Result |
| --- | --- | --- | --- |
| `transcription` | `provider.transcribe` | `{wavBase64, language}` with a standard Base64-encoded WAV and a language code | `{text}` |
| `chat` | `provider.complete` | `{messages, max_completion_tokens}` with optional `tools` and `tool_choice`, following the Chat Completions schema | `{message}` for the first completion, with any tool calls and provider-specific fields preserved |
| `speech` | `provider.synthesize` | `{text}` | `{pcmBase64, sampleRate, channels}` with standard Base64-encoded signed 16-bit little-endian PCM |

Advertise `modelCatalog: true` and implement `provider.models` to accept model selections. The catalog request is `{capability}` and returns `{models, default, available}`. Each model has an `id`, an optional `name`, and optional `voices`. When an upstream catalog is absent or temporarily unreachable, set `available` to false. The panel still accepts a model ID manually.

On inference calls, the optional `selection` object carries a model ID and provider-specific `parameters`, such as `{"model":"vendor/speech","parameters":{"voice":"Russian"}}`. The plugin supplies its default model when the selection omits the ID. Wire fields such as `model`, `messages`, `input_audio`, `response_format` and `stream` are reserved and cannot be overridden through `parameters`.

Use the types in `goodwin_plugin::provider` and implement the corresponding SDK trait methods. The host sends each synthesis request as one text string. PCM may use 8–96 kHz and one or two channels. WAV input is capped at 4,000,000 bytes, PCM at 8,000,000 bytes, and chat RPC frames at 512 KiB. Provider calls have a 60-second execution deadline. Each plugin process handles requests serially.

For incremental PCM playback, implement `supports_speech_streaming` and `synthesize_stream`. The SDK advertises `speechStreaming: true` when the host sends the same flag in `initialize`. The streaming call uses `provider.synthesize_stream` with the usual speech request. Each emitted `SpeechAudio` becomes a `provider.speech_chunk` notification with `{requestId, audio}`. Emit complete PCM frames in chunks up to 16 KiB with a consistent sample rate and channel count, followed by the normal RPC result or error. The host reads each stream frame with a 60-second timeout.

## Web entry point

For a Solid page, set `web.solidRuntimeVersion` to `1` and export the page component as the default export of `ui/index.tsx`. With `web.entry` set to `ui/index.mjs`, the packaging script compiles the TSX into that module. List CSS files separately in `web.styles`. Imports declared by the [shared runtime](../web/src/helpers/plugin-runtime.ts) resolve to the Solid instance used by the panel.

The component receives `pageId`, `language`, `request(path, method, body)` for your API, and `navigate(pagePath)` for declared pages as props. Use `onCleanup` to release timers and other resources when the panel unmounts the page. A prebuilt `.mjs` entry can also export `mount(container, context)` and return its own cleanup function.

The panel provides `gw-button` with `variant="primary"` and `disabled`, `gw-card`, the `.gw-row` and `.gw-stack` utilities, and `--gw-color-*`, `--gw-space-*`, and `--gw-radius` CSS variables.

Use `.gw-button` on a native button and set `data-variant="primary"` for the primary action or `data-variant="quiet"` for a low-emphasis action. Set `type="submit"` on the native button for forms. The `gw-button` element continues to act as a regular action button.

Use `.gw-field` to group a label and control, `.gw-field-label` for the label text, and `.gw-control` on a text or number input, select, textarea, or range. Use `.gw-field-hint` and `.gw-field-error` for text linked by `aria-describedby` and mark invalid controls with `aria-invalid="true"`. For an on/off control, use a native `button type="button"` with `.gw-switch`, `role="switch"`, an accessible name, and `aria-checked`. Add a decorative `.gw-switch-track` span inside the button.

Use `.gw-status` with `data-tone="pending"`, `"success"`, or `"error"` and a readable status label. Use `.gw-inline-error` for error text and `.gw-empty-state` for a confirmed empty result. To announce changes, put loading or empty text in a persistent `role="status"` region and use `role="alert"` for newly reported errors.

Use `.gw-disclosure` on a native `details` element with a `summary` to group collapsible content. Use `.gw-save-actions` around the native submit button and a `.gw-save-status` element with `role="status"` for progress and save results.

Plugin pages can use `--gw-page-content-width` with a fallback for their own centered content. The host gives plugin pages the full available width.

Scope your plugin styles to your page. The web module runs on the panel's origin with the current session, and the executable runs with the Goodwin service account's OS permissions.
