use core::fmt;

const MESSAGE_LIMIT: usize = 512;

pub(crate) fn line(prefix: fmt::Arguments<'_>, message: fmt::Arguments<'_>) -> String {
    let mut line = prefix.to_string();
    let mut writer = Message {
        line: &mut line,
        remaining: MESSAGE_LIMIT,
    };
    // A full message stops formatting at the character limit.
    let _ = fmt::write(&mut writer, message);
    line.push('\n');
    line
}

struct Message<'a> {
    line: &'a mut String,
    remaining: usize,
}

impl fmt::Write for Message<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for ch in text.chars() {
            if self.remaining == 0 {
                return Err(fmt::Error);
            }
            self.line.push(if ch.is_control() { ' ' } else { ch });
            self.remaining -= 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::line;

    #[test]
    fn journal_message_replaces_controls_and_preserves_unicode() {
        let reason = "связь\n\t🔊\u{7f}";
        let entry = line(
            format_args!("20ms WARN goodwin::network "),
            format_args!("Failed: {reason}"),
        );
        assert_eq!(entry, "20ms WARN goodwin::network Failed: связь  🔊 \n");
    }

    #[test]
    fn journal_message_limits_characters_across_format_arguments() {
        let text = "ж".repeat(511);
        let end = "🔊z";
        let entry = line(format_args!("prefix "), format_args!("{text}{end}!"));
        assert_eq!(entry, format!("prefix {text}🔊\n"));
    }
}
