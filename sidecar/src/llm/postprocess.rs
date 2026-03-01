use crate::llm::grounding::{GroundingContext, StaticGrounding};
use regex::Regex;
use std::sync::LazyLock;
use tracing::debug;

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
/// Minimum consecutive repetitions before collapsing.
const MIN_REPETITIONS: usize = 3;

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

    // Collapse LLM repetition (e.g. "2H 2H 2H 2H" -> "2H")
    text = collapse_repetition(&text);

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

/// Collapse repeated words/phrases that small LLMs produce.
/// "2H 2H 2H 2H 2H 2H" -> "2H"
/// "the sword the sword the sword" -> "the sword"
/// Only triggers on 3+ consecutive repetitions to avoid false positives.
///
/// Checks phrase lengths from 4 words down to 1 word, so longer repeated
/// phrases are caught before shorter ones.
fn collapse_repetition(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < MIN_REPETITIONS {
        return text.to_string();
    }

    let mut result_words = words.clone();
    let mut changed = false;

    // Try phrase lengths shortest first (1 word up to 4 words).
    // Shortest first ensures "2H 2H 2H 2H" is caught as single-token repeat
    // before a multi-word pattern like "2H 2H" could claim it.
    for phrase_len in 1..=4 {
        let mut i = 0;
        let mut new_words: Vec<&str> = Vec::with_capacity(result_words.len());
        while i < result_words.len() {
            // Check if a phrase of `phrase_len` words starting at `i` repeats
            if i + phrase_len * MIN_REPETITIONS <= result_words.len() {
                let phrase = &result_words[i..i + phrase_len];
                let mut reps = 1;
                let mut j = i + phrase_len;
                while j + phrase_len <= result_words.len()
                    && result_words[j..j + phrase_len] == *phrase
                {
                    reps += 1;
                    j += phrase_len;
                }
                if reps >= MIN_REPETITIONS {
                    // Keep one copy of the phrase, skip the rest
                    new_words.extend_from_slice(phrase);
                    i = j; // skip past all repetitions
                    changed = true;
                    debug!(
                        "Collapsed {} repetitions of {:?} in LLM output",
                        reps,
                        phrase.join(" ")
                    );
                    continue;
                }
            }
            new_words.push(result_words[i]);
            i += 1;
        }
        result_words = new_words;
    }

    if changed {
        result_words.join(" ")
    } else {
        text.to_string()
    }
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

/// Regex to find capitalized multi-word sequences that look like entity names.
/// Matches 2+ capitalized words (e.g. "Crystal Depths", "Abyssal Plate").
static PROPER_NOUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Z][a-z]+(?:'s)?(?:\s+(?:of\s+(?:the\s+)?)?[A-Z][a-z]+(?:'s?)?)+)\b").unwrap()
});

/// Levenshtein distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();

    if a_len == 0 { return b_len; }
    if b_len == 0 { return a_len; }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0usize; b_len + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost)
                .min(prev[j + 1] + 1)
                .min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_len]
}

/// Validate that proper nouns in the response match known entities from grounding.
///
/// This is a lightweight safety net -- GEPA grounding should prevent most
/// hallucinations. This catches stragglers by:
/// 1. Extracting capitalized multi-word sequences (potential entity names)
/// 2. Checking each against the grounding lists
/// 3. If a close match exists (Levenshtein distance <= 3), replacing it
/// 4. If no close match, leaving it alone (might be valid common speech)
///
/// Returns the (possibly corrected) text.
pub fn validate_entities(text: &str, grounding: &GroundingContext) -> String {
    validate_entities_full(text, grounding, None)
}

/// Validate entities with optional full static grounding for comprehensive checks.
/// When `static_grounding` is provided, uses the full 2,300+ entity set via HashSet
/// for O(1) exact-match validation before falling back to Levenshtein correction.
pub fn validate_entities_full(
    text: &str,
    grounding: &GroundingContext,
    static_grounding: Option<&StaticGrounding>,
) -> String {
    let known_names = grounding.all_names();
    // Build the full name set for fast exact-match lookups
    let full_names_set = static_grounding.map(|sg| sg.all_names_set());

    if known_names.is_empty() && full_names_set.is_none() {
        return text.to_string();
    }

    let mut result = text.to_string();
    let mut corrections = 0;

    // Find all potential entity names in the text
    let matches: Vec<(String, usize, usize)> = PROPER_NOUN
        .find_iter(text)
        .map(|m| (m.as_str().to_string(), m.start(), m.end()))
        .collect();

    // Process in reverse order to preserve offsets
    for (name, start, end) in matches.into_iter().rev() {
        // Fast path: check full static set first (O(1) HashSet lookup)
        if let Some(ref set) = full_names_set {
            if set.contains(name.as_str()) {
                continue;
            }
        }

        // Check per-request grounding context
        if known_names.iter().any(|&k| k == name) {
            continue;
        }

        // Skip common phrases that look like proper nouns but aren't entities
        let lower = name.to_lowercase();
        if lower.starts_with("friends' club")
            || lower.starts_with("sim player")
            || lower.starts_with("the ")
        {
            continue;
        }

        // Find closest match by Levenshtein distance
        let mut best_match: Option<(&str, usize)> = None;
        for &known in &known_names {
            let dist = levenshtein(&name, known);
            if dist <= 3 {
                if best_match.is_none() || dist < best_match.unwrap().1 {
                    best_match = Some((known, dist));
                }
            }
        }

        if let Some((replacement, dist)) = best_match {
            if dist > 0 {
                debug!(
                    "Entity validation: '{}' -> '{}' (distance: {})",
                    name, replacement, dist
                );
                result.replace_range(start..end, replacement);
                corrections += 1;
            }
        }
        // If no close match found, leave it alone -- might be valid dialog
    }

    if corrections > 0 {
        debug!("Entity validation made {} corrections", corrections);
    }

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

    #[test]
    fn test_collapse_repeated_token() {
        // Classic LLM parrot: single token repeated
        assert_eq!(clean("2H Weapons like the 2H 2H 2H 2H 2H 2H"), "2H Weapons like the 2H");
    }

    #[test]
    fn test_collapse_repeated_phrase() {
        // Multi-word phrase repeated
        assert_eq!(
            clean("Try the sword the sword the sword for combat."),
            "Try the sword for combat."
        );
    }

    #[test]
    fn test_no_collapse_normal_text() {
        // Two repetitions shouldn't trigger (threshold is 3+)
        assert_eq!(clean("go go adventurer"), "go go adventurer");
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("Port Azure", "Port Azure"), 0);
        assert_eq!(levenshtein("Port Azur", "Port Azure"), 1);
        assert_eq!(levenshtein("Port Azyre", "Port Azure"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn test_validate_entities_exact_match() {
        let ctx = GroundingContext {
            zones: vec!["Port Azure".to_string(), "Hidden Hills".to_string()],
            items: vec!["Abyssal Plate".to_string()],
            npcs: vec![],
            classes: vec![],
            quests: vec![],
            enemies: vec![],
        };
        // Exact match should not be changed
        let text = "I was in Port Azure yesterday.";
        assert_eq!(validate_entities(text, &ctx), text);
    }

    #[test]
    fn test_validate_entities_close_match() {
        let ctx = GroundingContext {
            zones: vec!["Port Azure".to_string(), "Hidden Hills".to_string()],
            items: vec!["Abyssal Plate".to_string()],
            npcs: vec![],
            classes: vec![],
            quests: vec![],
            enemies: vec![],
        };
        // Close misspelling should be corrected
        let text = "I found the Abyssal Plat in the cave.";
        let result = validate_entities(text, &ctx);
        assert!(result.contains("Abyssal Plate"), "Got: {}", result);
    }

    #[test]
    fn test_validate_entities_unknown_left_alone() {
        let ctx = GroundingContext {
            zones: vec!["Port Azure".to_string()],
            items: vec![],
            npcs: vec![],
            classes: vec![],
            quests: vec![],
            enemies: vec![],
        };
        // Completely unknown entity should be left alone (distance > 3)
        let text = "Crystal Depths is beautiful.";
        assert_eq!(validate_entities(text, &ctx), text);
    }
}
