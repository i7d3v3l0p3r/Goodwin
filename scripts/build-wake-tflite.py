"""Build the pinned TensorFlow Lite Micro backend for Goodwin."""

import argparse
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path
from tempfile import NamedTemporaryFile
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parent.parent
BRIDGE = ROOT / "native/wake-tflite"
SOURCES = {
    "tflite-micro": ("tensorflow/tflite-micro", "38a8f0f87302817ddfa99e6a995c7b71ef7118fe"),
    "livekit-wakeword": ("livekit/livekit-wakeword", "a7f6d9040793ee195ec85aaa8884db197a2cde0c"),
}
ESP_MODEL_RELEASE = "https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/download/v0.1.0"
ESP_MODEL_ASSETS = {
    "goodwin-micro-int8.tflite": "985a2f86cc3a7de4ec2489a1709ce70ceac210a3e30c6af858c5b8a5a385c24c",
    "LICENSE": "dc13336d128c111a6f4cfc36e21b00b210a0fdbc6d219061270596aa9fb87bd1",
    "NOTICE": "5dc4995813af89b5a49ba8c839879b51e2a4ca2c7081ed801b529c112f61c8e7",
}


def run(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)


def write_if_changed(path, data):
    if path.is_file() and path.read_bytes() == data:
        return
    file = NamedTemporaryFile(dir=path.parent, delete=False)
    temporary = Path(file.name)
    try:
        with file:
            file.write(data)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


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
        write_if_changed(generated / name, (source / name).read_bytes())
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
    write_if_changed(generated / "lkww_tflm.h", header.encode("utf-8"))


def zig_toolchain(dest):
    zig = shutil.which("zig")
    if not zig:
        raise RuntimeError("Install Zig to cross-compile the wake-word runtime")
    zig = Path(zig).as_posix()
    path = dest / "aarch64-musl.cmake"
    write_if_changed(
        path,
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
'''.encode("utf-8"),
    )
    return path


def esp32s3_toolchain(dest):
    cc = shutil.which("xtensa-esp32s3-elf-gcc")
    cxx = shutil.which("xtensa-esp32s3-elf-g++")
    if not cc or not cxx:
        raise RuntimeError("Activate the ESP-IDF toolchain to build the ESP32-S3 wake runtime")
    path = dest / "esp32s3.cmake"
    write_if_changed(
        path,
        f'''set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR xtensa)
set(CMAKE_C_COMPILER "{Path(cc).as_posix()}")
set(CMAKE_CXX_COMPILER "{Path(cxx).as_posix()}")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_C_FLAGS_INIT "-mlongcalls")
set(CMAKE_CXX_FLAGS_INIT "-mlongcalls")
'''.encode("utf-8"),
    )
    return path


def prepare_esp_model(build):
    for name, sha256 in ESP_MODEL_ASSETS.items():
        path = build / name
        if path.is_file():
            data = path.read_bytes()
        else:
            with urlopen(f"{ESP_MODEL_RELEASE}/{name}", timeout=30) as response:
                data = response.read(1024 * 1024)
        if hashlib.sha256(data).hexdigest() != sha256:
            raise RuntimeError(f"Unexpected SHA-256 for {path}")
        if not path.exists():
            path.write_bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["native", "aarch64-musl", "esp32s3"], default="native")
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
    elif args.target == "esp32s3":
        build.mkdir(exist_ok=True)
        prepare_esp_model(build)
        options = [
            f"-DCMAKE_TOOLCHAIN_FILE={esp32s3_toolchain(root)}",
            "-DTFLM_STRIP_ERROR_STRINGS=ON",
        ]
    run(
        "cmake", "-S", BRIDGE, "-B", build, "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release",
        f"-DTFLM_SOURCE={tflm.as_posix()}", f"-DTFLM_DEPS_DIR={deps.as_posix()}",
        f"-DLIVEKIT_SOURCE={(root / 'frontend').as_posix()}", *options,
    )
    run("cmake", "--build", build, "--target", "goodwin_wake_tflite", "--parallel", str(args.jobs))


if __name__ == "__main__":
    main()
