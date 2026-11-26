#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
python3 scripts/build-wake-tflite.py --sanitize

export GOODWIN_NATIVE_SANITIZERS=1
export ASAN_OPTIONS=detect_leaks=1
export UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1
export GOODWIN_TFLITE_MODEL=target/wake-tflite/native-sanitized/goodwin-micro-int8.tflite

features=wake-tflite,wasm-plugins
cargo test --locked -p goodwin --features "$features"
cargo test --locked -p goodwin --features "$features" tflite_reset_restores -- --ignored
