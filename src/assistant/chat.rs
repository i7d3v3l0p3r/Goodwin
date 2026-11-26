use goodwin_error::{Result, ensure};
use goodwin_plugin::provider::{ChatMessage, ChatRequest, ChatRole, Completion, MAX_CHAT_BYTES};

use crate::panel_core::Message;

pub(crate) fn request(instructions: &str, text: &str, history: Vec<Message>) -> ChatRequest {
    request_with_context(instructions, text, history, None)
}

pub(crate) fn request_with_context(
    instructions: impl Into<String>,
    text: &str,
    history: Vec<Message>,
    context: Option<ChatMessage>,
) -> ChatRequest {
    // recent_chat takes the newest messages first to enforce the context limit.
    let history = history.into_iter().rev().map(|message| {
        let role = if message.role == "assistant" {
            ChatRole::Assistant
        } else {
            ChatRole::User
        };
        ChatMessage::text(role, message.text)
    });
    let mut messages = Vec::with_capacity(history.len() + usize::from(context.is_some()) + 2);
    messages.push(ChatMessage::text(ChatRole::System, instructions));
    messages.extend(history);
    messages.extend(context);
    messages.push(ChatMessage::text(ChatRole::User, text));
    ChatRequest {
        messages,
        tools: Vec::new(),
        tool_choice: None,
        max_completion_tokens: 256,
        selection: Default::default(),
    }
}

pub(crate) fn assistant_message(completion: Completion) -> Result<ChatMessage> {
    ensure!(
        completion.message.role == ChatRole::Assistant,
        "Invalid completion role"
    );
    Ok(completion.message)
}

pub(crate) fn append_chunk(answer: &mut String, chunk: &str) -> Result<()> {
    ensure!(
        answer.len() + chunk.len() <= MAX_CHAT_BYTES,
        "Provider answer exceeds size limit"
    );
    answer.push_str(chunk);
    Ok(())
}

pub(crate) fn answer_text(mut text: String) -> Result<String> {
    text.truncate(text.trim_end().len());
    let start = text.len() - text.trim_start().len();
    text.drain(..start);
    ensure!(!text.is_empty(), "Empty provider answer");
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_answers_keep_the_last_valid_chunk_and_trim_unicode_whitespace() {
        let mut answer = String::from("\u{2003}");
        let chunk = "я".repeat((MAX_CHAT_BYTES - answer.len()) / 2);
        append_chunk(&mut answer, &chunk).unwrap();
        append_chunk(&mut answer, "\n").unwrap();
        assert!(append_chunk(&mut answer, "!").is_err());
        assert_eq!(answer_text(answer).unwrap(), chunk);
        assert!(answer_text(" \n".into()).is_err());
    }
}
