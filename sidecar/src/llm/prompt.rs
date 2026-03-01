use crate::llm::grounding::GroundingContext;
use crate::llm::personality::Personality;
use crate::routes::respond::RespondRequest;

/// Result from lore search, used for prompt context.
pub struct LoreContext {
    pub text: String,
}

/// Result from memory search, used for prompt context.
pub struct MemoryContext {
    pub text: String,
}

// Rough token budget: 1 token ~= 4 characters
const CHARS_PER_TOKEN: usize = 4;

pub struct PromptBuilder;

impl PromptBuilder {
    /// Build a complete prompt for LLM text generation.
    ///
    /// Token budget management: personality and instructions are never truncated.
    /// Memory is truncated first, then lore, to fit within context_budget_tokens.
    /// The optional `grounding` parameter adds a GEPA section with real entity
    /// names to prevent hallucination.
    pub fn build(
        personality: &Personality,
        lore_results: &[LoreContext],
        memory_results: &[MemoryContext],
        request: &RespondRequest,
        context_budget_tokens: usize,
        grounding: Option<&GroundingContext>,
    ) -> String {
        let system_section = build_system_section(personality, request);
        let instruction_section = build_instruction_section(request);

        // Build GEPA grounding section (~200-300 chars, from lore budget)
        let grounding_section = grounding
            .map(|g| g.format_prompt_section())
            .unwrap_or_default();

        // Calculate remaining budget for lore + memory
        let fixed_chars = system_section.len() + instruction_section.len() + grounding_section.len();
        let total_budget_chars = context_budget_tokens * CHARS_PER_TOKEN;
        let remaining_chars = total_budget_chars.saturating_sub(fixed_chars);

        // Split remaining budget: 60% lore, 40% memory
        let lore_budget = (remaining_chars as f32 * 0.6) as usize;
        let memory_budget = remaining_chars.saturating_sub(lore_budget);

        let lore_section = build_lore_section(lore_results, lore_budget);
        let memory_section = build_memory_section(memory_results, memory_budget);

        let mut prompt = system_section;

        // GEPA grounding goes after system section, before lore
        if !grounding_section.is_empty() {
            prompt.push_str(&grounding_section);
        }

        if !lore_section.is_empty() {
            prompt.push_str(&lore_section);
        }

        if !memory_section.is_empty() {
            prompt.push_str(&memory_section);
        }

        prompt.push_str(&instruction_section);
        prompt
    }
}

fn build_system_section(personality: &Personality, request: &RespondRequest) -> String {
    let mut s = String::with_capacity(1024);

    s.push_str(&format!(
        "You are {}, a {} in the world of Erenshor.\n",
        request.sim_name, personality.archetype
    ));
    s.push_str(&format!("Tone: {}\n", personality.tone));

    if !personality.vocabulary.is_empty() {
        s.push_str(&format!(
            "Vocabulary: {}\n",
            personality.vocabulary.join(", ")
        ));
    }

    if !personality.speech_patterns.is_empty() {
        s.push_str(&format!(
            "Speech patterns: {}\n",
            personality.speech_patterns.join("; ")
        ));
    }

    if !personality.quirks.is_empty() {
        s.push_str(&format!("Quirks: {}\n", personality.quirks.join("; ")));
    }

    // Style quirks from personality file
    if let Some(ref sq) = personality.style_quirks {
        let mut style_notes: Vec<String> = Vec::new();
        if sq.types_in_all_caps {
            style_notes.push("TYPE EVERYTHING IN ALL CAPS".to_string());
        }
        if sq.types_in_all_lowers {
            style_notes.push("type everything in lowercase".to_string());
        }
        if sq.types_in_third_person {
            style_notes.push(
                "always refer to yourself in the third person by your name instead of I/me/my"
                    .to_string(),
            );
        }
        if sq.typo_rate > 1.0 {
            style_notes.push("make occasional typos and spelling mistakes".to_string());
        }
        if sq.loves_emojis {
            style_notes.push("use emojis frequently".to_string());
        }
        if !sq.refers_to_self_as.is_empty() {
            style_notes.push(format!(
                "refer to yourself as \"{}\"",
                sq.refers_to_self_as
            ));
        }
        if !style_notes.is_empty() {
            s.push_str(&format!("Writing style: {}\n", style_notes.join("; ")));
        }
    }

    // Game state context
    s.push_str(&format!("\nCurrent zone: {}\n", request.zone));
    s.push_str(&format!("Channel: {}\n", request.channel));
    s.push_str(&format!(
        "Speaking to: {} (level {} {})\n",
        request.player_name, request.player_level, request.player_class
    ));

    let relationship_desc = match request.relationship {
        r if r >= 8.0 => "close friend",
        r if r >= 6.0 => "friendly acquaintance",
        r if r >= 4.0 => "neutral",
        r if r >= 2.0 => "wary",
        _ => "hostile",
    };
    s.push_str(&format!(
        "Relationship: {} ({:.1}/10)\n",
        relationship_desc, request.relationship
    ));

    if !request.group_members.is_empty() {
        s.push_str(&format!(
            "Group members: {}\n",
            request.group_members.join(", ")
        ));
    }

    // Guild context -- affects interaction tone
    if !request.sim_guild.is_empty() {
        s.push_str(&format!("Your guild: {}\n", request.sim_guild));
    }
    if !request.player_guild.is_empty() {
        s.push_str(&format!("Their guild: {}\n", request.player_guild));
    }

    if request.sim_is_rival {
        s.push_str(concat!(
            "\nIMPORTANT: You are a member of Friends' Club, the elite rival guild. ",
            "You are arrogant, dismissive, and competitive toward non-members. ",
            "You look down on other players. Be snarky and condescending, ",
            "like a classic MMO uber-guild player who thinks they are better than everyone. ",
            "Reference your guild's superiority. You are an entertaining jerk, not cruel.\n",
        ));
    } else if !request.sim_guild.is_empty() && !request.player_guild.is_empty() {
        if request.sim_guild == request.player_guild {
            s.push_str("You are in the same guild as the player -- be familiar, friendly, and use guild banter.\n");
        }
    }

    s
}

fn build_lore_section(lore_results: &[LoreContext], budget_chars: usize) -> String {
    if lore_results.is_empty() || budget_chars == 0 {
        return String::new();
    }

    let mut s = String::from("\nWORLD KNOWLEDGE:\n");
    let mut used = s.len();

    for lore in lore_results {
        let entry = format!("- {}\n", lore.text);
        if used + entry.len() > budget_chars {
            break;
        }
        s.push_str(&entry);
        used += entry.len();
    }

    if s == "\nWORLD KNOWLEDGE:\n" {
        return String::new();
    }

    s
}

fn build_memory_section(memory_results: &[MemoryContext], budget_chars: usize) -> String {
    if memory_results.is_empty() || budget_chars == 0 {
        return String::new();
    }

    let mut s = String::from("\nRECENT MEMORY:\n");
    let mut used = s.len();

    for memory in memory_results {
        let entry = format!("- {}\n", memory.text);
        if used + entry.len() > budget_chars {
            break;
        }
        s.push_str(&entry);
        used += entry.len();
    }

    if s == "\nRECENT MEMORY:\n" {
        return String::new();
    }

    s
}

fn build_instruction_section(request: &RespondRequest) -> String {
    format!(
        concat!(
            "\nRespond to the following message in character. ",
            "Keep your response to 1-2 sentences. ",
            "Do not use markdown formatting. ",
            "Do not break character or reference being an AI. ",
            "Reference specific lore, items, zones, or memories when relevant.\n\n",
            "{} says: \"{}\"\n\n",
            "Your response:"
        ),
        request.player_name, request.player_message
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::personality::Personality;
    use std::collections::HashMap;

    fn test_personality() -> Personality {
        Personality {
            name: "TestSim".to_string(),
            archetype: "warrior".to_string(),
            tone: "gruff and direct".to_string(),
            vocabulary: vec!["sword".to_string(), "battle".to_string()],
            speech_patterns: vec!["speaks tersely".to_string()],
            knowledge_areas: vec!["combat".to_string()],
            quirks: vec!["grunts often".to_string()],
            example_phrases: vec!["Draw your blade.".to_string()],
            style_quirks: None,
        }
    }

    fn test_request() -> RespondRequest {
        RespondRequest {
            player_message: "What weapon do you use?".to_string(),
            channel: "say".to_string(),
            sim_name: "TestSim".to_string(),
            personality: HashMap::new(),
            zone: "Stowaway Strand".to_string(),
            relationship: 5.0,
            player_name: "Hero".to_string(),
            player_level: 10,
            player_class: "Paladin".to_string(),
            player_guild: String::new(),
            sim_guild: String::new(),
            sim_is_rival: false,
            group_members: vec![],
            template_candidates: None,
            lore_context_count: None,
            memory_context_count: None,
            w_semantic: None,
            w_channel: None,
            w_zone: None,
            w_personality: None,
            w_relationship: None,
        }
    }

    #[test]
    fn test_build_prompt_basic() {
        let personality = test_personality();
        let request = test_request();
        let prompt = PromptBuilder::build(&personality, &[], &[], &request, 2048, None);

        assert!(prompt.contains("TestSim"));
        assert!(prompt.contains("warrior"));
        assert!(prompt.contains("gruff and direct"));
        assert!(prompt.contains("What weapon do you use?"));
        assert!(prompt.contains("Stowaway Strand"));
    }

    #[test]
    fn test_build_with_lore() {
        let personality = test_personality();
        let request = test_request();
        let lore = vec![LoreContext {
            text: "The Sunken Blade is a legendary weapon found in Coral Depths.".to_string(),
        }];
        let prompt = PromptBuilder::build(&personality, &lore, &[], &request, 2048, None);

        assert!(prompt.contains("WORLD KNOWLEDGE"));
        assert!(prompt.contains("Sunken Blade"));
    }

    #[test]
    fn test_build_with_memory() {
        let personality = test_personality();
        let request = test_request();
        let memory = vec![MemoryContext {
            text: "Hero asked about the Drowned Keep earlier.".to_string(),
        }];
        let prompt = PromptBuilder::build(&personality, &[], &memory, &request, 2048, None);

        assert!(prompt.contains("RECENT MEMORY"));
        assert!(prompt.contains("Drowned Keep"));
    }

    #[test]
    fn test_build_with_style_quirks_caps() {
        use crate::llm::personality::StyleQuirks;
        let mut personality = test_personality();
        personality.style_quirks = Some(StyleQuirks {
            types_in_all_caps: true,
            ..Default::default()
        });
        let request = test_request();
        let prompt = PromptBuilder::build(&personality, &[], &[], &request, 2048, None);

        assert!(prompt.contains("Writing style:"));
        assert!(prompt.contains("ALL CAPS"));
    }

    #[test]
    fn test_build_without_style_quirks() {
        let personality = test_personality();
        let request = test_request();
        let prompt = PromptBuilder::build(&personality, &[], &[], &request, 2048, None);

        assert!(!prompt.contains("Writing style:"));
    }

    #[test]
    fn test_style_quirks_deserialization() {
        let json = r#"{
            "name": "TestSim",
            "archetype": "warrior",
            "tone": "gruff",
            "vocabulary": [],
            "speech_patterns": [],
            "knowledge_areas": [],
            "quirks": [],
            "example_phrases": [],
            "style_quirks": {
                "types_in_all_caps": true,
                "types_in_third_person": true,
                "typo_rate": 5.0,
                "loves_emojis": true,
                "refers_to_self_as": "Blademann"
            }
        }"#;
        let p: Personality = serde_json::from_str(json).unwrap();
        let sq = p.style_quirks.unwrap();
        assert!(sq.types_in_all_caps);
        assert!(sq.types_in_third_person);
        assert!(!sq.types_in_all_lowers); // default
        assert!((sq.typo_rate - 5.0).abs() < 0.01);
        assert!(sq.loves_emojis);
        assert_eq!(sq.refers_to_self_as, "Blademann");
    }

    #[test]
    fn test_style_quirks_optional_missing() {
        let json = r#"{
            "name": "TestSim",
            "archetype": "warrior",
            "tone": "gruff",
            "vocabulary": [],
            "speech_patterns": [],
            "knowledge_areas": [],
            "quirks": [],
            "example_phrases": []
        }"#;
        let p: Personality = serde_json::from_str(json).unwrap();
        assert!(p.style_quirks.is_none());
    }
}
