use regex::Regex;
use std::sync::LazyLock;

static MARKDOWN_BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").unwrap());
static MARKDOWN_ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*(.+?)\*").unwrap());
static MARKDOWN_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`(.+?)`").unwrap());
// Match emote markers like *waves*, *laughs* -- single word (no spaces) between asterisks
static EMOTE_MARKER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*[a-zA-Z]+\*").unwrap());
static INSTRUCTION_LEAK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?mi)^(PERSONALITY|WORLD KNOWLEDGE|MEMORY|INSTRUCTIONS|SYSTEM|CONTEXT|NOTE):.*$")
        .unwrap()
});
static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"  +").unwrap());
static MULTI_NEWLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{2,}").unwrap());

const MAX_RESPONSE_CHARS: usize = 2000;

/// Clean raw LLM output into game-appropriate dialog text.
pub fn clean(raw: &str) -> String {
    let mut text = raw.to_string();

    // Strip markdown: bold first (** **), then emote markers (*word*),
    // then italic (* *) for any remaining single-asterisk formatting.
    // Bold must go first so *bold* inside **bold** isn't caught by emote.
    text = MARKDOWN_BOLD.replace_all(&text, "$1").to_string();
    text = EMOTE_MARKER.replace_all(&text, "").to_string();
    text = MARKDOWN_ITALIC.replace_all(&text, "$1").to_string();
    text = MARKDOWN_CODE.replace_all(&text, "$1").to_string();

    // Remove instruction leakage lines
    text = INSTRUCTION_LEAK.replace_all(&text, "").to_string();

    // Collapse whitespace
    text = MULTI_NEWLINE.replace_all(&text, " ").to_string();
    text = MULTI_SPACE.replace_all(&text, " ").to_string();

    // Trim
    text = text.trim().to_string();

    // Ensure text ends at a complete sentence. LLM output is often cut mid-sentence
    // by the token limit -- trim back to the last sentence boundary.
    text = ensure_complete_sentence(&text);

    // Truncate at sentence boundary if still too long
    if text.len() > MAX_RESPONSE_CHARS {
        text = truncate_at_sentence(&text, MAX_RESPONSE_CHARS);
    }

    text
}

/// Minimum length before we attempt sentence-completion trimming.
/// Short text (template responses, stripped formatting) is left alone since
/// it's typically complete game dialog that just lacks punctuation.
const MIN_SENTENCE_FIX_LEN: usize = 60;

/// Ensure text ends at a complete sentence boundary.
/// LLM output is frequently cut mid-sentence by the token limit. This trims
/// back to the last sentence-ending punctuation (. ! ?). Only activates on
/// text longer than MIN_SENTENCE_FIX_LEN to avoid mangling short dialog.
fn ensure_complete_sentence(text: &str) -> String {
    if text.is_empty() || text.len() < MIN_SENTENCE_FIX_LEN {
        return text.to_string();
    }

    // Already ends with sentence-ending punctuation -- nothing to do
    let last = text.chars().last().unwrap();
    if last == '.' || last == '!' || last == '?' {
        return text.to_string();
    }

    // Find the last sentence boundary
    if let Some(pos) = text.rfind(|c: char| c == '.' || c == '!' || c == '?') {
        // Only trim if we'd keep a meaningful amount of text (at least 30% of original)
        if pos > text.len() * 3 / 10 {
            return text[..=pos].trim().to_string();
        }
    }

    // No usable sentence boundary -- append a period to close the fragment.
    let trimmed = text.trim_end();
    format!("{}.", trimmed)
}

/// Truncate text at the last sentence boundary before max_chars.
/// Falls back to word boundary, then hard cut with ellipsis.
fn truncate_at_sentence(text: &str, max_chars: usize) -> String {
    let search_region = &text[..max_chars];

    // Try to find the last sentence-ending punctuation
    if let Some(pos) = search_region.rfind(|c: char| c == '.' || c == '!' || c == '?') {
        // Include the punctuation mark
        return text[..=pos].trim().to_string();
    }

    // Fall back to word boundary
    if let Some(pos) = search_region.rfind(' ') {
        let mut result = text[..pos].trim().to_string();
        if !result.ends_with(|c: char| c == '.' || c == '!' || c == '?') {
            result.push('.');
        }
        return result;
    }

    // Hard cut (shouldn't happen with normal text)
    let mut result = text[..max_chars].to_string();
    result.push_str("...");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown() {
        // Bold (**) stripped, multi-word italic (*) stripped (preserving text)
        assert_eq!(clean("**bold** and *italic text*"), "bold and italic text");
        // Single-word *emote* is treated as emote marker and removed
        assert_eq!(clean("Hello **friend**"), "Hello friend");
    }

    #[test]
    fn test_strip_code() {
        assert_eq!(clean("use `fireball` spell"), "use fireball spell");
    }

    #[test]
    fn test_strip_instruction_leakage() {
        let input = "Hello adventurer!\nPERSONALITY: scholarly\nHow can I help?";
        assert_eq!(clean(input), "Hello adventurer! How can I help?");
    }

    #[test]
    fn test_strip_emotes() {
        assert_eq!(clean("Hello! *waves* How are you?"), "Hello! How are you?");
    }

    #[test]
    fn test_collapse_whitespace() {
        assert_eq!(clean("Hello   there\n\n\nfriend"), "Hello there friend");
    }

    #[test]
    fn test_truncate_at_sentence() {
        let long = "First sentence. Second sentence. ".repeat(20);
        let result = clean(&long);
        assert!(result.len() <= MAX_RESPONSE_CHARS + 1); // +1 for punctuation
        assert!(result.ends_with('.'));
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(clean(""), "");
    }

    #[test]
    fn test_ensure_complete_sentence_already_complete() {
        assert_eq!(
            ensure_complete_sentence("Hello there adventurer, welcome to our guild hall and enjoy the festivities!"),
            "Hello there adventurer, welcome to our guild hall and enjoy the festivities!"
        );
    }

    #[test]
    fn test_ensure_complete_sentence_short_text_unchanged() {
        // Short text should be left alone (under MIN_SENTENCE_FIX_LEN)
        assert_eq!(
            ensure_complete_sentence("Hello there, adventurer"),
            "Hello there, adventurer"
        );
    }

    #[test]
    fn test_ensure_complete_sentence_mid_cut() {
        // LLM cut mid-sentence -> trims to last complete sentence
        assert_eq!(
            ensure_complete_sentence("I enjoy hunting in the Ashlands near the dragon caves. I was heading to the northern"),
            "I enjoy hunting in the Ashlands near the dragon caves."
        );
    }

    #[test]
    fn test_ensure_complete_sentence_single_fragment() {
        // Single incomplete sentence with no prior boundary (long enough to trigger)
        assert_eq!(
            ensure_complete_sentence("I was just heading to the northern plains where the wolves tend to gather around"),
            "I was just heading to the northern plains where the wolves tend to gather around."
        );
    }

    #[test]
    fn test_ensure_complete_sentence_preserves_question() {
        assert_eq!(
            ensure_complete_sentence("Want to group up and go hunt some wolves together? I know a good spot for hunting near the"),
            "Want to group up and go hunt some wolves together?"
        );
    }

    #[test]
    fn test_mid_sentence_cutoff_through_clean() {
        // End-to-end: raw LLM output cut mid-sentence
        let raw = "Greetings, friend! I've been exploring the caves lately. The monsters down there are";
        let result = clean(raw);
        assert!(result.ends_with('.') || result.ends_with('!') || result.ends_with('?'),
            "Expected sentence-ending punctuation, got: {}", result);
    }
}
