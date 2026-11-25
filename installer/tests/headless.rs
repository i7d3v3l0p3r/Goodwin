#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn headless_installer_returns_the_panel_url_and_propagates_remote_failures() {
    let scratch = tempfile::tempdir().unwrap();
    let package = scratch.path().join("Local build");
    fs::create_dir(&package).unwrap();
    let mut elf = [0; 20];
    elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
    elf[18] = 0xb7;
    fs::write(package.join("goodwin"), elf).unwrap();

    let adb = scratch.path().join("mock adb");
    fs::write(
        &adb,
        r#"#!/bin/sh
set -eu
if [ "$1" = connect ]; then
    [ "$2" = station.local:5556 ]
    echo 'mock ADB connected'
    exit
fi
[ "$1" = -s ] && [ "$2" = station.local:5556 ]
case "$3" in
    get-state) echo device ;;
    push) [ -f "$4" ] ;;
    shell)
        case "$4" in
            *GOODWIN_STATION_READY*) echo GOODWIN_STATION_READY ;;
            *config.json*) echo update ;;
            'df -Pk /tmp')
                printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\ntmpfs 65536 0 %s 0%% /tmp\n' "$MOCK_TMP_KIB"
                ;;
            *mktemp*)
                case "$4" in *" $MOCK_UPLOAD_PARENT/.goodwin-upload.XXXXXX") ;; *) exit 95 ;; esac
                echo "$MOCK_UPLOAD_PARENT/.goodwin-upload.test01"
                ;;
            'sh '*)
                case "$4" in *' keep; status='*) ;; *) exit 92 ;; esac
                if [ "$MOCK_INSTALL_STATUS" = 0 ]; then echo 'GOODWIN_INSTALL_OK update'; fi
                echo "GOODWIN_INSTALL_EXIT $MOCK_INSTALL_STATUS"
                ;;
            'rm -rf '*) ;;
            *) exit 93 ;;
        esac
        ;;
    *) exit 94 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&adb, fs::Permissions::from_mode(0o700)).unwrap();

    let run = |status: &str, tmp_kib: &str, upload_parent: &str| {
        Command::new(env!("CARGO_BIN_EXE_goodwin-installer"))
            .args(["install", "station.local:5556", "--model", "keep", "--adb"])
            .arg(&adb)
            .arg("--package")
            .arg(&package)
            .env("MOCK_INSTALL_STATUS", status)
            .env("MOCK_TMP_KIB", tmp_kib)
            .env("MOCK_UPLOAD_PARENT", upload_parent)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    };
    let success = run("0", "65536", "/tmp");
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    assert_eq!(success.stdout, b"http://station.local:8787/\n");
    assert!(String::from_utf8_lossy(&success.stderr).contains("mock ADB connected"));

    let failure = run("2", "0", "/data/opt");
    assert_eq!(failure.status.code(), Some(1));
    assert!(failure.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("завершился с кодом 2"));
}
