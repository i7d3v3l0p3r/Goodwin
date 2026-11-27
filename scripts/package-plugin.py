"""Build a Goodwin plugin archive from an already compiled executable."""

import argparse
import gzip
import io
import json
import subprocess
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
    parser.add_argument("target", help="Runtime platform such as linux-aarch64, windows-x86_64 or wasm32")
    parser.add_argument("output", type=Path)
    parser.add_argument("--version", help="Override the package version for this build")
    parser.add_argument("--companion", type=Path, action="append", default=[],
                        help="Additional native package file to place under bin/")
    parser.add_argument("--wasm-state-file", help="JSON state filename in the WASM plugin data directory")
    args = parser.parse_args()
    if args.wasm_state_file and args.target != "wasm32":
        parser.error("--wasm-state-file requires the wasm32 target")

    manifest = json.loads((args.plugin / "manifest.json").read_text(encoding="utf-8"))
    if args.target == "wasm32":
        manifest.pop("nativeProxy", None)
    if args.target != "wasm32":
        manifest.pop("providerConnections", None)
        manifest.pop("wasmHttp", None)
        manifest.pop("wasmStorage", None)
    suffix = ".wasm" if args.target == "wasm32" else ".exe" if args.target.startswith("windows-") else ""
    manifest["executable"] = f"bin/{manifest['id']}{suffix}"
    manifest["target"] = args.target
    if args.wasm_state_file:
        manifest["wasmStateFile"] = args.wasm_state_file
    if args.version:
        manifest["version"] = args.version

    web = manifest.get("web")
    compiled_entry = None
    if web and web.get("solidRuntimeVersion") == 1:
        source = (args.plugin / web["entry"]).with_suffix(".tsx")
        if source.is_file():
            builder = Path(__file__).resolve().parents[1] / "web" / "build-plugin.mjs"
            command = ["node", str(builder), str(source)]
            if web.get("now"):
                command.append("--now")
            compiled_entry = subprocess.check_output(command)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            add_bytes(archive, "manifest.json", json.dumps(manifest, ensure_ascii=False).encode("utf-8"))
            add_bytes(archive, manifest["executable"], args.executable.read_bytes())
            for companion in args.companion:
                add_bytes(archive, f"bin/{companion.name}", companion.read_bytes())
            if compiled_entry is not None:
                add_bytes(archive, web["entry"], compiled_entry)
            for file in sorted((args.plugin / "ui").rglob("*")):
                if not file.is_file():
                    continue
                relative = file.relative_to(args.plugin).as_posix()
                if compiled_entry is not None and (relative == web["entry"] or file.suffix in (".ts", ".tsx")):
                    continue
                add_bytes(archive, relative, file.read_bytes())
    print(args.output)


if __name__ == "__main__":
    main()
