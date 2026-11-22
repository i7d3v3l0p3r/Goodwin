"""Build a Goodwin plugin archive from an already compiled executable."""

import argparse
import gzip
import io
import json
import tarfile
from pathlib import Path


def add_bytes(archive, name, data):
    entry = tarfile.TarInfo(name)
    entry.size = len(data)
    archive.addfile(entry, io.BytesIO(data))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plugin", type=Path, help="Directory with manifest.json and optional ui/")
    parser.add_argument("executable", type=Path)
    parser.add_argument("target", help="Runtime platform such as linux-aarch64 or windows-x86_64")
    parser.add_argument("output", type=Path)
    parser.add_argument("--version", help="Override the package version for this build")
    parser.add_argument("--probe-mode", choices=("delay", "fail", "timeout"), help="Readiness test mode for the probe")
    args = parser.parse_args()

    manifest = json.loads((args.plugin / "manifest.json").read_text(encoding="utf-8"))
    if args.probe_mode and manifest["id"] != "probe":
        parser.error("--probe-mode is only available for the probe plugin")
    suffix = ".exe" if args.target.startswith("windows-") else ""
    manifest["executable"] = f"bin/{manifest['id']}{suffix}"
    manifest["target"] = args.target
    if args.version:
        manifest["version"] = args.version

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            add_bytes(archive, "manifest.json", json.dumps(manifest, ensure_ascii=False).encode("utf-8"))
            add_bytes(archive, manifest["executable"], args.executable.read_bytes())
            if args.probe_mode:
                add_bytes(archive, "bin/probe-mode", args.probe_mode.encode("ascii"))
            for file in sorted((args.plugin / "ui").rglob("*")):
                if file.is_file():
                    add_bytes(archive, file.relative_to(args.plugin).as_posix(), file.read_bytes())
    print(args.output)


if __name__ == "__main__":
    main()
