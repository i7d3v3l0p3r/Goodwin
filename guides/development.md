# Build and develop Goodwin

Run the commands below from the repository root.

## Run the panel on your computer

Install Rust, Node.js 22.12+ or 24 LTS, and npm.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked --release
```

The panel is compressed with gzip during the build and embedded in the binary alongside the Russian offline error announcement.

Start Goodwin on the loopback address.

```sh
cargo run --locked --release -- setup goodwin-data
cargo run --locked --release -- serve goodwin-data
```

The panel opens at `http://127.0.0.1:8787`. To edit the panel with live reload, leave Goodwin running there and start `npm run dev --prefix web` in another terminal.

To try a text conversation, set a panel password and sign in. Install a provider package for your host and configure a connection in the panel. The [OpenAI-compatible provider guide](../plugins/openai-compatible/README.md) includes packaging instructions for a native Windows build. The [plugin SDK guide](../plugins/README.md#wasm-plugins) covers WASM support on Linux.

## Build targets

Goodwin also has a standalone [ESP32-S3 N16R8 build](../firmware/esp32s3/README.md).
It supports local «Гудвин» activation, push-to-talk, Wi-Fi setup and firmware updates from the panel.
The Cargo target selects the platform backend. The build features
`model-mini3` and `model-esp32s3-generic` select the model and its drivers.
Each platform defaults to its supported model, so existing Mini 3 build commands
continue to work. Both builds use the same Rust voice pipeline, panel contract,
and Solid panel assets. `/api/capabilities` describes the selected model to the browser.

| Build | Target | Wake runtimes | Providers and add-ons | Audio and panel |
| --- | --- | --- | --- | --- |
| Mini 3 release | `aarch64-unknown-linux-musl` with `wake-tflite,wasm-plugins` | Standard and Micro ONNX, Micro INT8 TFLite | Native ARM64 and WASM | YIO microphone, ALSA playback, logo and clock indicators, panel events |
| Mini 3 without TFLite | Same target, omit `wake-tflite` | Standard and Micro ONNX | Native ARM64, WASM with `wasm-plugins` | Same Station hardware and panel |
| Mini 3 INT8-only | Same target, `--no-default-features --features wake-tflite,wasm-plugins` | Micro INT8 TFLite | Native ARM64 and WASM | Same Station hardware and panel |
| Native development | Linux x86-64 and Windows builds | Standard and Micro ONNX by default, optional Micro INT8 on Linux x86-64 | WASM is supported on Linux with `wasm-plugins` | Panel and test fixtures for Station audio |
| ESP32-S3 N16R8 | `xtensa-esp32s3-espidf`, default firmware script | Bundled microWakeWord and LiveKit Micro INT8 catalog | WASM or built-in providers | I²S, push-to-talk, Wi-Fi setup, panel polling, OTA |
| ESP32-S3 button-only | Same target with `--button-only` | None | WASM or built-in providers | Same ESP hardware and panel, activation by button |
| ESP32-S3 without plug-ins | Same target with `--no-plugins`, optionally with `--button-only` | microWakeWord and LiveKit Micro INT8 with the default firmware, none with `--button-only` | Built-in provider | Same ESP hardware and panel |

Use `--emulator` with the ESP32-S3 build script for ESP-EMU. Its local
provider fixture and OTA image tag are specific to the emulator.
The [desktop installer](mini-3.md#desktop-installer) is built separately and deploys
the Mini 3 package.

## Build for Mini 3

Building the Station's INT8 wake-word runtime requires Git, Python 3, CMake 3.23 or newer, Ninja, and Zig. With `wasm-plugins`, Cargo fetches and builds the pinned WAMR Fast interpreter. This also requires Git, CMake 3.23 or newer, and Ninja. Use Zig when cross-compiling for the Station. On Windows, the Station build script also builds and packages the WASM and native plugins.

```powershell
rustup target add aarch64-unknown-linux-musl wasm32-unknown-unknown
.\scripts\build-station.ps1
```

The Windows script reuses installed panel dependencies while the Node.js version, platform, and npm lockfiles match. It also reuses the built panel while its inputs and generated files match. Pass `-CleanWebDependencies` to reinstall the dependencies and rebuild the panel.

On Linux, use [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) with Zig.

```sh
rustup target add aarch64-unknown-linux-musl wasm32-unknown-unknown
python3 scripts/build-wake-tflite.py --target aarch64-musl
python3 scripts/build-wasm-plugins.py
cargo zigbuild --locked --release --features wake-tflite,wasm-plugins --target aarch64-unknown-linux-musl
```

The Station binary is `target/aarch64-unknown-linux-musl/release/goodwin`. The native binary runs the panel on a development computer. The live microphone and playback tests require a Station or an audio test fixture.

Run `python scripts/package-station.py` to package the ARM64 binary and launcher in `target/goodwin-mini3-aarch64.tar.gz`. Model assets and their license and notice are distributed in the [Goodwin-wakeword releases](https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases).

See the [native runtime notes](../native/README.md) for dependency pins, FFI ownership, and checks with AddressSanitizer and UndefinedBehaviorSanitizer.

### Build the Mini 3 INT8-only package

After building the panel and native runtime prerequisites above, build and package the INT8-only binary with cargo-zigbuild.

```sh
cargo zigbuild --locked --release --no-default-features --features wake-tflite,wasm-plugins \
  --target aarch64-unknown-linux-musl --target-dir target/mini3-int8 \
  --config 'profile.release.package.goodwin.strip="symbols"'
python3 scripts/package-station.py \
  --binary target/mini3-int8/aarch64-unknown-linux-musl/release/goodwin \
  --output target/goodwin-mini3-int8-aarch64.tar.gz
```

The installer defaults to `goodwin-micro-int8` for a fresh installation from this local package. For an existing installation, switch to a Micro INT8 model in the panel before upgrading, or pass `--model goodwin-micro-int8` to the installer with the INT8-only package. A Station with an ONNX model still selected will reject the upgrade during its preflight check.

## Build the desktop installer

Build and launch the desktop installer from the repository root.

```sh
cargo run --locked --manifest-path installer/Cargo.toml --release
```

The installer is a separate Rust project and builds without the Goodwin web panel or ARM cross-compiler. On Linux it uses OpenGL with Wayland or X11, and the native folder picker uses the desktop portal.

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

On Linux x86-64, build the optional wake runtime and WASM fixtures to test both against the native host.

```sh
python scripts/build-wake-tflite.py
python scripts/build-wasm-plugins.py
cargo test --locked --workspace --features goodwin/wake-tflite,goodwin/wasm-plugins
```

The panel embeds [Inter](https://rsms.me/inter/) under the [SIL Open Font License](../web/Inter-OFL.txt).

## Publish a release

Set the release version in the Goodwin and installer Cargo manifests and lockfiles. Add an entry headed `## [<version>] - YYYY-MM-DD` to `CHANGELOG.md` with English and Russian notes. Push the commit, then push a `v<version>` tag on it. GitHub Actions builds the ARM64 Station package, WASM plugin archives and desktop installers, runs the tests, and publishes the downloads and `SHA256SUMS` with the matching changelog entry.

To start the workflow manually for an existing release tag, run `gh workflow run release.yml --ref v<version>`.
