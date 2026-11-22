use axum::{Json, Router, routing::post};
use serde_json::{Value, json};

use super::{reply_with_tools, strip_wake_word};
use crate::{
    openrouter::{Models, OpenRouter},
    plugins::VoiceTools,
    test_support::serve,
};

#[tokio::test]
async fn voice_model_can_reply_directly_or_use_a_registered_plugin_tool() {
    let (cloud, task) = serve(Router::new().route(
        "/chat/completions",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["tools"][0]["function"]["name"], "probe__example");
            assert_eq!(body["provider"]["require_parameters"], true);
            assert!(body.get("parallel_tool_calls").is_none());
            let messages = body["messages"].as_array().unwrap();
            let last = messages.last().unwrap();
            let message = if last["role"] == "tool" {
                assert_eq!(last["tool_call_id"], "test-call");
                assert_eq!(messages[2]["reasoning_details"], json!([{"id": "fixture"}]));
                let content: Value =
                    serde_json::from_str(last["content"].as_str().unwrap()).unwrap();
                assert_eq!(content["error"], "clarification_required");
                assert_eq!(body["tool_choice"], "none");
                json!({"role": "assistant", "content": "Какой таймер отменить?"})
            } else if last["content"] == "Отмени таймер" {
                json!({"role": "assistant", "content": null, "reasoning_details": [{"id": "fixture"}], "tool_calls": [{
                    "id": "test-call", "type": "function",
                    "function": { "name": "probe__example", "arguments": "{\"seconds\":300}" }
                }]})
            } else {
                json!({"role": "assistant", "content": "Пять минут"})
            };
            Json(json!({"choices": [{"message": message}]}))
        }),
    ))
    .await;
    let openrouter = OpenRouter::new().unwrap().pointed_at(cloud);
    let models = Models::default();
    let result = reply_with_tools(
        &openrouter,
        "fixture",
        &models,
        "Сколько минут ждать?",
        VoiceTools::mock(|_| panic!("A question must not call a tool")),
    )
    .await
    .unwrap();
    assert_eq!(result, "Пять минут");
    let result = reply_with_tools(
        &openrouter,
        "fixture",
        &models,
        "Отмени таймер",
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
    task.abort();
}

#[test]
fn strips_only_a_leading_wake_word() {
    assert_eq!(
        strip_wake_word("«Гудвин», сколько времени?"),
        "сколько времени?"
    );
    assert_eq!(strip_wake_word("Гудвин."), "");
    assert_eq!(strip_wake_word("Кто такой Гудвин?"), "Кто такой Гудвин?");
}

#[tokio::test]
async fn voice_model_can_use_saved_timer_names_before_its_first_tool_call() {
    let (cloud, task) = serve(Router::new().route(
        "/chat/completions",
        post(|Json(body): Json<Value>| async move {
            let messages = body["messages"].as_array().unwrap();
            let snapshot: Value = serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(snapshot, json!({"pluginContext": {"probe": {"active": [
                {"name": "для яиц", "seconds": 120}, {"name": "для картошки", "seconds": 300}
            ]}}}));
            assert_eq!(messages[2]["content"], "Отмени таймер картошка");
            let message = if messages.len() == 3 {
                json!({"role": "assistant", "tool_calls": [{
                    "type": "function", "id": "cancel",
                    "function": {"name": "probe__example", "arguments": json!({"name": "для картошки"}).to_string()}
                }]})
            } else {
                let result: Value = serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap()).unwrap();
                assert_eq!(result["cancelled"]["name"], "для картошки");
                json!({"role": "assistant", "content": "Таймер для картошки отменён."})
            };
            Json(json!({"choices": [{"message": message}]}))
        }),
    )).await;
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
    let result = reply_with_tools(
        &OpenRouter::new().unwrap().pointed_at(cloud),
        "fixture",
        &Models::default(),
        "Отмени таймер картошка",
        tools,
    )
    .await
    .unwrap();
    assert_eq!(result, "Таймер для картошки отменён.");
    task.abort();
}
