use serde_json::{Value, json};

use super::reply_with_tools;
use crate::{
    plugins::{Process, VoiceTools},
    providers::Provider,
    voice_core::{
        VOICE_INSTRUCTIONS, strip_wake_word,
        tools::{CONTEXT_INSTRUCTIONS, TOOL_INSTRUCTIONS},
    },
};

#[tokio::test]
async fn voice_model_can_reply_directly_or_use_a_registered_plugin_tool() {
    let provider = Provider::mock(Process::mock_rpc(|method, body| {
        assert_eq!(method, "provider.complete");
        assert_eq!(body["tools"][0]["function"]["name"], "probe__example");
        assert!(body.get("provider").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages[0],
            json!({
                "role": "system",
                "content": format!("{VOICE_INSTRUCTIONS}\n{TOOL_INSTRUCTIONS}")
            })
        );
        let last = messages.last().unwrap();
        let message = if last["role"] == "tool" {
            assert_eq!(last["tool_call_id"], "test-call");
            assert_eq!(
                messages[2]["reasoning_details"],
                json!([{
                    "type": "reasoning.text", "id": "fixture", "text": "reasoning", "signature": null
                }])
            );
            let content: Value = serde_json::from_str(last["content"].as_str().unwrap()).unwrap();
            assert_eq!(content["error"], "clarification_required");
            assert_eq!(body["tool_choice"], "none");
            json!({"role": "assistant", "content": "Какой таймер отменить?"})
        } else if last["content"] == "Отмени таймер" {
            json!({"role": "assistant", "content": null, "reasoning_details": [{
                "type": "reasoning.text", "id": "fixture", "text": "reasoning", "signature": null
            }], "tool_calls": [{
                "id": "test-call", "type": "function",
                "function": { "name": "probe__example", "arguments": "{\"seconds\":300}" }
            }]})
        } else {
            json!({"role": "assistant", "content": "Пять минут"})
        };
        Ok(json!({"message": message}))
    }));
    let result = reply_with_tools(
        &provider,
        "Сколько минут ждать?",
        "",
        VoiceTools::mock(|_| panic!("A question must not call a tool")),
    )
    .await
    .unwrap();
    assert_eq!(result, "Пять минут");
    let result = reply_with_tools(
        &provider,
        "Отмени таймер",
        "",
        VoiceTools::mock(|call| {
            assert_eq!(
                call,
                json!({"name": "example", "arguments": {"seconds": 300}, "voiceTurn": 3})
            );
            json!({"error": "clarification_required"})
        }),
    )
    .await
    .unwrap();
    assert_eq!(result, "Какой таймер отменить?");
}

#[test]
fn strips_only_a_leading_wake_word() {
    assert_eq!(
        strip_wake_word("«Гудвин», сколько времени?", "Гудвин"),
        "сколько времени?"
    );
    assert_eq!(strip_wake_word("Гудвин.", "Гудвин"), "");
    assert_eq!(
        strip_wake_word("Кто такой Гудвин?", "Гудвин"),
        "Кто такой Гудвин?"
    );
    assert_eq!(
        strip_wake_word("ЕГОР, сколько времени?", "Егор"),
        "сколько времени?"
    );
    assert_eq!(
        strip_wake_word("Гудвин, кто такой Егор?", "Егор"),
        "Гудвин, кто такой Егор?"
    );
    assert_eq!(
        strip_wake_word("Hey, Goodwin! What time is it?", "Hey Goodwin"),
        "What time is it?"
    );
}

#[tokio::test]
async fn a_batch_with_a_duplicate_call_id_runs_no_tools() {
    let provider = Provider::mock(Process::mock_rpc(|_, _| {
        let call = json!({
            "id": "duplicated", "function": { "name": "probe__example", "arguments": "{}" }
        });
        Ok(json!({"message": {"role": "assistant", "tool_calls": [&call, &call]}}))
    }));
    let result = reply_with_tools(
        &provider,
        "Отмени таймер",
        "",
        VoiceTools::mock(|_| panic!("An invalid batch must be rejected before running a tool")),
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("repeated tool call identifier")
    );
}

#[tokio::test]
async fn voice_model_can_use_saved_timer_names_before_its_first_tool_call() {
    const PROMPT: &str = "Обращайся ко мне на ты.\nОтвечай спокойно.";
    let provider = Provider::mock(Process::mock_rpc(|method, body| {
        assert_eq!(method, "provider.complete");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages[0],
            json!({
                "role": "system",
                "content": format!("{PROMPT}\n\nUse plain text suitable for speaking aloud.\n{TOOL_INSTRUCTIONS}\n{CONTEXT_INSTRUCTIONS}")
            })
        );
        let snapshot: Value =
            serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            snapshot,
            json!({"pluginContext": {"probe": {"active": [
                {"name": "для яиц", "seconds": 120}, {"name": "для картошки", "seconds": 300}
            ]}}})
        );
        assert_eq!(messages[2]["content"], "Отмени таймер картошка");
        let message = if messages.len() == 3 {
            json!({"role": "assistant", "tool_calls": [{
                "type": "function", "id": "cancel",
                "function": {"name": "probe__example", "arguments": json!({"name": "для картошки"}).to_string()}
            }]})
        } else {
            let result: Value =
                serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                    .unwrap();
            assert_eq!(result["cancelled"]["name"], "для картошки");
            json!({"role": "assistant", "content": "Таймер для картошки отменён."})
        };
        Ok(json!({"message": message}))
    }));
    let tools = VoiceTools::mock(|call| {
        assert_eq!(call["arguments"], json!({"name": "для картошки"}));
        json!({"cancelled": {"name": "для картошки"}})
    })
    .with_mock_context(|call| {
        assert_eq!(
            call,
            json!({"name": "list", "arguments": {}, "voiceTurn": 3})
        );
        json!({"active": [
            {"name": "для яиц", "seconds": 120}, {"name": "для картошки", "seconds": 300}
        ]})
    });
    let result = reply_with_tools(&provider, "Отмени таймер картошка", PROMPT, tools)
        .await
        .unwrap();
    assert_eq!(result, "Таймер для картошки отменён.");
}
