"""Fetch the pinned public models used by the synthetic wake-runtime checks."""

import argparse
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/wake-reference")
    args = parser.parse_args()
    spec = json.loads((ROOT / "tests/fixtures/wake_reference.json").read_text(encoding="utf-8"))
    args.output.mkdir(parents=True, exist_ok=True)
    for model in spec["models"]:
        path = args.output / model["filename"]
        size = model["size_bytes"]
        if path.is_file():
            data = path.read_bytes()
        else:
            with urlopen(f"{spec['release']}/{path.name}", timeout=30) as response:
                data = response.read(size + 1)
        if len(data) != size or hashlib.sha256(data).hexdigest() != model["sha256"]:
            raise ValueError(f"Wake reference model failed integrity validation: {path}")
        if not path.is_file():
            temporary = path.with_suffix(path.suffix + ".partial")
            temporary.write_bytes(data)
            temporary.replace(path)
        print(f"Verified {path.name}")


if __name__ == "__main__":
    main()
