use std::io::{BufRead, BufReader, Read};

use anyhow::{Context, Result, ensure};
use goodwin_plugin::provider::{MAX_CHAT_BYTES, MAX_CHAT_CHUNK_BYTES};
use serde_json::Value;

const MAX_WIRE_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn read(response: impl Read, emit: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
    let mut reader = BufReader::new(response.take(MAX_WIRE_BYTES + 1));
    let mut line = String::new();
    let mut data = String::new();
    let mut received = 0;
    let mut text_bytes = 0;
    loop {
        line.clear();
        let count = reader
            .read_line(&mut line)
            .context("Read provider chat stream")?;
        ensure!(count > 0, "Provider chat stream ended before completion");
        received += count as u64;
        ensure!(
            received <= MAX_WIRE_BYTES,
            "Provider chat stream exceeds size limit"
        );
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        } else if line.is_empty() && !data.is_empty() {
            if data == "[DONE]" {
                return Ok(());
            }
            let event: Value =
                serde_json::from_str(&data).context("Invalid provider chat event")?;
            data.clear();
            ensure!(
                event["error"].is_null(),
                "Provider chat stream returned an error"
            );
            let Some(choice) = event["choices"].as_array().and_then(|choices| {
                choices
                    .iter()
                    .find(|choice| choice["index"].as_u64().unwrap_or(0) == 0)
            }) else {
                continue;
            };
            let finish = choice["finish_reason"].as_str();
            ensure!(
                finish != Some("error"),
                "Provider chat stream returned an error"
            );
            if let Some(text) = choice["delta"]["content"].as_str() {
                text_bytes += text.len();
                ensure!(
                    text_bytes <= MAX_CHAT_BYTES,
                    "Provider answer exceeds size limit"
                );
                emit_text(text, emit)?;
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
}
