"""Build and package Goodwin for the ESP32-S3 N16R8 reference board."""

import argparse
import hashlib
import json
import shutil
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "xtensa-esp32s3-espidf"
PROFILE = "esp-release"
CARGO = ["cargo", "+esp"]
ESPTOOL = [sys.executable, "-m", "esptool", "--chip", "esp32s3"]


def run(args):
    subprocess.run(args, cwd=ROOT, check=True)


def app_slot(table):
    for magic, kind, subtype, offset, size, _, _ in struct.iter_unpack("<HBBII16sI", table.read_bytes()):
        if magic == 0x50AA and kind == 0 and subtype == 0x10:
            return offset, size
    raise RuntimeError("ESP-IDF partition table has no OTA 0 application slot")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emulator", action="store_true", help="Enable the emulator's local provider fixture")
    parser.add_argument("--button-only", action="store_true", help="Omit the INT8 wake-word runtime and model")
    parser.add_argument("--no-plugins", action="store_true", help="Omit the WASM plugin runtime")
    parser.add_argument("--output", type=Path, help="Output directory for the flash images")
    args = parser.parse_args()
    variant = "emulator" if args.emulator else "production"
    if args.button_only:
        variant += "-button-only"
    if args.no_plugins:
        variant += "-no-plugins"
    output = args.output.resolve() if args.output else ROOT / "target/esp32s3" / variant
    output.mkdir(parents=True, exist_ok=True)

    features = ["model-esp32s3-generic"]
    if not args.no_plugins:
        features.append("wasm-plugins")
    if not args.button_only:
        run([sys.executable, "scripts/build-wake-tflite.py", "--target", "esp32s3"])
        features.append("wake-tflite")
    if args.emulator:
        features.append("esp-emulator")
    run([*CARGO, "build", "--locked", "--profile", PROFILE, "--target", TARGET, "-Zbuild-std=std,panic_abort",
         "--features", ",".join(features)])
    metadata = subprocess.check_output([*CARGO, "metadata", "--locked", "--format-version=1", "--no-deps"], cwd=ROOT)
    build = Path(json.loads(metadata)["target_directory"]) / TARGET / PROFILE

    bootloader = output / "bootloader.bin"
    table = output / "partition-table.bin"
    app = output / "app.bin"
    merged = output / "merged.bin"
    shutil.copyfile(build / bootloader.name, bootloader)
    shutil.copyfile(build / table.name, table)
    offset, size = app_slot(table)
    flash_options = ["--flash_mode", "dio", "--flash_size", "16MB"]
    run([*ESPTOOL, "elf2image", *flash_options, "-o", app, build / "goodwin"])
    if app.stat().st_size > size:
        raise RuntimeError(f"Application ({app.stat().st_size} bytes) exceeds the {size}-byte OTA slot")
    run([*ESPTOOL, "merge_bin", *flash_options, "-o", merged, "0x0", bootloader, "0x8000", table, hex(offset), app])

    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE")
    if not args.no_plugins:
        licenses = output / "licenses/wamr"
        licenses.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / "licenses/wamr-apache-2.0.txt", licenses / "LICENSE")
    if not args.button_only:
        licenses = output / "licenses/goodwin-wakeword"
        licenses.mkdir(parents=True, exist_ok=True)
        for name in ("LICENSE", "NOTICE"):
            shutil.copyfile(ROOT / "target/wake-tflite/esp32s3" / name, licenses / name)
    images = (bootloader, table, app, merged)
    (output / "SHA256SUMS").write_text(
        "".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in images),
        encoding="ascii",
    )
    print(f"{variant}: {app.stat().st_size} bytes at {offset:#x} ({size} bytes available), {output}")


if __name__ == "__main__":
    main()
