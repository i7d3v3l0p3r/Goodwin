use std::{env, path::PathBuf};

fn main() {
    if env::var_os("CARGO_FEATURE_WAKE_TFLITE").is_some() {
        link_tflite();
    }
}

fn link_tflite() {
    let target = env::var("TARGET").expect("Cargo sets TARGET");
    let variant = match target.as_str() {
        "aarch64-unknown-linux-musl" => "aarch64-musl",
        "x86_64-unknown-linux-gnu" => "native",
        _ => panic!("wake-tflite is unsupported on {target}"),
    };
    let dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets manifest dir"))
        .join("target/wake-tflite")
        .join(variant)
        .join("lib");
    for library in ["goodwin_wake_tflite", "tflm_micro"] {
        let file = dir.join(format!("lib{library}.a"));
        println!("cargo:rerun-if-changed={}", file.display());
        assert!(
            file.is_file(),
            "Missing {}. Run python scripts/build-wake-tflite.py --target {variant}",
            file.display()
        );
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=goodwin_wake_tflite");
    println!("cargo:rustc-link-lib=static=tflm_micro");
    if variant == "native" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
