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
    let wake_tflite = env::var_os("CARGO_FEATURE_WAKE_TFLITE").is_some();
    assert!(
        esp || wake_tflite || env::var_os("CARGO_FEATURE_WAKE_ONNX").is_some(),
        "Select wake-onnx or wake-tflite for the Station build"
    );
    println!("cargo:rerun-if-env-changed=GOODWIN_NATIVE_SANITIZERS");
    let sanitize = env::var_os("GOODWIN_NATIVE_SANITIZERS").is_some();
    if sanitize {
        assert_eq!(
            env::var("TARGET").as_deref(),
            Ok("x86_64-unknown-linux-gnu"),
            "Native sanitizers require x86-64 GNU/Linux"
        );
        println!("cargo:rustc-link-lib=dylib=asan");
        println!("cargo:rustc-link-lib=dylib=ubsan");
    }
    if (esp || linux) && env::var_os("CARGO_FEATURE_WASM_PLUGINS").is_some() {
        wamr::link(esp, sanitize);
    }
    if wake_tflite {
        link_tflite(sanitize);
    }
}

fn link_tflite(sanitize: bool) {
    let target = env::var("TARGET").expect("Cargo sets TARGET");
    let variant = match target.as_str() {
        "aarch64-unknown-linux-musl" => "aarch64-musl",
        "x86_64-unknown-linux-gnu" => "native",
        "xtensa-esp32s3-espidf" => "esp32s3",
        _ => panic!("wake-tflite is unsupported on {target}"),
    };
    let dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets manifest dir"))
        .join("target/wake-tflite")
        .join(if sanitize {
            "native-sanitized"
        } else {
            variant
        });
    let libraries = dir.join("lib");
    for library in ["goodwin_wake_tflite", "tflm_micro"] {
        let file = libraries.join(format!("lib{library}.a"));
        println!("cargo:rerun-if-changed={}", file.display());
        assert!(
            file.is_file(),
            "Missing {}. Run python scripts/build-wake-tflite.py --target {variant}{}",
            file.display(),
            if sanitize { " --sanitize" } else { "" },
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
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets output dir"));
        for name in [
            "goodwin-micro-int8.tflite",
            "microwakeword/goodwin-microwakeword.tflite",
        ] {
            let model = dir.join(name);
            println!("cargo:rerun-if-changed={}", model.display());
            std::fs::copy(&model, out.join(model.file_name().expect("model filename")))
                .unwrap_or_else(|error| {
                    panic!(
                        "Read {}: {error}. Run python scripts/build-wake-tflite.py --target esp32s3",
                        model.display()
                    )
                });
        }
    }
}
