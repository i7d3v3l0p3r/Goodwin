use super::*;

fn arguments(args: &[&str]) -> Result<Action> {
    parse(args.iter().copied().map(OsString::from))
}

#[test]
fn only_a_bare_invocation_launches_the_gui() {
    assert!(matches!(arguments(&[]).unwrap(), Action::Gui));
    assert!(matches!(arguments(&["--help"]).unwrap(), Action::Help));
    assert!(matches!(
        arguments(&["install", "--help"]).unwrap(),
        Action::Help
    ));
    assert!(matches!(
        arguments(&["--version"]).unwrap(),
        Action::Version
    ));
    assert!(arguments(&["other"]).is_err());
}

#[test]
fn installation_defaults_match_the_gui() {
    let Action::Install(options) = arguments(&["install", "station.local"]).unwrap() else {
        panic!("Expected an installation");
    };
    assert_eq!(options.address.serial(), "station.local:5555");
    assert!(options.adb.is_none());
    assert!(matches!(options.source, Source::Latest));
    assert!(matches!(options.model, Model::Keep));
}

#[test]
fn installation_accepts_a_named_adb_and_a_local_package() {
    let Action::Install(options) = arguments(&[
        "install",
        "[fd00::10]:4444",
        "--adb",
        "Platform Tools/adb",
        "--model",
        "goodwin-micro",
        "--package",
        "Local Builds/goodwin.tar.gz",
    ])
    .unwrap() else {
        panic!("Expected an installation");
    };
    assert_eq!(options.address.serial(), "[fd00::10]:4444");
    assert_eq!(options.adb.unwrap(), PathBuf::from("Platform Tools/adb"));
    assert_eq!(options.model, Model::Id("goodwin-micro".into()));
    let Source::Local(path) = options.source else {
        panic!("Expected a local package")
    };
    assert_eq!(path, PathBuf::from("Local Builds/goodwin.tar.gz"));
}

#[test]
fn invalid_options_fail_before_an_installation_can_run() {
    for args in [
        &["install"][..],
        &["install", "http://station.local"][..],
        &["install", "station.local", "--package"][..],
        &["install", "station.local", "--package", "--model", "micro"][..],
        &["install", "station.local", "--model", "../unknown"][..],
        &[
            "install",
            "station.local",
            "--model",
            "micro",
            "--model",
            "keep",
        ][..],
        &["install", "station.local", "--unknown"][..],
    ] {
        assert!(arguments(args).is_err(), "{args:?}");
    }
}

#[cfg(unix)]
#[test]
fn local_paths_keep_native_non_utf8_bytes() {
    use std::os::unix::ffi::OsStringExt;

    let path = OsString::from_vec(b"local-\xff.tar.gz".to_vec());
    let Action::Install(options) = parse(
        [
            OsString::from("install"),
            OsString::from("station.local"),
            OsString::from("--package"),
            path.clone(),
        ]
        .into_iter(),
    )
    .unwrap() else {
        panic!("Expected an installation");
    };
    let Source::Local(actual) = options.source else {
        panic!("Expected a local package")
    };
    assert_eq!(actual, PathBuf::from(path));
}
