"""Build a thin Goodwin VPN package and its separately downloadable workers."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import urlsplit


WORKERS = ("vless-stream", "vless-http2", "vless-xtls")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("arch", choices=("x86_64", "aarch64"))
    parser.add_argument("output", type=Path)
    parser.add_argument("--engines-dir", type=Path)
    parser.add_argument("--base-url", help="URL serving the three .gz files. Defaults to the tagged Goodwin release.")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    base = (args.base_url or f"https://github.com/i7d3v3l0p3r/Goodwin/releases/download/v{version}").rstrip("/")
    source = urlsplit(base)
    if source.scheme not in ("http", "https") or not source.hostname or source.username or source.password or source.query or source.fragment:
        parser.error("--base-url must be an HTTP(S) directory URL")
    directory = args.engines_dir or args.executable.parent
    args.output.parent.mkdir(parents=True, exist_ok=True)
    workers = {}
    for name in WORKERS:
        binary = (directory / name).read_bytes()
        packed = gzip.compress(binary, compresslevel=9, mtime=0)
        (args.output.parent / f"{name}-linux-{args.arch}.gz").write_bytes(packed)
        workers[name] = {"size": len(binary), "gzip_size": len(packed), "sha256": hashlib.sha256(binary).hexdigest()}
    with tempfile.TemporaryDirectory() as staging:
        catalog = Path(staging) / "engines.json"
        catalog.write_text(json.dumps({"arch": args.arch, "base_url": base, "workers": workers}), encoding="utf-8")
        companions = [catalog, args.executable.with_name("vpn-fetch"),
                      *(root / "plugins/vpn/engines" / name for name in ("notice.txt", "mpl-2.0.txt"))]
        subprocess.run([sys.executable, str(root / "scripts/package-plugin.py"), str(root / "plugins/vpn"),
                        str(args.executable), f"linux-{args.arch}", str(args.output),
                        *(arg for file in companions for arg in ("--companion", str(file)))], check=True)


if __name__ == "__main__":
    main()
