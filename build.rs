use std::env;
use std::path::PathBuf;

#[path = "native/wamr/build.rs"]
mod wamr;

fn main() {
    let esp = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("espidf");
    let mini3 = env::var_os("CARGO_FEATURE_MODEL_MINI3").is_some();
    let esp_generic = env::var_os("CARGO_FEATURE_MODEL_ESP32S3_GENERIC").is_some();
    let model = match (esp, mini3, esp_generic) {
        (false, _, false) => "mini3",
        (true, false, _) => "esp32s3-generic",
        _ => panic!("Select one model compatible with the platform"),
    };
    println!("cargo:rustc-check-cfg=cfg(goodwin_model, values(\"mini3\", \"esp32s3-generic\"))");
    println!("cargo:rustc-cfg=goodwin_model=\"{model}\"");

    if esp {
        assert_eq!(
            env::var("TARGET").as_deref(),
            Ok("xtensa-esp32s3-espidf"),
            "Goodwin's ESP-IDF adapter targets ESP32-S3"
        );
        embuild::espidf::sysenv::output();
    }
    let linux = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    if (esp || linux) && env::var_os("CARGO_FEATURE_WASM_PLUGINS").is_some() {
        wamr::link(esp);
    }
    if env::var_os("CARGO_FEATURE_WAKE_TFLITE").is_some() {
        link_tflite();
    }
}

fn link_tflite() {
    let target = env::var("TARGET").expect("Cargo sets TARGET");
    let variant = match target.as_str() {
        "aarch64-unknown-linux-musl" => "aarch64-musl",
        "x86_64-unknown-linux-gnu" => "native",
        "xtensa-esp32s3-espidf" => "esp32s3",
        _ => panic!("wake-tflite is unsupported on {target}"),
    };
    let dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets manifest dir"))
        .join("target/wake-tflite")
        .join(variant);
    let libraries = dir.join("lib");
    for library in ["goodwin_wake_tflite", "tflm_micro"] {
        let file = libraries.join(format!("lib{library}.a"));
        println!("cargo:rerun-if-changed={}", file.display());
        assert!(
            file.is_file(),
            "Missing {}. Run python scripts/build-wake-tflite.py --target {variant}",
            file.display()
        );
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-search=native={}", libraries.display());
    println!("cargo:rustc-link-lib=static=goodwin_wake_tflite");
    println!("cargo:rustc-link-lib=static=tflm_micro");
    if variant == "native" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    if variant == "esp32s3" {
        let model = dir.join("goodwin-micro-int8.tflite");
        println!("cargo:rerun-if-changed={}", model.display());
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets output dir"));
        std::fs::copy(&model, out.join("wake.tflite")).unwrap_or_else(|error| {
            panic!(
                "Read {}: {error}. Run python scripts/build-wake-tflite.py --target esp32s3",
                model.display()
            )
        });
    }
}
