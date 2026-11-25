use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command},
    time::{SystemTime, UNIX_EPOCH},
};

const UPSTREAM: &str = "https://github.com/wasm-micro-runtime/wasm-micro-runtime.git";
const REVISION: &str = include_str!("revision");

pub(super) fn link(esp: bool) {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let native = root.join("native/wamr");
    let source = prepare_source(&root.join("target/wamr"));
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory")).join("wamr");
    fs::create_dir_all(&output).expect("Create WAMR build directory");
    for file in [
        "revision",
        "build.rs",
        "CMakeLists.txt",
        "bridge.c",
        "esp-idf.patch",
        "table-limit.patch",
    ] {
        println!("cargo:rerun-if-changed={}", native.join(file).display());
    }
    let target = env::var("TARGET").expect("Cargo target");
    let host = env::var("HOST").expect("Cargo host");
    let mut cmake = Command::new("cmake");
    cmake
        .arg("-S")
        .arg(native)
        .arg("-B")
        .arg(&output)
        .args([
            "-G",
            "Ninja",
            "-DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY",
        ])
        .arg(format!("-DWAMR_ROOT_DIR={}", source.display()));

    if esp {
        let includes = embuild::espidf::sysenv::cincl_args().expect("ESP-IDF C compiler arguments");
        cmake
            .args([
                "-DCMAKE_SYSTEM_NAME=Generic",
                "-DCMAKE_SYSTEM_PROCESSOR=xtensa",
                "-DCMAKE_C_COMPILER=xtensa-esp32s3-elf-gcc",
                "-DCMAKE_ASM_COMPILER=xtensa-esp32s3-elf-gcc",
                "-DCMAKE_ASM_FLAGS=-mlongcalls",
                "-DWAMR_BUILD_PLATFORM=esp-idf",
                "-DWAMR_BUILD_TARGET=XTENSA",
            ])
            .arg(format!(
                "-DCMAKE_C_FLAGS={} -DESP_PLATFORM -mlongcalls -fstrict-volatile-bitfields",
                includes.args
            ));
    } else {
        cmake.arg("-DWAMR_BUILD_PLATFORM=linux");
        let arch = if target.starts_with("aarch64-") {
            "AARCH64"
        } else if target.starts_with("x86_64-") {
            "X86_64"
        } else {
            panic!("Unsupported WAMR Linux target {target}");
        };
        cmake.arg(format!("-DWAMR_BUILD_TARGET={arch}"));
        if target != host {
            assert_eq!(
                target, "aarch64-unknown-linux-musl",
                "Unsupported WAMR cross target"
            );
            let toolchain = output.join("aarch64-musl.cmake");
            fs::write(
                &toolchain,
                r#"
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
find_program(ZIG zig REQUIRED)
set(CMAKE_C_COMPILER "${ZIG}" cc -target aarch64-linux-musl)
set(CMAKE_ASM_COMPILER "${ZIG}" cc -target aarch64-linux-musl)
set(CMAKE_C_ARCHIVE_CREATE "\"${ZIG}\" ar rcs <TARGET> <OBJECTS>")
set(CMAKE_ASM_ARCHIVE_CREATE "\"${ZIG}\" ar rcs <TARGET> <OBJECTS>")
set(CMAKE_C_ARCHIVE_FINISH "")
set(CMAKE_ASM_ARCHIVE_FINISH "")
"#,
            )
            .expect("Write WAMR cross toolchain");
            cmake.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", toolchain.display()));
        }
    }

    run(&mut cmake, "Configure WAMR");
    run(
        Command::new("cmake").arg("--build").arg(&output),
        "Build WAMR",
    );
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=goodwin_wamr");
    // musl provides the math symbols in libc, including in Rust's self-contained sysroot.
    if !esp && !target.ends_with("-musl") {
        println!("cargo:rustc-link-lib=m");
    }
}

fn prepare_source(cache: &Path) -> PathBuf {
    let revision = REVISION.trim();
    let source = cache.join(revision);
    if source.is_dir() {
        return source;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let staging = cache.join(format!("download-{}-{nonce}", process::id()));
    fs::create_dir_all(&staging).expect("Create WAMR source staging directory");
    run(
        Command::new("git").current_dir(&staging).arg("init"),
        "Initialize WAMR checkout",
    );
    run(
        Command::new("git")
            .current_dir(&staging)
            .args(["fetch", "--depth=1", UPSTREAM, revision]),
        "Fetch pinned WAMR source",
    );
    run(
        Command::new("git")
            .current_dir(&staging)
            .args(["checkout", "--detach", revision]),
        "Check out pinned WAMR source",
    );
    if let Err(error) = fs::rename(&staging, &source) {
        assert!(source.is_dir(), "Store WAMR checkout: {error}");
    }
    source
}

fn run(command: &mut Command, description: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("{description}: {error}"));
    assert!(status.success(), "{description} failed with {status}");
}
