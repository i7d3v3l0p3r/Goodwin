"""Build the pinned TensorFlow Lite Micro backend for Goodwin."""

import argparse
import hashlib
import io
import shutil
import subprocess
import sys
from pathlib import Path
from tempfile import NamedTemporaryFile
from urllib.request import urlopen
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parent.parent
BRIDGE = ROOT / "native/wake-tflite"
SOURCES = {
    "tflite-micro": ("tensorflow/tflite-micro", "38a8f0f87302817ddfa99e6a995c7b71ef7118fe"),
    "livekit-wakeword": ("livekit/livekit-wakeword", "a7f6d9040793ee195ec85aaa8884db197a2cde0c"),
    "esp-nn": ("espressif/esp-nn", "ace42ac7721119cd543fc40ffe719e601fa8f20f"),
}
ESP_MODEL_RELEASE = "https://github.com/i7d3v3l0p3r/Goodwin-wakeword/releases/download/v0.1.0"
ESP_MODEL_ASSETS = {
    "goodwin-micro-int8.tflite": "985a2f86cc3a7de4ec2489a1709ce70ceac210a3e30c6af858c5b8a5a385c24c",
    "LICENSE": "dc13336d128c111a6f4cfc36e21b00b210a0fdbc6d219061270596aa9fb87bd1",
    "NOTICE": "5dc4995813af89b5a49ba8c839879b51e2a4ca2c7081ed801b529c112f61c8e7",
}
MICRO_WAKE_WORD_RELEASE = ESP_MODEL_RELEASE.replace("v0.1.0", "v0.3.0")
MICRO_WAKE_WORD_ASSETS = {
    "goodwin-microwakeword.tflite": "7a78f81336045fc99e6e34ccee488072706d3025a8010f6570fc59ef7637f6ca",
    "LICENSE": ESP_MODEL_ASSETS["LICENSE"],
    "NOTICE": "41c7ca0839b1667194745fbb5071d07ec6cf1e8e792c1ddfb43305c486434a8a",
}
MICRO_FRONTEND_URL = (
    "https://components-file.espressif.com/components/esphome/esp-micro-speech-features/"
    "1.2.3/download/9f9065ae-1281-47a6-a58c-7b796c1fe30c.zip"
)
MICRO_FRONTEND_SHA = "f98c15ca683f63e5cf95809d3e1a722f272a9b354911dbc12c975e3a310acd82"


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


def prepare_esp32s3_frontend(frontend, generated):
    header = (frontend / "lkww_tflm.h").read_text(encoding="utf-8")
    before, rest = header.split("namespace lkww_conv {", 1)
    conv, after = rest.split("}  // namespace lkww_conv", 1)
    changes = (
        ("  bool ours;", "  const int32_t* correction;\n  const int8_t* packed;\n  bool ours;"),
        ("inline TfLiteStatus Prepare(",
         "inline int8_t Requantize(int32_t, const lkww_tfl::OpDataConv&, int, int32_t);\n"
         '#include "esp32s3-conv.inc"\n\ninline TfLiteStatus Prepare('),
        ("  if (d->ours && !grouped_1) {",
         "  d->correction = PrepareSimd(c, *in, *w, *out, b, &d->packed);\n"
         "  d->ours = d->ours || d->correction != nullptr;\n"
         "  if (d->ours && !grouped_1 && d->correction == nullptr) {"),
        ("  ConvS8(p, d,", "  if (data->correction != nullptr) {\n"
         "    ConvSimd(p, d, tflite::micro::GetTensorShape(in), tflite::micro::GetTensorData<int8_t>(in),\n"
         "             tflite::micro::GetTensorShape(w), tflite::micro::GetTensorData<int8_t>(w),\n"
         "             tflite::micro::GetOptionalTensorData<int32_t>(b), tflite::micro::GetTensorShape(out),\n"
         "             tflite::micro::GetTensorData<int8_t>(out), data->correction, data->packed);\n"
         "    return kTfLiteOk;\n"
         "  }\n"
         "  ConvS8(p, d,"),
    )
    for old, new in changes:
        assert conv.count(old) == 1, old
        conv = conv.replace(old, new)
    before = before.replace('#include "tensorflow/lite/micro/kernels/conv.h"',
                            '#include "tensorflow/lite/micro/kernels/conv.h"\n'
                            '#include "tensorflow/lite/micro/kernels/depthwise_conv.h"')
    changes = (
        ("    LoadMelParams();",
         "    if (interpreter_.SetMicroExternalContext(&packed_weights_) != kTfLiteOk) return;\n"
         "    LoadMelParams();"),
        ("      r.AddDepthwiseConv2D();", "      r.AddDepthwiseConv2D(lkww_conv::DepthwiseRegistration());"),
        ("  tflite::MicroInterpreter interpreter_;",
         "  lkww_conv::PackedWeights packed_weights_;\n  tflite::MicroInterpreter interpreter_;"),
    )
    for old, new in changes:
        assert after.count(old) == 1, old
        after = after.replace(old, new)
    generated.mkdir(exist_ok=True)
    for name in ("lkww_frontend.c", "lkww_micro.h", "lkww_tables.h"):
        write_if_changed(generated / name, (frontend / name).read_bytes())
    header = before + "namespace lkww_conv {" + conv + "}  // namespace lkww_conv" + after
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
set(CMAKE_ASM_COMPILER "{Path(cc).as_posix()}")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_C_FLAGS_INIT "-mlongcalls")
set(CMAKE_CXX_FLAGS_INIT "-mlongcalls")
'''.encode("utf-8"),
    )
    return path


def verified_asset(path, url, sha256):
    if path.is_file():
        data = path.read_bytes()
    else:
        with urlopen(url, timeout=30) as response:
            data = response.read(1024 * 1024)
    if hashlib.sha256(data).hexdigest() != sha256:
        raise RuntimeError(f"Unexpected SHA-256 for {path}")
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        write_if_changed(path, data)
    return data


def prepare_model_assets(build, names, release=ESP_MODEL_RELEASE, digests=ESP_MODEL_ASSETS):
    for name in names:
        verified_asset(build / name, f"{release}/{name}", digests[name])


def prepare_micro_frontend(root):
    source = root / "esp-micro-speech-features"
    data = verified_asset(root / "esp-micro-speech-features-1.2.3.zip", MICRO_FRONTEND_URL, MICRO_FRONTEND_SHA)
    with ZipFile(io.BytesIO(data)) as archive:
        for member in archive.namelist():
            relative = Path(member).relative_to(source.name)
            if relative == Path("LICENSE") or (
                len(relative.parts) == 2 and relative.parts[0] in {"include", "src"}
                and relative.suffix in {".c", ".h"}
            ):
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                write_if_changed(path, archive.read(member))
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["native", "aarch64-musl", "esp32s3"], default="native")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--sanitize", action="store_true", help="instrument the native runtime with ASan and UBSan")
    args = parser.parse_args()
    if args.sanitize and args.target != "native":
        parser.error("--sanitize requires --target native")
    root = ROOT / "target/wake-tflite"
    build = root / ("native-sanitized" if args.sanitize else args.target)
    for name in ("tflite-micro", "livekit-wakeword"):
        checkout(name, root / name)
    tflm = root / "tflite-micro"
    deps = root / "deps"
    run(sys.executable, tflm / "cmake/download_deps.py", deps, "--deps", "flatbuffers")
    frontend = root / "frontend"
    prepare_frontend(root / "livekit-wakeword", frontend)
    options = []
    if args.sanitize:
        prepare_model_assets(build, ("goodwin-micro-int8.tflite",))
        flags = "-g -fsanitize=address,undefined -fno-omit-frame-pointer"
        options = ["-DCMAKE_C_COMPILER=gcc", "-DCMAKE_CXX_COMPILER=g++",
                   f"-DCMAKE_C_FLAGS={flags}", f"-DCMAKE_CXX_FLAGS={flags}",
                   "-DCMAKE_C_FLAGS_RELEASE=-O1 -DNDEBUG", "-DCMAKE_CXX_FLAGS_RELEASE=-O1 -DNDEBUG"]
    elif args.target == "aarch64-musl":
        options = [f"-DCMAKE_TOOLCHAIN_FILE={zig_toolchain(root)}"]
    elif args.target == "esp32s3":
        prepare_model_assets(build, ESP_MODEL_ASSETS)
        prepare_model_assets(build / "microwakeword", MICRO_WAKE_WORD_ASSETS,
                             MICRO_WAKE_WORD_RELEASE, MICRO_WAKE_WORD_ASSETS)
        micro_frontend = prepare_micro_frontend(root)
        checkout("esp-nn", root / "esp-nn")
        prepare_esp32s3_frontend(frontend, root / "frontend-esp32s3")
        frontend = root / "frontend-esp32s3"
        options = [
            f"-DCMAKE_TOOLCHAIN_FILE={esp32s3_toolchain(root)}",
            "-DTFLM_STRIP_ERROR_STRINGS=ON",
            f"-DESP_NN_SOURCE={(root / 'esp-nn').as_posix()}",
            f"-DMICRO_FRONTEND_SOURCE={micro_frontend.as_posix()}",
        ]
    run(
        "cmake", "-S", BRIDGE, "-B", build, "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release",
        f"-DTFLM_SOURCE={tflm.as_posix()}", f"-DTFLM_DEPS_DIR={deps.as_posix()}",
        f"-DLIVEKIT_SOURCE={frontend.as_posix()}", *options,
    )
    run("cmake", "--build", build, "--target", "goodwin_wake_tflite", "--parallel", str(args.jobs))


if __name__ == "__main__":
    main()
