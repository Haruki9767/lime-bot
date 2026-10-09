use std::time::Duration;

/// Leave room below Discord's 2,000 UTF-16-code-unit message ceiling.
pub const MAX_DISCORD_MESSAGE_CHARS: usize = 1_800;
const TRUNCATION_MARK: char = '…';
const CODE_BLOCK_OPEN: &str = "```text\n";
const CODE_BLOCK_CLOSE: &str = "\n```";

/// Replaces backticks, mention syntax, and unsafe controls while preserving line breaks.
pub fn escape_discord_text(input: &str) -> String {
    let escaped: String = input
        .chars()
        .map(|ch| match ch {
            '`' => 'ˋ',
            '\n' | '\t' => ch,
            c if c.is_control() => '�',
            c => c,
        })
        .collect();
    escaped
        .replace('@', "@\u{200b}")
        .replace("<#", "<\u{200b}#")
}

/// Truncates to at most `max_units` UTF-16 code units without splitting a scalar value.
pub fn truncate_discord_text(input: &str, max_units: usize) -> String {
    if input.encode_utf16().count() <= max_units {
        return input.to_owned();
    }
    if max_units == 0 {
        return String::new();
    }

    let mut result = String::new();
    let mut used_units = 0;
    let content_budget = max_units - TRUNCATION_MARK.len_utf16();
    for ch in input.chars() {
        let units = ch.len_utf16();
        if used_units + units > content_budget {
            break;
        }
        result.push(ch);
        used_units += units;
    }
    result.push(TRUNCATION_MARK);
    result
}

/// Formats uptime as days, hours, minutes, and seconds, omitting no units.
pub fn format_uptime(duration: Duration) -> String {
    let total = duration.as_secs();
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    format!("{days} days, {hours} hours, {minutes} minutes")
}

/// Produces one escaped code-block message whose complete length is at most 1,800 UTF-16 units.
/// The header is outside the code block; all untrusted text is neutralized before clipping.
pub fn format_code_block(header: &str, content: &str) -> String {
    let safe_header = escape_discord_text(header).replace(['\n', '\t'], " ");
    let safe_content = escape_discord_text(content);

    let header_prefix = if safe_header.is_empty() {
        String::new()
    } else {
        format!("{safe_header}\n")
    };
    let fixed_units =
        utf16_units(&header_prefix) + utf16_units(CODE_BLOCK_OPEN) + utf16_units(CODE_BLOCK_CLOSE);
    let content_budget = MAX_DISCORD_MESSAGE_CHARS.saturating_sub(fixed_units);
    let body = truncate_discord_text(&safe_content, content_budget);

    // A long header must not be allowed to consume the closing fence or message budget.
    if fixed_units > MAX_DISCORD_MESSAGE_CHARS {
        let header_budget = MAX_DISCORD_MESSAGE_CHARS
            - utf16_units(CODE_BLOCK_OPEN)
            - utf16_units(CODE_BLOCK_CLOSE)
            - 1;
        let clipped_header = truncate_discord_text(&safe_header, header_budget);
        return format!("{clipped_header}\n{CODE_BLOCK_OPEN}{CODE_BLOCK_CLOSE}");
    }

    format!("{header_prefix}{CODE_BLOCK_OPEN}{body}{CODE_BLOCK_CLOSE}")
}

fn utf16_units(input: &str) -> usize {
    input.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
fn formats_uptime_at_unit_boundaries() {
    assert_eq!(format_uptime(Duration::ZERO), "0 days, 0 hours, 0 minutes");
    assert_eq!(format_uptime(Duration::from_secs(59)), "0 days, 0 hours, 0 minutes");
    assert_eq!(format_uptime(Duration::from_secs(60)), "0 days, 0 hours, 1 minutes");
    assert_eq!(format_uptime(Duration::from_secs(3_600)), "0 days, 1 hours, 0 minutes");
    assert_eq!(format_uptime(Duration::from_secs(86_400)), "1 days, 0 hours, 0 minutes");
    assert_eq!(format_uptime(Duration::from_secs(90_061)), "1 days, 1 hours, 1 minutes");
}
    #[test]
    fn neutralizes_controls_backtick_fences_and_mass_mentions() {
        let escaped = escape_discord_text("a\0b\x1bc```\nnext\tline @everyone @here");
        assert_eq!(
            escaped,
            "a�b�cˋˋˋ\nnext\tline @\u{200b}everyone @\u{200b}here"
        );
        assert!(!escaped.contains("```"));
        assert!(!escaped.contains("@everyone"));
        assert!(!escaped.contains("@here"));
        assert!(
            !escaped
                .chars()
                .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
        );
    }

    #[test]
    fn neutralizes_user_role_channel_and_mass_mention_tokens() {
        let escaped = escape_discord_text("<@123> <@!123> <@&123> <#123> @everyone @here");
        assert_eq!(
            escaped,
            "<@\u{200b}123> <@\u{200b}!123> <@\u{200b}&123> <\u{200b}#123> @\u{200b}everyone @\u{200b}here"
        );
        for mention in ["<@123>", "<@&123>", "@everyone", "@here", "<#123>"] {
            assert!(!escaped.contains(mention), "mention survived: {mention}");
        }
    }

    #[test]
    fn truncates_without_splitting_scalars_and_respects_utf16_budget() {
        assert_eq!(truncate_discord_text("é🙂x", 3), "é…");
        assert_eq!(truncate_discord_text("🙂🙂", 1), "…");
        assert_eq!(truncate_discord_text("anything", 0), "");
        let clipped = truncate_discord_text(&"🙂".repeat(20), 8);
        assert_eq!(clipped, format!("{}…", "🙂".repeat(3)));
        assert!(utf16_units(&clipped) <= 8);
    }

    #[test]
    fn entire_code_block_message_stays_within_utf16_safe_limit() {
        let message = format_code_block("HTTP result", &format!("🙂{}\n```end", "x".repeat(4_000)));
        assert!(utf16_units(&message) <= MAX_DISCORD_MESSAGE_CHARS);
        assert!(message.starts_with("HTTP result\n```text\n"));
        assert!(message.ends_with("\n```"));
        assert!(!message.contains("```end"));
        assert!(message.contains('…'));
    }

    #[test]
    fn repeated_astral_characters_keep_final_message_within_limit() {
        let message = format_code_block("🙂 header @everyone", &"🚀".repeat(2_000));
        assert!(utf16_units(&message) <= MAX_DISCORD_MESSAGE_CHARS);
        assert!(message.contains("@\u{200b}everyone"));
        assert!(!message.contains("@everyone"));
        assert!(message.ends_with("\n```"));
    }

    #[test]
    fn an_overlong_astral_header_cannot_consume_the_code_fence() {
        let message = format_code_block(&"界🙂".repeat(2_000), "body");
        assert!(utf16_units(&message) <= MAX_DISCORD_MESSAGE_CHARS);
        assert!(message.contains("\n```text\n\n```"));
    }
}
