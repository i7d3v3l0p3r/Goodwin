"""Package a Linux ARM64 Goodwin build and the two trained ONNX models."""

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
./goodwin wake-check models/standard.onnx
./goodwin wake-check models/micro.onnx
./goodwin setup ./goodwin-data
GOODWIN_PANEL_ADDRESS=0.0.0.0:8787 ./start.sh standard
```

Open the Station's address on port 8787 to create the panel password and sign in.
The first visitor who completes setup sets the password.
Install a provider package in the Plugins panel to enable inference.

For an existing Goodwin installation, reuse its configured data directory with
`GOODWIN_DATA_DIR` and skip setup. Stop the current Goodwin process before
starting the new one. Pass `micro` instead of `standard` to start with Micro.
Use a trusted network for the HTTP panel.

Both bundled models were calibrated to an ONNX score threshold of 0.50.
Set panel sensitivity to about 64% to use that threshold.
"""


def add_bytes(archive: tarfile.TarFile, name: str, data: bytes, mode: int = 0o644) -> None:
    info = tarfile.TarInfo(f"{PREFIX}/{name}")
    info.size = len(data)
    info.mode = mode
    archive.addfile(info, io.BytesIO(data))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    training = ROOT / "datasets/goodwin/training"
    parser.add_argument("--binary", type=Path, default=ROOT / "target/aarch64-unknown-linux-musl/release/goodwin")
    parser.add_argument("--standard", type=Path, default=training / "livekit-standard-v2/goodwin.onnx")
    parser.add_argument("--micro", type=Path, default=training / "livekit-micro-v2/goodwin_micro.onnx")
    parser.add_argument("--encoders", type=Path, default=ROOT / "target/wakeword-v2/encoders",
                        help="Extracted LiveKit encoder directory containing LICENSE and NOTICE")
    parser.add_argument("--output", type=Path, default=ROOT / "target/goodwin-mini3-aarch64.tar.gz")
    args = parser.parse_args()
    binary = args.binary.read_bytes()
    if binary[:6] != b"\x7fELF\x02\x01" or binary[18:20] != b"\xb7\x00":
        parser.error("The Goodwin binary must be little-endian AArch64 ELF")

    try:
        model_license = (args.encoders / "LICENSE").read_bytes()
        model_notice = (args.encoders / "NOTICE").read_bytes()
    except OSError as error:
        parser.error(f"Could not read model attribution from {args.encoders}: {error}")

    files = {
        "goodwin": (binary, 0o755),
        "start.sh": ((ROOT / "scripts/start-station.sh").read_text().encode(), 0o755),
        "models/standard.onnx": (args.standard.read_bytes(), 0o644),
        "models/micro.onnx": (args.micro.read_bytes(), 0o644),
        "LICENSE": ((ROOT / "LICENSE").read_bytes(), 0o644),
        "models/LICENSE": (model_license, 0o644),
        "models/NOTICE": (model_notice, 0o644),
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
