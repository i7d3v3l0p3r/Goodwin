# Goodwin

Goodwin is an alternative voice assistant for Yandex Station Mini 3 (`bergamot`). It runs separately from Alice on a prepared Station. Say “Гудвин” followed by a question in one breath, or send a message from the web panel.

The Station detects the wake word locally. The assistant's cascade engine transcribes the recording, generates an answer, and plays synthesized speech. In **Assistant**, select a provider and model independently for each stage. OpenRouter is selected by default. Each activation handles one voice request. Numbered timer choices carry over to the next voice request. The current web conversation lives in memory and clears when you start a new chat or stop Goodwin.

Download the desktop installer, Station package and plugins from [GitHub Releases](https://github.com/i7d3v3l0p3r/Goodwin/releases). See the [changelog](CHANGELOG.md) for release notes.

## Desktop installer

The Rust installer runs on Windows, Linux and macOS. It installs Goodwin on a prepared Yandex Station Mini 3 over network ADB with root access and support for the `/data/init.d` service directory. Install [Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools) and put ADB on your `PATH`, or select the ADB executable under **Дополнительные настройки**.

1. Select a Station from the list. The installer starts an mDNS search for Mini 3 on launch. Click **Обновить** to search again or **Указать адрес вручную** to enter an IP address or hostname. Add an ADB port if it differs from 5555.
2. Click **Проверить подключение** to check ADB access and Station compatibility. Confirm the target and package source, then choose a wake word and model tier. The default keeps the current model on updates and downloads «Гудвин» Standard on a fresh installation.
3. Click **Установить Goodwin** or **Обновить Goodwin**. The installer verifies the Station package and any model download with SHA-256, starts the service and enables it at boot.
4. After a fresh installation, click **Настроить Goodwin** and create a password. The panel guides you through the provider setup and checks for text and voice replies.

For local testing, open **Дополнительные настройки → Локальная сборка** and select an archive or a directory containing `goodwin-mini3-aarch64.tar.gz` or an extracted package. An unpackaged test build needs the AArch64 `goodwin` binary. If the directory contains `SHA256SUMS`, make sure it matches the local build. Click **Сбросить** in **Локальная сборка** to resume installing from GitHub.

Updates to the managed Station service reuse its data directory at `/data/opt/etc/goodwin` and keep its wake model selection by default, including models from older bundled releases. When there is room, the installer uploads and extracts the package in `/tmp`. If the startup check fails, it restores the previous version and restarts it if it was running.

To build and launch the desktop installer locally, run:

```sh
cargo run --locked --manifest-path installer/Cargo.toml --release
```

The installer is a separate Rust project and builds without the Goodwin web panel or ARM cross-compiler. On Linux it uses OpenGL with Wayland or X11, and the native folder picker uses the desktop portal.

### Headless installation

Pass the address of a prepared Station to install or update Goodwin without opening a window.

```sh
goodwin-installer install station.local
goodwin-installer install 192.168.1.42:5555 --package ./goodwin-mini3-aarch64.tar.gz --model egor-micro
```

The default ADB port is 5555 and the default package is the latest stable release. Use `--package <path>` to select a local archive or build directory, `--adb <path>` to select an ADB executable, and `--model <id>` to select a model ID from the latest release's [metadata.json](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/latest/download/metadata.json). The default `keep` retains the model on updates and downloads `goodwin-standard` for a fresh installation. Put IPv6 addresses in brackets, as in `[fd00::42]:5555`.

The command writes progress and errors to stderr. On success it exits with status 0 and prints the panel URL to stdout. An installation failure returns status 1 and invalid arguments return status 2. Fresh installations still ask for a password on the first panel visit.

The regular installer supports both modes. To build it without desktop dependencies, run `cargo build --locked --release --no-default-features --manifest-path installer/Cargo.toml`.

### Update from the web panel

On a Station set up with the installer, sign in and open **Station → Goodwin update**. Check for updates, then select **Install and restart** when a newer stable release is available. The Station needs HTTPS access to GitHub to download and verify the package. If the update page is missing, use the installer to install a build with web update support first.

Goodwin checks that the panel starts after the restart and restores the previous executable if startup fails. Settings, plugins and the selected wake-word model are retained. The current chat clears when Goodwin restarts. Reload the panel when it reports that the update is complete.

## Build

Goodwin also has a standalone [ESP32-S3 N16R8 build](firmware/esp32s3/README.md).
It supports local «Гудвин» activation, push-to-talk, Wi-Fi setup and firmware updates from the panel.
The Cargo target selects the platform backend. The build features
`model-mini3` and `model-esp32s3-generic` select the model and its drivers.
Each platform defaults to its supported model, so existing Mini 3 build commands
continue to work. Both builds use the same Rust voice pipeline, panel contract,
and Solid panel assets. `/api/capabilities` describes the selected model to the browser.

Use Rust, Node.js 22.12+ or 24 LTS, and npm.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked --release
```

The panel is compressed with gzip during the build and embedded in the binary alongside the Russian offline error announcement. Building the Station's INT8 wake-word runtime requires Git, Python 3, CMake 3.23 or newer, Ninja, and Zig. With `wasm-plugins`, Cargo fetches and builds the pinned WAMR Fast interpreter. This also requires Git, CMake 3.23 or newer, and Ninja. Use Zig when cross-compiling for the Station. On Windows, the Station build script also builds and packages the WASM and native plugins.

```powershell
rustup target add aarch64-unknown-linux-musl wasm32-unknown-unknown
.\scripts\build-station.ps1
```

On Linux, use [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) with Zig.

```sh
rustup target add aarch64-unknown-linux-musl wasm32-unknown-unknown
python3 scripts/build-wake-tflite.py --target aarch64-musl
python3 scripts/build-wasm-plugins.py
cargo zigbuild --locked --release --features wake-tflite,wasm-plugins --target aarch64-unknown-linux-musl
```

The Station binary is `target/aarch64-unknown-linux-musl/release/goodwin`. The native binary runs the panel on a development computer. The live microphone and playback tests require a Station or an audio test fixture.

Run `python scripts/package-station.py` to package the ARM64 binary and launcher in `target/goodwin-mini3-aarch64.tar.gz`. Model assets and their license and notice are distributed in the [Goodwin-wakeword releases](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases).

## Publish a release

Set the release version in the Goodwin and installer Cargo manifests and lockfiles. Add an entry headed `## [<version>] - YYYY-MM-DD` to `CHANGELOG.md` with English and Russian notes. Push the commit, then push a `v<version>` tag on it. GitHub Actions builds the ARM64 Station package, plugin archives and desktop installers, runs the tests, and publishes the downloads and `SHA256SUMS` with the matching changelog entry.

To start the workflow manually for an existing release tag, run `gh workflow run release.yml --ref v<version>`.

## Run on the Station

Copy the Linux binary to a writable, executable directory on a prepared Mini 3. Leave the Station's existing voice services running. Goodwin connects to `yio_audio` at `127.0.0.1:9850` and uses `aplay -D speakers`. Its configuration and model go in a separate data directory.

```sh
chmod +x ./goodwin
./goodwin setup ./goodwin-data
./goodwin serve ./goodwin-data 0.0.0.0:8787
```

Open the Station's address on port 8787 and create a panel password. In **Set up Goodwin**, upload `openai-compatible-wasm32.tar.gz`, configure OpenRouter or another compatible API, and check text and voice replies. Keep the panel on a trusted network. It uses plain HTTP, and the first visitor who completes setup sets the password. For explicit password-free access, run `goodwin setup ./goodwin-data open` when initializing a new data directory.

To enable or change voice activation, open **Station → Activation and microphone**. Choose a model and click **Download and switch**. The Station needs access to GitHub for the download. The new model takes effect without restarting the service and is saved for subsequent starts.

On a manual installation, you can also place a local LiveKit v2 Standard or Micro ONNX at `goodwin-data/goodwin.onnx` before starting Goodwin. Goodwin reads the managed selection first and uses the local model when no catalog model is selected. Set `GOODWIN_WAKE_MODEL` to use a different local file. Run `goodwin wake-runtimes` to list the runtimes compiled into the binary. Use `goodwin wake-check <model.onnx>` or `goodwin wake-check <model.tflite> <metadata.json>` to check loading, warm-up, and one inference step without the microphone. The Station build supports Micro INT8 TFLite and uses the catalog threshold at 50% sensitivity.

On the Station, extract the package with `gzip -dc goodwin-mini3-aarch64.tar.gz | tar -xf -`. In the extracted directory, run `./start.sh` after initializing the data directory. Use `GOODWIN_PANEL_ADDRESS=0.0.0.0:8787` to open the panel to your trusted network and `GOODWIN_DATA_DIR` to reuse an existing data directory.

The text chat and push-button Station test work without a wake model. In Settings you can pause background detection, change sensitivity, test the speaker and microphone, and see timings for the last request. Pausing detection leaves the button test available. Goodwin keeps transcripts in the current conversation and does not save recordings or diagnostic transcripts to disk.

**Settings → Microphone** lets you set separate input gains for wake word detection and speech recognition. The recognition field defaults to automatic gain. A fixed multiplier applies to the recording and its speech detection, including audio buffered before the wake word triggers. Gain changes take effect without restarting Goodwin. An active voice turn keeps its original settings.

Use **Settings → System prompt** to define Goodwin’s role, personality, and tone for both text and voice replies. Save the prompt to apply it to subsequent requests. Clear the field and save to restore the built-in instructions. Goodwin keeps the instructions for spoken output and voice plugin calls when a custom prompt is set.

On a development computer, use the default loopback address.

```sh
goodwin setup goodwin-data
goodwin serve goodwin-data
```

The panel opens at `http://127.0.0.1:8787`. When running Vite in development mode, keep Goodwin on that address for the API proxy.

| Environment variable | Purpose | Default |
| --- | --- | --- |
| `GOODWIN_MICROPHONE` | Local `yio_audio` socket address | `127.0.0.1:9850` |
| `GOODWIN_MIC_GAIN` | Default wake word gain when the panel field is empty and input gain in automatic recognition mode | `4` |
| `GOODWIN_WAKE_MODEL` | Path to a local Standard or Micro ONNX when no catalog model is selected | `<data-dir>/goodwin.onnx` |
| `GOODWIN_APLAY` | ALSA playback program path | `aplay` |
| `GOODWIN_PLAYBACK_DEVICE` | ALSA playback device | `speakers` |

## Plugins

Sign in to the panel and use **Plugins → Upload package** to install a trusted `.tar.gz` archive or update an existing plugin. When the process reports readiness, its page, navigation and API become available. A failed update restores the previous version and its data. Removing a plugin also removes its saved data.

If you previously set up open access, stop Goodwin, run `goodwin set-password <data-dir>`, then start it again. Your saved settings and API key are retained.

After upgrading a Mini 3 with an Argon2 or PBKDF2-SHA512 panel password, use the same command to set a password before signing in. Managed installations use `/data/opt/etc/goodwin` as the data directory.

The Windows Station build produces `target/plugins/timers-wasm32.tar.gz` and `target/plugins/timers-linux-aarch64.tar.gz`. Upload Timers to create, list and cancel timers in the panel. Updating an existing Timers installation with the WASM package retains its saved timers. For voice commands, select an answer model that supports tool calling. Panel timers and their sounds run locally.

Upload `wake-dataset-wasm32.tar.gz` to collect and label Station recordings for wake-word training. Open **Wake-word dataset** in the password-protected panel and start collection. Listen to recordings, mark whether the wake word is present, and choose **Download labelled dataset**. See the [dataset plugin guide](plugins/wake-dataset/) for the audio format and metadata.

The [OpenAI-compatible WASM provider](plugins/openai-compatible/) supports multiple named connections, including OpenRouter and local inference servers such as KittenTTS. Add connections in the plugin's settings and select them by name for each stage of the assistant. The assistant settings hold the models and voice. On first installation, the plugin creates an OpenRouter connection. Configure its key on the plugin settings page.

See the [plugin development guide](plugins/README.md) for the package format, process protocol, and UI kit.

## Train the Russian wake word

Goodwin supports streaming **LiveKit WakeWord v2 Standard and Micro** ONNX models. Train and evaluate the Russian phrase using [LiveKit's trainer](https://github.com/livekit/livekit-wakeword/blob/v2.1.3/client/python/TRAINING.md) on a CUDA computer or an Apple silicon Mac. The stock trainer generates speech through LiveKit Inference and requires LiveKit Cloud credentials for that step.

```sh
uv tool install --python 3.13 "livekit-wakeword[train]==2.1.3"
livekit-wakeword setup --wakeword "Гудвин" --language ru --eval \
  --hard-negative "Гарвин" --hard-negative "Гудман" --hard-negative "будни"
livekit-wakeword train --wakeword "Гудвин" --type standard --out ./models
livekit-wakeword eval --wakeword "Гудвин"
```

Copy the exported ONNX file to `goodwin-data/goodwin.onnx`. Check activation with real recordings from the Station, including a question spoken immediately after the name, a name without a question, music, background conversation, and requests to Alice.

### Fine-tune on a local dataset

The scripts below use the WAV files and split labels in `datasets/goodwin/manifest.jsonl`. They use LiveKit's pretrained Standard encoder, streaming trainer, and ONNX exporter. All augmentation audio comes from the training split, and only the validation split is used for checkpoint and threshold selection.

Extract the [v2.1.3 source archive](https://github.com/livekit/livekit-wakeword/archive/refs/tags/v2.1.3.zip) under `target/wakeword-v2/`. Download the [official encoder archive](https://github.com/livekit/livekit-wakeword/releases/download/v2.0.0/encoders.tar.gz), check the SHA-256 against `ENCODERS_RELEASE` in LiveKit's `encoders.py`, then extract its `encoders/` at the same location. Run these commands from the repository root in a Python 3.12 virtual environment.

```powershell
git apply --directory=target/wakeword-v2/livekit-wakeword-2.1.3 scripts/livekit-wakeword-cpu.patch
python -m pip install --index-url https://download.pytorch.org/whl/cpu torch==2.10.0+cpu torchaudio==2.10.0+cpu
python -m pip install --no-deps ./target/wakeword-v2/livekit-wakeword-2.1.3
python -m pip install numpy pydantic pyyaml librosa safetensors onnx onnxruntime matplotlib
$out = "datasets/goodwin/training/livekit-standard-v2"
python scripts/wakeword_train_livekit.py --encoders target/wakeword-v2/encoders --out $out
python scripts/wakeword_plot_livekit.py --run $out --training-only
python scripts/wakeword_eval_livekit.py --encoders target/wakeword-v2/encoders --out $out
python scripts/wakeword_plot_livekit.py --run $out
```

The export is `goodwin.onnx`. The evaluator streams audio through the LiveKit runtime and writes `evaluation.json`, per-clip predictions, and score traces. The threshold budget is 1% of negative validation clips, and the exporter maps the selected score to 0.50. The evaluation reports false alarms per clip and per hour of negative test audio. It counts the first activation in the annotated wake-word window as a hit.

For another wake word, pass its corpus with `--dataset` to both scripts, its text with `--phrase` to the trainer, and the export file stem with `--name` to the evaluator. Use a separate `--out` directory for each run.

Use the selected Standard checkpoint as the teacher for the smaller Micro model. The Micro ONNX export uses `onnxsim` to fold padding into constants for the Goodwin runtime.

```powershell
python -m pip install onnxsim==0.8.1
$micro = "datasets/goodwin/training/livekit-micro-v2"
python scripts/wakeword_train_livekit.py --tier micro --teacher "$out/selected.pt" --encoders target/wakeword-v2/encoders --out $micro
python scripts/wakeword_eval_livekit.py --tier micro --encoders target/wakeword-v2/encoders --out $micro
python scripts/wakeword_plot_livekit.py --run $micro
```

The Micro ONNX takes log-mel frames and recurrent state. Use LiveKit's `MicroRuntime` for Python evaluation. Goodwin computes the Micro frontend from PCM in Rust and reads the normalization parameters embedded in the ONNX.

For the INT8 export, install LiveKit's conversion dependencies. The converter uses audio from the local training split to calibrate quantization ranges. The script selects the INT8 trigger threshold on validation and reports its test results separately.

```powershell
python -m pip install tensorflow==2.21.0 tf-keras==2.21.0 onnx2tf==1.28.8 onnxsim==0.8.1 ai-edge-litert==2.3.0 onnx-graphsurgeon==0.6.1 sng4onnx==2.0.1 psutil
python scripts/wakeword_quantize_livekit.py --encoders target/wakeword-v2/encoders --out $micro
```

ESP32 firmware can run the wake word locally with the LiveKit ESP-IDF component. Embed `goodwin-micro-int8.tflite` in flash and use its catalog `score_threshold`. Call `lkww_process()` from an audio processing task with 16 kHz mono PCM to avoid the separate task and one-second audio buffer used by `lkww_feed()`. Check the arena allocation and `lkww_last_step_us()` with Wi-Fi and audio active on the target board. Each 1,280-sample inference step adds 80 ms of audio. [LiveKit ESP-IDF component](https://github.com/livekit/livekit-wakeword/tree/v2.1.3/client/esp32) · [Goodwin-wakeword releases](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/latest)

## Check the voice pipeline with a recording

Use a data directory with configured provider plugins and supply a Russian WAV recording. Stop the Goodwin service before running the CLI against its data directory.

```sh
goodwin wav question.wav answer.wav goodwin-data
```

This uses the same provider assignments and plugin settings as the panel and saves the spoken answer as WAV. If you omit the data directory, Goodwin uses `GOODWIN_DATA_DIR` or `goodwin-data`.

## Local verification

### Profile Rust allocations

Build with `--features hotpath-alloc` to collect allocation counts and bytes by function and thread. On Windows, `.\scripts\build-station.ps1 -ProfileAllocations` enables the feature in the Station build. For the Linux Station build, use `--features wake-tflite,wasm-plugins,hotpath-alloc`.

Run a short profile and stop Goodwin normally to write the report.

```sh
HOTPATH_METRICS_SERVER_OFF=1 HOTPATH_OUTPUT_FORMAT=json-pretty \
  HOTPATH_OUTPUT_PATH=goodwin-profile.json ./goodwin serve ./goodwin-data
```

For live monitoring, leave the metrics server enabled and run `hotpath console`. The server listens on loopback port 6770. For a Station connected through ADB, forward that port with `adb forward tcp:6770 tcp:6770`. Compare allocation traffic with Linux RSS or PSS to account for native TFLite allocations, mapped executable pages, and allocator overhead. [Hotpath memory profiling](https://hotpath.rs/memory_profiling)

### Run checks

```sh
npm run build --prefix web
cargo test --locked --workspace
```

The tests emulate a Station microphone and provider transport on loopback and capture the generated audio without playing it.

The panel embeds [Inter](https://rsms.me/inter/) under the [SIL Open Font License](web/Inter-OFL.txt).
