# Goodwin on Yandex Station Mini 3

Install the service and connect a model provider to hear your first reply. For a fresh installation, keep your computer and Station on a trusted network while you set up panel access.

## Desktop installer

The Rust installer runs on Windows, Linux and macOS. It installs Goodwin on a prepared Yandex Station Mini 3 over network ADB with root access and support for the `/data/init.d` service directory. Download the installer for your operating system from [GitHub Releases](https://github.com/i7d3v3l0p3r/Goodwin/releases/latest). Install [Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools) and put ADB on your `PATH`, or select the ADB executable under **Дополнительные настройки**.

1. Select a Station from the list. The installer starts an mDNS search for Mini 3 on launch. Click **Обновить** to search again or **Указать адрес вручную** to enter an IP address or hostname. Add an ADB port if it differs from 5555.
2. Click **Проверить подключение** to check ADB access and Station compatibility. Confirm the target and package source, then choose a wake word and model tier. The default keeps the current model on updates and downloads «Гудвин» Standard on a fresh installation.
3. Click **Установить Goodwin** or **Обновить Goodwin**. The installer verifies the Station package and any model download with SHA-256, starts the service and enables it at boot.
4. After a fresh installation, click **Настроить Goodwin** and choose how to access the panel. Use a password to install the provider add-on and check text and voice replies.

For local testing, open **Дополнительные настройки → Локальная сборка** and select an archive or a directory containing `goodwin-mini3-aarch64.tar.gz` or an extracted package. An unpackaged test build needs the AArch64 `goodwin` binary. If the directory contains `SHA256SUMS`, make sure it matches the local build. Click **Сбросить** in **Локальная сборка** to resume installing from GitHub.

Updates to the managed Station service reuse its data directory at `/data/opt/etc/goodwin` and keep its wake model selection by default, including models from older bundled releases. When there is room, the installer uploads and extracts the package in `/tmp`. If the startup check fails, it restores the previous version and restarts it if it was running.

### Headless installation

Pass the address of a prepared Station to install or update Goodwin without opening a window.

```sh
goodwin-installer install station.local
goodwin-installer install 192.168.1.42:5555 --package ./goodwin-mini3-aarch64.tar.gz --model egor-micro
```

The default ADB port is 5555 and the default package is the latest stable release. Use `--package <path>` to select a local archive or build directory, `--adb <path>` to select an ADB executable, and `--model <id>` to select a model ID from the latest release's [metadata.json](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/latest/download/metadata.json). The default `keep` retains the model on updates and downloads `goodwin-standard` for a fresh installation. Put IPv6 addresses in brackets, as in `[fd00::42]:5555`.

The command writes progress and errors to stderr. On success it exits with status 0 and prints the panel URL to stdout. An installation failure returns status 1 and invalid arguments return status 2. On a fresh installation, the panel asks how you want to sign in.

The regular installer supports both modes. To build it without desktop dependencies, run `cargo build --locked --release --no-default-features --manifest-path installer/Cargo.toml`.

### Update from the web panel

On a Station set up with the installer, sign in and open **Station → Goodwin update**. Check for updates, then select **Install and restart** when a newer stable release is available. The Station needs HTTPS access to GitHub to download and verify the package. If the update page is missing, use the installer to install a build with web update support first.

Goodwin checks that the panel starts after the restart and restores the previous executable if startup fails. Settings, plugins and the selected wake-word model are retained. The current chat clears when Goodwin restarts. Reload the panel when it reports that the update is complete.

## Manual installation

Copy the Linux binary to a writable, executable directory on a prepared Mini 3. Leave the Station's existing voice services running. Goodwin connects to `yio_audio` at `127.0.0.1:9850` and uses `aplay -D speakers`. Its configuration and model go in a separate data directory.

```sh
chmod +x ./goodwin
./goodwin setup ./goodwin-data
./goodwin serve ./goodwin-data 0.0.0.0:8787
```

Open the Station's address on port 8787 and choose whether to use a password. To install `openai-compatible-wasm32.tar.gz` from **Set up Goodwin**, sign in with a password. Configure OpenRouter or another compatible API and check text and voice replies. Keep the panel on a trusted network. It uses plain HTTP, and the first visitor who completes setup chooses the access mode. To initialize a data directory with open access, run `goodwin setup ./goodwin-data open`.

On the Station, extract the package with `gzip -dc goodwin-mini3-aarch64.tar.gz | tar -xf -`. In the extracted directory, run `./start.sh` after initializing the data directory. Use `GOODWIN_PANEL_ADDRESS=0.0.0.0:8787` to open the panel to your trusted network and `GOODWIN_DATA_DIR` to reuse an existing data directory.

## Voice and microphone settings

To enable or change voice activation, open **Station → Activation and microphone**. Choose a model and click **Download and switch**. The Station needs access to GitHub for the download. The new model takes effect without restarting the service and is saved for subsequent starts.

On a manual installation, you can also place a local LiveKit v2 Standard or Micro ONNX at `goodwin-data/goodwin.onnx` before starting Goodwin. Goodwin reads the managed selection first and uses the local model when no catalog model is selected. Set `GOODWIN_WAKE_MODEL` to use a different local file. Run `goodwin wake-runtimes` to list the runtimes compiled into the binary. Use `goodwin wake-check <model.onnx>` or `goodwin wake-check <model.tflite> <metadata.json>` to check loading, warm-up, and one inference step without the microphone. The Station build supports Micro INT8 TFLite and uses the catalog threshold at 50% sensitivity.

Say the wake word once to start a conversation. By default, you have eight seconds after each spoken answer to start your next question. Follow-ups use the recent exchanges in that voice conversation. If you stay quiet, Goodwin silently ends the conversation. In **Settings → Activation and microphone → Voice conversation**, change the wait time (1–60 seconds) or choose one request per wake word. Devices with a push-to-talk button also offer one request per button press. Panel microphone tests handle a single request. Numbered timer choices carry over to the next voice request. The current web conversation lives in memory and clears when you start a new chat or stop Goodwin.

The text chat and push-button Station test work without a wake model. In Settings you can pause background detection, change sensitivity, test the speaker and microphone, and see timings for the last request. Pausing detection leaves the button test available. Goodwin keeps transcripts in the current conversation and does not save recordings or diagnostic transcripts to disk.

**Settings → Microphone** lets you set separate input gains for wake word detection and speech recognition. The recognition field defaults to automatic gain. A fixed multiplier applies to the recording and its speech detection, including audio buffered before the wake word triggers. Gain changes take effect without restarting Goodwin. An active voice turn keeps its original settings.

Use **Settings → System prompt** to define Goodwin’s role, personality, and tone for both text and voice replies. Save the prompt to apply it to subsequent requests. Clear the field and save to restore the built-in instructions. Goodwin keeps the instructions for spoken output and voice plugin calls when a custom prompt is set.

## Diagnostics

System resources in Settings can download a diagnostic JSON snapshot. It contains process-local counts for full queues, wake inference failures, skipped capture frames and voice cancellations. The latency summaries report sample counts, bucket upper bounds for the median and 95th percentile, and the measured maximum. ESP32-S3 also reports write latency for NVS settings, WASM plugin data, wake model assets and OTA. The export has no recordings, conversations, credentials or journal entries.

## Environment variables

| Environment variable | Purpose | Default |
| --- | --- | --- |
| `GOODWIN_MICROPHONE` | Local `yio_audio` socket address | `127.0.0.1:9850` |
| `GOODWIN_MIC_GAIN` | Default wake word gain when the panel field is empty and input gain in automatic recognition mode | `4` |
| `GOODWIN_WAKE_MODEL` | Path to a local Standard or Micro ONNX when no catalog model is selected | `<data-dir>/goodwin.onnx` |
| `GOODWIN_APLAY` | ALSA playback program path | `aplay` |
| `GOODWIN_PLAYBACK_DEVICE` | ALSA playback device | `speakers` |

## Plugins

Sign in to the panel and use **Plugins → Upload package** to install a trusted `.tar.gz` archive or update an existing plugin. When the process reports readiness, its page, navigation and API become available. A failed update restores the previous version and its data. Removing a plugin also removes its saved data.

Enable or disable the password in **Settings → Panel**. Sessions issued after sign-in last 30 days. Updates and logs require a password. Plugin packages and recordings are available in open mode.

Saved PBKDF2 passwords continue to work. After upgrading a Mini 3 with an Argon2 panel password, stop Goodwin, run `goodwin set-password <data-dir>`, then start it again. Managed installations use `/data/opt/etc/goodwin` as the data directory.

The Windows Station build produces `target/plugins/timers-wasm32.tar.gz` and `target/plugins/timers-linux-aarch64.tar.gz`. Upload Timers to create, list and cancel timers in the panel. Updating an existing Timers installation with the WASM package retains its saved timers. For voice commands, select an answer model that supports tool calling. Panel timers and their sounds run locally.

Upload `wake-dataset-wasm32.tar.gz` to collect and label Station recordings for wake-word training. Open **Wake-word dataset** in the panel and start collection. Listen to recordings, mark whether the wake word is present, and choose **Download labelled dataset**. See the [dataset plugin guide](../plugins/wake-dataset/) for the audio format and metadata.

The [OpenAI-compatible WASM provider](../plugins/openai-compatible/) supports multiple named connections, including OpenRouter and local inference servers such as KittenTTS. Add connections in the plugin's settings and select them by name for each stage of the assistant. The assistant settings hold the models and voice. On first installation, the plugin creates an OpenRouter connection. Configure its key on the plugin settings page.

See the [plugin development guide](../plugins/README.md) for the package format, process protocol, and UI kit.
