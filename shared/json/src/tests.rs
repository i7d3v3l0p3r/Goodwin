use alloc::{format, string::String};

use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Message {
    id: u64,
    text: String,
}

#[test]
fn decodes_escaped_strings_and_ignores_valid_unknown_fields() {
    let input = br#"{"extra":{"list":[null,1.5,true]},"id":18446744073709551615,"text":"\u0410\ud83c\udf19\n"}"#;
    let message = from_slice::<Message>(input).unwrap();
    assert_eq!(message.id, u64::MAX);
    assert_eq!(message.text, "А🌙\n");
    assert_eq!(
        from_slice::<Message>(&to_vec(&message).unwrap()).unwrap(),
        message
    );
}

#[test]
fn accepts_long_escaped_strings_and_grows_the_output() {
    let text = "\n🌙".repeat(300);
    let input = format!(r#"{{"id":1,"text":"{}"}}"#, "\\n\\ud83c\\udf19".repeat(300));
    let message = from_str::<Message>(&input).unwrap();
    assert_eq!(message.text, text);
    assert_eq!(
        from_slice::<Message>(&to_vec(&message).unwrap()).unwrap(),
        message
    );
}

#[test]
fn normalizes_surrogates_and_unescapes_mixed_width_text() {
    let input = r#"{"id":1,"text":"\u0041\u0410\u20AC\uD83C\uDF19\\\n"}"#;
    assert_eq!(from_str::<Message>(input).unwrap().text, "AА€🌙\\\n");
    let input = r#"{"id":1,"text":"\uD83C\uDF19"}"#;
    assert_eq!(from_str::<Message>(input).unwrap().text, "🌙");
}

#[test]
fn long_string_prefix_leaves_special_bytes_for_the_decoder() {
    for extra in 0..size_of::<usize>() {
        let prefix = "x".repeat(512 + extra);
        let json = format!(r#"{{"id":7,"text":"{prefix}\"\ud83c\udf19\\"}}"#);
        assert_eq!(
            from_str::<Message>(&json).unwrap().text,
            format!("{prefix}\"🌙\\"),
        );

        let input = format!(r#"{{"ignored":"{prefix}{}","id":7}}"#, '\u{1f}');
        assert!(validate(input.as_bytes()).is_err());
        assert!(object_fields(input.as_bytes(), ["id"]).is_err());
    }
    assert!(validate(format!("\"{}", "x".repeat(1024)).as_bytes()).is_err());
}

#[test]
fn rejects_malformed_values_even_in_unknown_fields() {
    for invalid in [
        r#"{"ignored":tru,"id":7,"text":"ok"}"#,
        r#"{"ignored":01,"id":7,"text":"ok"}"#,
        r#"{"ignored":[1,],"id":7,"text":"ok"}"#,
        r#"{"ignored":true false,"id":7,"text":"ok"}"#,
        r#"{"id":7,"text":"\ud83c"}"#,
        r#"{"id":7,"text":"ok"} null"#,
    ] {
        assert!(matches!(
            from_str::<Message>(invalid),
            Err(DecodeError::Syntax)
        ));
    }
}

#[test]
fn rejects_deep_unknown_fields_before_the_decoder_can_recurse() {
    let input = format!(
        r#"{{"ignored":{}null{},"id":7,"text":"ok"}}"#,
        "[".repeat(128),
        "]".repeat(128),
    );
    assert!(matches!(
        from_str::<Message>(&input),
        Err(DecodeError::Syntax)
    ));
}

#[test]
fn borrows_only_the_selected_fields_of_an_object() {
    let input = br#" { "list": [null, {"id": 1}], "\u0069d" : 18446744073709551615 } "#;
    let [id, list, missing] = object_fields(input, ["id", "list", "missing"]).unwrap();
    assert_eq!(id, Some(&b"18446744073709551615"[..]));
    assert_eq!(list, Some(&br#"[null, {"id": 1}]"#[..]));
    assert_eq!(missing, None);
    assert!(object_fields(br#"{"id":1,"\u0069d":2}"#, ["id"]).is_err());
    assert!(object_fields(br#"{"id":1,"ignored":tru}"#, ["id"]).is_err());
}
