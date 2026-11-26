use base64::engine::general_purpose::STANDARD;

use super::*;

impl WakeModels {
    pub(crate) fn stage_fixture(&self, id: &str) -> (String, Selection) {
        // A Standard ONNX graph with one Identity node from chunk to score.
        let mut bytes = STANDARD
            .decode(concat!(
                "CAg6VwoYCgVjaHVuaxIFc2NvcmUiCElkZW50aXR5EgdmaXh0dXJl",
                "WhgKBWNodW5rEg8KDQgBEgkKAggBCgMIgApiGAoFc2NvcmUSDwoNCAESCQoCCAEKAwiACkICEA0=",
            ))
            .unwrap();
        // Give each fixture a different producer_name and therefore a different digest.
        bytes.extend([0x12, id.len() as u8]);
        bytes.extend(id.as_bytes());
        let selection = Selection {
            id: id.into(),
            wake_word: "Егор".into(),
            tier: "standard".into(),
            sha256: crate::checksum::hex(&<sha2::Sha256 as sha2::Digest>::digest(&bytes)),
            runtime: Runtime::Standard,
            score_threshold: None,
        };
        let asset = model_path(&self.dir, &selection);
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(&asset, bytes).unwrap();
        let model = checked_model(&asset, &selection).unwrap();
        let ticket = self.start_install().unwrap();
        self.stage_model(selection.clone(), model, self.read_active())
            .unwrap();
        (ticket, selection)
    }
}

fn commit(models: &WakeModels) {
    let candidate = models.take_pending().unwrap();
    models.commit_selection(candidate.manifest).unwrap();
    models.finish_install(candidate.selection);
}

#[test]
fn a_staged_model_resumes_after_restart_and_preserves_the_old_model_until_commit() {
    let dir = tempfile::tempdir().unwrap();
    let models = WakeModels::new(dir.path());
    let (_, previous) = models.stage_fixture("previous");
    commit(&models);
    let (ticket, staged) = models.stage_fixture("staged");
    let previous_asset = model_path(dir.path(), &previous);
    assert!(previous_asset.is_file());
    assert_eq!(read_selection(dir.path()).unwrap(), Some(previous.clone()));
    assert!(matches!(models.start_install(), Err(InstallError::Busy)));
    drop(models);

    let models = WakeModels::new(dir.path());
    assert!(matches!(models.install_result(&ticket), Some(Ok(None))));
    assert_eq!(models.initial_model().unwrap().1, Some(previous));
    assert!(previous_asset.is_file());
    commit(&models);
    assert_eq!(read_selection(dir.path()).unwrap(), Some(staged.clone()));
    assert!(!previous_asset.exists());
    assert!(
        matches!(models.install_result(&ticket), Some(Ok(Some(selection))) if selection == staged)
    );
    drop(models);
    assert!(
        matches!(WakeModels::new(dir.path()).install_result(&ticket),
        Some(Ok(Some(selection))) if selection == staged)
    );
}

#[test]
fn a_restart_after_the_manifest_rename_still_completes_the_original_ticket() {
    let dir = tempfile::tempdir().unwrap();
    let models = WakeModels::new(dir.path());
    let (ticket, selected) = models.stage_fixture("committed");
    models
        .commit_selection(models.take_pending().unwrap().manifest)
        .unwrap();
    drop(models);

    let models = WakeModels::new(dir.path());
    assert_eq!(models.initial_model().unwrap().1, Some(selected.clone()));
    assert!(matches!(models.install_result(&ticket), Some(Ok(None))));
    commit(&models);
    assert!(
        matches!(models.install_result(&ticket), Some(Ok(Some(selection))) if selection == selected)
    );
}

#[test]
fn preparing_is_interrupted_after_restart_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let models = WakeModels::new(dir.path());
    let ticket = models.start_install().unwrap();
    drop(models);

    let models = WakeModels::new(dir.path());
    assert!(matches!(
        models.install_result(&ticket),
        Some(Err(InstallError::Interrupted))
    ));
    assert_ne!(models.start_install().unwrap(), ticket);
    assert!(models.install_result(&ticket).is_none());
}

#[test]
fn a_missing_staged_asset_keeps_the_active_model_and_reports_failure() {
    let dir = tempfile::tempdir().unwrap();
    let models = WakeModels::new(dir.path());
    let (_, previous) = models.stage_fixture("previous");
    commit(&models);
    let (ticket, staged) = models.stage_fixture("staged");
    fs::remove_file(model_path(dir.path(), &staged)).unwrap();
    drop(models);

    let models = WakeModels::new(dir.path());
    assert_eq!(models.initial_model().unwrap().1, Some(previous));
    assert!(models.take_pending().is_none());
    assert!(matches!(
        models.install_result(&ticket),
        Some(Err(InstallError::Failed))
    ));
}
