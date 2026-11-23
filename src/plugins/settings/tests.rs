use serde_json::{Value, json};

use super::{Failure, Field, SettingsData, View, apply, validate};

fn fields() -> Vec<Field> {
    serde_json::from_value(json!([
        {"id":"base_url","type":"url","required":true,"label":{"en":"URL","ru":"URL"}},
        {"id":"api_key","type":"secret","label":{"en":"Key","ru":"Ключ"}},
        {"id":"enabled","type":"boolean","label":{"en":"Enabled","ru":"Включено"}},
        {"id":"retries","type":"integer","label":{"en":"Retries","ru":"Попытки"}}
    ]))
    .unwrap()
}

#[test]
fn settings_patches_validate_types_and_the_required_base_url() {
    let fields = fields();
    validate(&fields).unwrap();
    for patch in [
        json!({"base_url":"file:///tmp/fixture"}),
        json!({"base_url":"http://alice:secret@127.0.0.1/v1"}),
        json!({"base_url":"http://localhost/v1","enabled":"true"}),
        json!({"base_url":"http://localhost/v1","retries":1.5}),
        json!({"base_url":"http://localhost/v1","retries":9_007_199_254_740_992_i64}),
        json!({"base_url":"http://localhost/v1","unknown":true}),
        json!({"api_key":"fixture"}),
    ] {
        assert!(matches!(
            apply(
                &fields,
                SettingsData::default(),
                patch.as_object().unwrap().clone()
            ),
            Err(Failure::InvalidSettings)
        ));
    }
    let fields_with_duplicate_id = [fields.clone(), fields].concat();
    assert!(validate(&fields_with_duplicate_id).is_err());
}

#[test]
fn secret_values_survive_other_edits_and_are_always_hidden_by_the_view() {
    let fields = fields();
    let patch =
        json!({"base_url":" http://localhost:8000/v1 ","api_key":"fixture secret","enabled":false});
    let initial = apply(
        &fields,
        SettingsData::default(),
        patch.as_object().unwrap().clone(),
    )
    .ok()
    .unwrap();
    let patch = json!({"retries":0});
    let updated = apply(&fields, initial, patch.as_object().unwrap().clone())
        .ok()
        .unwrap();
    assert_eq!(updated.values["base_url"], "http://localhost:8000/v1");
    assert_eq!(updated.values["enabled"], false);
    assert_eq!(updated.values["retries"], 0);
    assert_eq!(updated.secrets["api_key"], "fixture secret");

    let patch = json!({"api_key":null});
    let removed = apply(
        &fields,
        SettingsData {
            values: updated.values.clone(),
            secrets: updated.secrets.clone(),
        },
        patch.as_object().unwrap().clone(),
    )
    .ok()
    .unwrap();
    assert!(removed.secrets.is_empty());

    let mut changed_schema = fields.clone();
    changed_schema[1].kind = super::Kind::Text;
    let hidden = serde_json::to_value(View::new(
        changed_schema,
        SettingsData {
            values: updated.values.clone(),
            secrets: updated.secrets.clone(),
        },
    ))
    .unwrap();
    assert_eq!(hidden["values"]["api_key"], Value::Null);

    let hidden = serde_json::to_value(View::new(fields, updated)).unwrap();
    assert_eq!(hidden["configuredSecrets"], json!(["api_key"]));
    assert!(!hidden.to_string().contains("fixture secret"));
}
