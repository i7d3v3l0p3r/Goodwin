use std::io::{BufRead, BufReader, Read};

use goodwin_error::{Context, Result, ensure};
use goodwin_plugin::{
    json_codec,
    provider::{MAX_CHAT_BYTES, MAX_CHAT_CHUNK_BYTES},
};
use serde::{Deserialize, de::IgnoredAny};

const MAX_WIRE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
struct Event {
    error: Option<IgnoredAny>,
    choices: Option<Vec<Choice>>,
}

#[derive(Deserialize)]
struct Choice {
    index: Option<u64>,
    finish_reason: Option<String>,
    delta: Option<Delta>,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

pub(super) fn read(response: impl Read, emit: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
    let mut reader = BufReader::new(response.take(MAX_WIRE_BYTES + 1));
    let mut line = String::new();
    let mut data = String::new();
    let mut received = 0;
    let mut text_bytes = 0;
    loop {
        line.clear();
        let count = reader
            .by_ref()
            .take(MAX_EVENT_BYTES as u64 + 1)
            .read_line(&mut line)
            .context("Read provider chat stream")?;
        ensure!(
            count <= MAX_EVENT_BYTES,
            "Provider chat line exceeds size limit"
        );
        ensure!(count > 0, "Provider chat stream ended before completion");
        received += count as u64;
        ensure!(
            received <= MAX_WIRE_BYTES,
            "Provider chat stream exceeds size limit"
        );
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            ensure!(
                data.len() + usize::from(!data.is_empty()) + value.len() <= MAX_EVENT_BYTES,
                "Provider chat event exceeds size limit"
            );
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value);
        } else if line.is_empty() && !data.is_empty() {
            if data == "[DONE]" {
                return Ok(());
            }
            let event: Event =
                json_codec::from_str(&data).context("Invalid provider chat event")?;
            data.clear();
            ensure!(
                event.error.is_none(),
                "Provider chat stream returned an error"
            );
            let Some(choice) = event.choices.and_then(|choices| {
                choices
                    .into_iter()
                    .find(|choice| choice.index.unwrap_or(0) == 0)
            }) else {
                continue;
            };
            let finish = choice.finish_reason.as_deref();
            ensure!(
                finish != Some("error"),
                "Provider chat stream returned an error"
            );
            if let Some(text) = choice.delta.and_then(|delta| delta.content) {
                text_bytes += text.len();
                ensure!(
                    text_bytes <= MAX_CHAT_BYTES,
                    "Provider answer exceeds size limit"
                );
                emit_text(&text, emit)?;
            }
            if finish.is_some() {
                return Ok(());
            }
        }
    }
}

pub(super) fn emit_text(mut text: &str, emit: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
    while !text.is_empty() {
        let split = text.floor_char_boundary(text.len().min(MAX_CHAT_CHUNK_BYTES));
        let (chunk, rest) = text.split_at(split);
        emit(chunk)?;
        text = rest;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_lines_and_accumulated_event_data_before_decoding() {
        let payload = "x".repeat(MAX_EVENT_BYTES / 2);
        for (stream, expected) in [
            (format!("data: {payload}{payload}\n"), "line"),
            (format!("data: {payload}\ndata: {payload}\n\n"), "event"),
        ] {
            let error = read(stream.as_bytes(), &mut |_| unreachable!()).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Provider chat {expected} exceeds size limit")
            );
        }
    }

    #[test]
    fn a_completed_stream_accepts_a_finish_reason_or_done_marker() {
        for last in [
            "data: {\"choices\":[{\"index\":0,\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ] {
            let stream = format!(
                "data: {{\"choices\":[{{\"index\":1,\"delta\":{{\"content\":\"ignored\"}}}},{{\"index\":0,\"delta\":{{\"content\":\"Готово\"}}}}]}}\n\n{last}"
            );
            let mut text = String::new();
            read(stream.as_bytes(), &mut |chunk| {
                text.push_str(chunk);
                Ok(())
            })
            .unwrap();
            assert_eq!(text, "Готово");
        }
    }

    #[test]
    fn a_stream_without_a_terminal_event_is_an_error() {
        let mut text = String::new();
        let stream = br#"data: {"choices":[{"delta":{"content":"Partial"}}]}

"#;
        assert!(
            read(stream.as_slice(), &mut |chunk| {
                text.push_str(chunk);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(text, "Partial");
    }

    #[test]
    fn ignores_provider_metadata_and_decodes_escaped_text() {
        let stream = br#"data: {"usage":{"tokens":[1,2]},"choices":[{"finish_reason":"stop","delta":{"content":"\u0410\ud83c\udf19","metadata":{"x":false}}}]}

"#;
        let mut text = String::new();
        read(stream.as_slice(), &mut |chunk| {
            text.push_str(chunk);
            Ok(())
        })
        .unwrap();
        assert_eq!(text, "А🌙");
    }
}
