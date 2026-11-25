"""Package the Linux ARM64 Goodwin build."""

import argparse
import hashlib
import io
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PREFIX = "goodwin-mini3"
GUIDE = """# Goodwin on Yandex Station Mini 3

Extract the archive into a writable executable directory on a prepared Station.
Mini 3 firmware with BusyBox tar uses a separate gzip command.

```sh
gzip -dc goodwin-mini3-aarch64.tar.gz | tar -xf -
cd goodwin-mini3
sha256sum -c SHA256SUMS
./goodwin setup ./goodwin-data
GOODWIN_PANEL_ADDRESS=0.0.0.0:8787 ./start.sh
```

Open the Station's address on port 8787 to create the panel password and sign in.
The first visitor who completes setup sets the password.
Install a provider package in the Plugins panel to enable inference.
Download a wake word model in Settings → Activation and microphone.

For an existing Goodwin installation, reuse its configured data directory with
`GOODWIN_DATA_DIR` and skip setup. Stop the current Goodwin process before
starting the new one.
Use a trusted network for the HTTP panel.
"""


def add_bytes(archive: tarfile.TarFile, name: str, data: bytes, mode: int = 0o644) -> None:
    info = tarfile.TarInfo(f"{PREFIX}/{name}")
    info.size = len(data)
    info.mode = mode
    archive.addfile(info, io.BytesIO(data))


def licenses(binary: bytes) -> bytes:
    license_text = (ROOT / "LICENSE").read_bytes()
    if b"goodwin.plugins.wamr.v1" in binary:
        license_text += b"\n\nWAMR\n\n" + (ROOT / "licenses/wamr-apache-2.0.txt").read_bytes()
    if b"goodwin.plugins.wasmi.v1" in binary:
        license_text += b"\n\nWasmi\n\n" + (ROOT / "licenses/wasmi-mit.txt").read_bytes()
    marker = (ROOT / "shared/wake-tflite-capability.txt").read_bytes().strip()
    if marker not in binary:
        return license_text
    sources = ROOT / "target/wake-tflite"
    for title, directory in (
        ("TensorFlow Lite Micro", sources / "tflite-micro"),
        ("LiveKit WakeWord", sources / "livekit-wakeword"),
        ("FlatBuffers", sources / "deps/flatbuffers"),
    ):
        license_text += b"\n\n" + title.encode() + b"\n\n" + (directory / "LICENSE").read_bytes()
        notice = directory / "NOTICE"
        if notice.is_file():
            license_text += b"\n" + notice.read_bytes()
    return license_text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/aarch64-unknown-linux-musl/release/goodwin")
    parser.add_argument("--output", type=Path, default=ROOT / "target/goodwin-mini3-aarch64.tar.gz")
    args = parser.parse_args()
    binary = args.binary.read_bytes()
    if binary[:6] != b"\x7fELF\x02\x01" or binary[18:20] != b"\xb7\x00":
        parser.error("The Goodwin binary must be little-endian AArch64 ELF")

    files = {
        "goodwin": (binary, 0o755),
        "start.sh": ((ROOT / "scripts/start-station.sh").read_text().encode(), 0o755),
        "LICENSE": (licenses(binary), 0o644),
        "README.md": (GUIDE.encode(), 0o644),
    }
    checksums = "".join(f"{hashlib.sha256(data).hexdigest()}  {name}\n"
                        for name, (data, _) in files.items())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(args.output, "w:gz", format=tarfile.GNU_FORMAT) as archive:
        for name, (data, mode) in files.items():
            add_bytes(archive, name, data, mode)
        add_bytes(archive, "SHA256SUMS", checksums.encode())
    print(args.output.resolve())


if __name__ == "__main__":
    main()
