"""Build the pinned TensorFlow Lite Micro backend for Goodwin."""

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BRIDGE = ROOT / "native/wake-tflite"
SOURCES = {
    "tflite-micro": ("tensorflow/tflite-micro", "38a8f0f87302817ddfa99e6a995c7b71ef7118fe"),
    "livekit-wakeword": ("livekit/livekit-wakeword", "a7f6d9040793ee195ec85aaa8884db197a2cde0c"),
}


def run(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)


def checkout(name, dest):
    repo, commit = SOURCES[name]
    if not (dest / ".git").is_dir():
        dest.mkdir(parents=True, exist_ok=True)
        run("git", "init", "-q", dest)
    current = subprocess.run(
        ("git", "rev-parse", "HEAD"), cwd=dest, capture_output=True, text=True,
    ).stdout.strip()
    if current != commit:
        run("git", "fetch", "--depth", "1", f"https://github.com/{repo}.git", commit, cwd=dest)
        run("git", "-c", "advice.detachedHead=false", "checkout", "--detach", "FETCH_HEAD", cwd=dest)


def prepare_frontend(source, generated):
    source /= "client/esp32/livekit_wakeword/src"
    generated.mkdir(exist_ok=True)
    for name in ("lkww_frontend.c", "lkww_micro.h", "lkww_tables.h"):
        shutil.copyfile(source / name, generated / name)
    header = (source / "lkww_tflm.h").read_text(encoding="utf-8")
    destructor = "~LkwwDetector() { free(temp_); }"
    assert header.count(destructor) == 1
    header = header.replace(
        destructor,
        "~LkwwDetector() { free(temp_); free(warm_state_); }\n\n"
        + (BRIDGE / "warm-state.inc").read_text(encoding="utf-8"),
    )
    field = "uint8_t* temp_ = nullptr;"
    assert header.count(field) == 1
    header = header.replace(field, field + "\n  uint8_t* warm_state_ = nullptr;")
    (generated / "lkww_tflm.h").write_text(header, encoding="utf-8")


def zig_toolchain(dest):
    zig = shutil.which("zig")
    if not zig:
        raise RuntimeError("Install Zig to cross-compile the wake-word runtime")
    zig = Path(zig).as_posix()
    path = dest / "aarch64-musl.cmake"
    path.write_text(
        fr'''set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER "{zig}" cc -target aarch64-linux-musl)
set(CMAKE_CXX_COMPILER "{zig}" c++ -target aarch64-linux-musl)
set(CMAKE_C_COMPILER_WORKS TRUE)
set(CMAKE_CXX_COMPILER_WORKS TRUE)
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_C_ARCHIVE_CREATE "\"{zig}\" ar rcs <TARGET> <OBJECTS>")
set(CMAKE_CXX_ARCHIVE_CREATE "\"{zig}\" ar rcs <TARGET> <OBJECTS>")
set(CMAKE_C_ARCHIVE_FINISH "")
set(CMAKE_CXX_ARCHIVE_FINISH "")
''',
        encoding="utf-8",
    )
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["native", "aarch64-musl"], default="native")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    root = ROOT / "target/wake-tflite"
    build = root / args.target
    for name in SOURCES:
        checkout(name, root / name)
    tflm = root / "tflite-micro"
    deps = root / "deps"
    run(sys.executable, tflm / "cmake/download_deps.py", deps, "--deps", "flatbuffers")
    prepare_frontend(root / "livekit-wakeword", root / "frontend")
    options = []
    if args.target == "aarch64-musl":
        options = [f"-DCMAKE_TOOLCHAIN_FILE={zig_toolchain(root)}"]
    run(
        "cmake", "-S", BRIDGE, "-B", build, "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release",
        f"-DTFLM_SOURCE={tflm.as_posix()}", f"-DTFLM_DEPS_DIR={deps.as_posix()}",
        f"-DLIVEKIT_SOURCE={(root / 'frontend').as_posix()}", *options,
    )
    run("cmake", "--build", build, "--target", "goodwin_wake_tflite", "--parallel", str(args.jobs))


if __name__ == "__main__":
    main()
