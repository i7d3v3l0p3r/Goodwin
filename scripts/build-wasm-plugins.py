"""Build the packaged WASM plugins and the host's WASM test fixture."""

import argparse
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    fixture_dir = ROOT / "target/wasm-plugins"
    fixture_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["zig", "cc", "-target", "wasm32-freestanding", "-Oz", "-ffreestanding", "-nostdlib",
         "-Wl,--no-entry", "-Wl,--strip-all", "-Wl,-z,stack-size=16384", "-Wl,--max-memory=1048576",
         ROOT / "src/plugins/process/fixtures/limits.c", "-o", fixture_dir / "limits.wasm"],
        check=True,
    )
    for plugin, stack, memory in (
        ("timers", 64 * 1024, 512 * 1024),
        ("openai-compatible", 256 * 1024, 64 * 1024 * 1024),
        ("openrouter", 256 * 1024, 64 * 1024 * 1024),
        ("wake-dataset", 256 * 1024, 8 * 1024 * 1024),
    ):
        subprocess.run(
            ["cargo", "rustc", "--locked", "-p", f"goodwin-{plugin}", "--bin", f"goodwin-{plugin}",
             "--release", "--target", "wasm32-unknown-unknown",
             "--config", 'profile.release.lto="fat"', "--config", "profile.release.codegen-units=1",
             "--jobs", str(args.jobs), "--", "-C", "opt-level=z", "-C", "strip=symbols",
             "-C", f"link-arg=-zstack-size={stack}", "-C", f"link-arg=--max-memory={memory}"],
            cwd=ROOT,
            check=True,
        )


if __name__ == "__main__":
    main()
