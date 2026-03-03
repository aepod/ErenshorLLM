use crate::llm::cloud::ChatMessage;
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

impl PromptBuilder {
    /// Build structured chat messages for local models.
    ///
    /// Returns a system message (personality + context + response instructions)
    /// and a user message (framed as the player speaking). Base instruct models
    /// need explicit instructions to stay in character; fine-tuned models will
    /// follow the instructions too since they're compatible with ChatML format.
    pub fn build_messages(
        personality: &Personality,
        lore_results: &[LoreContext],
        memory_results: &[MemoryContext],
        request: &RespondRequest,
        grounding: Option<&GroundingContext>,
    ) -> Vec<ChatMessage> {
        let mut system = build_system_section(personality, request);

        // Append grounding context
        if let Some(g) = grounding {
            let section = g.format_prompt_section();
            if !section.is_empty() {
                system.push_str(&section);
            }
        }

        // Append lore as world knowledge
        if !lore_results.is_empty() {
            system.push_str("\nWorld knowledge:\n");
            for lore in lore_results {
                system.push_str(&format!("- {}\n", lore.text));
            }
        }

        // Append memory
        if !memory_results.is_empty() {
            system.push_str("\nRecent memory:\n");
            for mem in memory_results {
                system.push_str(&format!("- {}\n", mem.text));
            }
        }

        // Response instructions -- essential for base instruct models that
        // don't know they should stay in character without explicit guidance.
        // Fine-tuned models trained with these instructions will also benefit.
        system.push_str(concat!(
            "\nRespond to the player's message in character as the NPC described above. ",
            "Keep your response to 1-2 sentences. ",
            "Do not use markdown formatting. ",
            "Do not break character or reference being an AI. ",
            "Reference specific lore, items, zones, or memories when relevant.",
        ));

        vec![
            ChatMessage {
                role: "system".to_string(),
                content: system,
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!("{} says: \"{}\"", request.player_name, request.player_message),
            },
        ]
    }
}

impl PromptBuilder {
    /// Build a paraphrase prompt for rephrasing a template through LLM.
    ///
    /// This is a lightweight prompt (~200 tokens) that provides only the
    /// character voice and the template text to rephrase. No world knowledge
    /// or memory context -- the template already contains grounded facts.
    /// GEPA validates the output to prevent entity drift.
    pub fn build_paraphrase(
        personality: &Personality,
        template_text: &str,
        request: &RespondRequest,
        grounding: Option<&GroundingContext>,
    ) -> String {
        let mut prompt = build_paraphrase_system(personality, request);

        if let Some(g) = grounding {
            let section = g.format_prompt_section();
            if !section.is_empty() {
                prompt.push_str(&section);
            }
        }

        prompt.push_str(&format!(
            concat!(
                "\nRephrase the following dialog line while staying in character. ",
                "Keep all proper nouns (zone names, item names, NPC names) exactly as written. ",
                "Vary the sentence structure and word choice. ",
                "Keep it to 1-2 sentences. No markdown.\n\n",
                "Original: \"{}\"\n\n",
                "Rephrased:"
            ),
            template_text
        ));

        prompt
    }

    /// Build structured paraphrase messages for local models.
    pub fn build_paraphrase_messages(
        personality: &Personality,
        template_text: &str,
        request: &RespondRequest,
        grounding: Option<&GroundingContext>,
    ) -> Vec<ChatMessage> {
        let mut system = build_paraphrase_system(personality, request);

        if let Some(g) = grounding {
            let section = g.format_prompt_section();
            if !section.is_empty() {
                system.push_str(&section);
            }
        }

        system.push_str(concat!(
            "\nRephrase the player's dialog line while staying in character. ",
            "Keep all proper nouns (zone names, item names, NPC names) exactly as written. ",
            "Vary the sentence structure and word choice. ",
            "Keep it to 1-2 sentences. No markdown.",
        ));

        vec![
            ChatMessage {
                role: "system".to_string(),
                content: system,
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!("Rephrase: \"{}\"", template_text),
            },
        ]
    }
}

impl PromptBuilder {
    /// Build structured messages for event-triggered paraphrasing.
    ///
    /// Richer than template paraphrase: includes event context, lore snippets,
    /// zone knowledge, and full personality voice. Used by `/v1/paraphrase` for
    /// game event dialog (death reactions, loot requests, combat callouts, etc.).
    ///
    /// Social channels (guild, group, shout) get the full personality treatment:
    /// vocabulary, quirks, example phrases, and strong voice instructions.
    pub fn build_event_paraphrase_messages(
        personality: &Personality,
        canned_text: &str,
        trigger: &str,
        event_context: &std::collections::HashMap<String, String>,
        zone: &str,
        channel: &str,
        relationship: f32,
        lore_results: &[LoreContext],
        grounding: Option<&GroundingContext>,
    ) -> Vec<ChatMessage> {
        let mut system = String::with_capacity(2048);

        // Whether this channel gets full personality enrichment
        let full_personality = matches!(channel, "guild" | "group" | "shout" | "say");

        // Character voice
        system.push_str(&format!(
            "You are {}, a {} in the world of Erenshor.\n",
            personality.name, personality.archetype
        ));
        system.push_str(&format!("Tone: {}\n", personality.tone));

        // Vocabulary -- gives the model character-specific words to use
        if !personality.vocabulary.is_empty() {
            system.push_str(&format!(
                "Vocabulary: {}\n",
                personality.vocabulary.join(", ")
            ));
        }

        if !personality.speech_patterns.is_empty() {
            system.push_str(&format!(
                "Speech patterns: {}\n",
                personality.speech_patterns.join("; ")
            ));
        }

        // Quirks -- personality flavor
        if !personality.quirks.is_empty() {
            system.push_str(&format!("Quirks: {}\n", personality.quirks.join("; ")));
        }

        // Style quirks (full set including typos and emojis)
        if let Some(ref sq) = personality.style_quirks {
            let mut style_notes: Vec<String> = Vec::new();
            if sq.types_in_all_caps {
                style_notes.push("TYPE EVERYTHING IN ALL CAPS".to_string());
            }
            if sq.types_in_all_lowers {
                style_notes.push("type everything in lowercase".to_string());
            }
            if sq.types_in_third_person {
                style_notes.push(format!(
                    "always refer to yourself in the third person as {}",
                    personality.name
                ));
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
                system.push_str(&format!("Writing style: {}\n", style_notes.join("; ")));
            }
        }

        // Example phrases -- strongest signal for voice matching on small models
        if full_personality && !personality.example_phrases.is_empty() {
            // Pick up to 4 examples to keep prompt size reasonable
            let examples: Vec<&str> = personality
                .example_phrases
                .iter()
                .take(4)
                .map(|s| s.as_str())
                .collect();
            system.push_str(&format!(
                "\nExamples of how you talk:\n- {}\n",
                examples.join("\n- ")
            ));
        }

        // Channel context
        if !channel.is_empty() {
            system.push_str(&format!("\nChannel: {} chat\n", channel));
        }

        // Relationship context
        let relationship_desc = match relationship {
            r if r >= 8.0 => "close friend",
            r if r >= 6.0 => "friendly acquaintance",
            r if r >= 4.0 => "neutral",
            r if r >= 2.0 => "wary",
            _ => "hostile",
        };
        system.push_str(&format!(
            "Relationship with player: {} ({:.0}/10)\n",
            relationship_desc, relationship
        ));

        // Zone context
        if !zone.is_empty() {
            system.push_str(&format!("Current zone: {}\n", zone));
        }

        // Event context
        if !event_context.is_empty() {
            system.push_str(&format!("\nEvent type: {}\n", trigger_description(trigger)));
            for (key, value) in event_context {
                system.push_str(&format!("- {}: {}\n", key, value));
            }
        }

        // Lore snippets for world knowledge
        if !lore_results.is_empty() {
            system.push_str("\nWorld knowledge:\n");
            for lore in lore_results {
                system.push_str(&format!("- {}\n", lore.text));
            }
        }

        // GEPA grounding
        if let Some(g) = grounding {
            let section = g.format_prompt_section();
            if !section.is_empty() {
                system.push_str(&section);
            }
        }

        // Channel-aware instructions
        if full_personality {
            // Social channels: push hard for personality voice
            system.push_str(concat!(
                "\nRewrite the following line in YOUR voice. ",
                "Use your vocabulary, speech patterns, and personality quirks. ",
                "Make it sound like something YOU would say, not a generic NPC. ",
                "Weave in event details and world knowledge naturally. ",
                "Keep all proper nouns exactly as written. ",
                "Keep it to 1-2 sentences, casual MMO chat style. No markdown. ",
                "Do NOT repeat the original line verbatim. ",
                "Never say \"in the world of Erenshor\" -- everyone already knows where they are.",
            ));
        } else {
            // Non-social channels: lighter touch
            system.push_str(concat!(
                "\nRephrase the following dialog line in character. ",
                "Weave in event details and world knowledge naturally. ",
                "Keep all proper nouns exactly as written. ",
                "Keep it to 1-2 sentences, casual MMO chat style. No markdown. ",
                "Never say \"in the world of Erenshor\" -- everyone already knows where they are.",
            ));
        }

        vec![
            ChatMessage {
                role: "system".to_string(),
                content: system,
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!("Rephrase: \"{}\"", canned_text),
            },
        ]
    }
}

/// Human-readable description of a trigger type for the LLM prompt.
fn trigger_description(trigger: &str) -> &str {
    match trigger {
        "group_death" => "A group member just died",
        "loot_request" => "Requesting or rolling on loot",
        "loot_drop" => "An item just dropped",
        "group_invite" => "Inviting someone to group or accepting an invite",
        "group_join" => "Joined a group",
        "group_leave" => "Left or was removed from a group",
        "combat_callout" => "Combat situation callout",
        "zone_entry" => "Entering a new zone",
        "zone_exit" => "Leaving a zone",
        "hail" => "Greeting or hailing someone",
        "level_up" => "Someone leveled up",
        "trade" => "Trading or selling items",
        "buff_request" => "Requesting a buff or buff offer",
        "quest_share" => "Sharing a quest",
        "achievement" => "Completed an achievement or milestone",
        "revival" => "Reviving or being revived",
        _ => "General dialog",
    }
}

/// Build the system section for a paraphrase prompt.
/// Lighter than full generation -- only personality voice, no world knowledge.
fn build_paraphrase_system(personality: &Personality, request: &RespondRequest) -> String {
    let mut s = String::with_capacity(512);

    s.push_str(&format!(
        "You are {}, a {} in the world of Erenshor.\n",
        request.sim_name, personality.archetype
    ));
    s.push_str(&format!("Tone: {}\n", personality.tone));

    if !personality.speech_patterns.is_empty() {
        s.push_str(&format!(
            "Speech patterns: {}\n",
            personality.speech_patterns.join("; ")
        ));
    }

    // Style quirks matter for paraphrasing (caps, third person, etc.)
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
            sim_to_sim: false,
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
    fn test_build_paraphrase_prompt() {
        let personality = test_personality();
        let request = test_request();
        let template_text = "Sivakaya's corruption spread from the Monolith across the land.";
        let prompt = PromptBuilder::build_paraphrase(&personality, template_text, &request, None);

        // Contains personality voice
        assert!(prompt.contains("TestSim"));
        assert!(prompt.contains("warrior"));
        assert!(prompt.contains("gruff and direct"));
        // Contains the template text to rephrase
        assert!(prompt.contains("Sivakaya's corruption spread from the Monolith"));
        // Contains rephrase instructions
        assert!(prompt.contains("Rephrase"));
        assert!(prompt.contains("proper nouns"));
        // Does NOT contain full generation context (no zone, no player message)
        assert!(!prompt.contains("Stowaway Strand"));
        assert!(!prompt.contains("What weapon do you use?"));
    }

    #[test]
    fn test_build_paraphrase_messages() {
        let personality = test_personality();
        let request = test_request();
        let template_text = "Farm Azynthi's Garden for the best endgame gear.";
        let messages =
            PromptBuilder::build_paraphrase_messages(&personality, template_text, &request, None);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        // System has personality but not full game context
        assert!(messages[0].content.contains("TestSim"));
        assert!(messages[0].content.contains("Rephrase"));
        // User message has the template text
        assert!(messages[1].content.contains("Azynthi's Garden"));
    }

    #[test]
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

    #[test]
    fn test_build_event_paraphrase_messages() {
        let personality = test_personality();
        let mut event_ctx = HashMap::new();
        event_ctx.insert("dead_member".to_string(), "Phanty".to_string());
        event_ctx.insert("cause".to_string(), "Abyssal Lurker".to_string());

        let lore = vec![LoreContext {
            text: "The Bone Pits are infested with undead and lurking creatures.".to_string(),
        }];

        let messages = PromptBuilder::build_event_paraphrase_messages(
            &personality,
            "Nooo! We lost them!",
            "group_death",
            &event_ctx,
            "The Bone Pits",
            "guild",
            7.0,
            &lore,
            None,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        // System has personality
        assert!(messages[0].content.contains("TestSim"));
        assert!(messages[0].content.contains("warrior"));
        // System has event context
        assert!(messages[0].content.contains("group member just died"));
        assert!(messages[0].content.contains("Phanty"));
        assert!(messages[0].content.contains("Abyssal Lurker"));
        // System has zone
        assert!(messages[0].content.contains("The Bone Pits"));
        // System has lore
        assert!(messages[0].content.contains("infested with undead"));
        // System has vocabulary (guild channel = full personality)
        assert!(messages[0].content.contains("sword"));
        assert!(messages[0].content.contains("battle"));
        // System has quirks
        assert!(messages[0].content.contains("grunts often"));
        // System has example phrases
        assert!(messages[0].content.contains("Draw your blade"));
        // System has channel context
        assert!(messages[0].content.contains("guild chat"));
        // System has relationship
        assert!(messages[0].content.contains("friendly acquaintance"));
        // Full personality instructions (not just "Rephrase")
        assert!(messages[0].content.contains("YOUR voice"));
        // User has the canned text
        assert!(messages[1].content.contains("Nooo! We lost them!"));
    }
}
