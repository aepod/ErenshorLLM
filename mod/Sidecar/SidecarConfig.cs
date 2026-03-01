using System.IO;
using System.Text.RegularExpressions;
using BepInEx.Configuration;
using BepInEx.Logging;

namespace ErenshorLLMDialog.Sidecar
{
    /// <summary>
    /// BepInEx configuration entries for the sidecar process,
    /// response tuning, re-ranking weights, and Phase 3 LLM settings.
    /// </summary>
    public class SidecarConfig
    {
        // ── Section 2: Sidecar ──────────────────────────────────────────

        public ConfigEntry<int> Port { get; }
        public ConfigEntry<Toggle> AutoStart { get; }
        public ConfigEntry<string> DataDir { get; }
        public ConfigEntry<string> BinaryPath { get; }
        public ConfigEntry<float> MinConfidence { get; }
        public ConfigEntry<int> EmbeddingThreads { get; }
        public ConfigEntry<float> HealthPollInterval { get; }
        public ConfigEntry<float> StartupTimeout { get; }
        public ConfigEntry<int> MaxRestarts { get; }
        public ConfigEntry<Toggle> RebuildIndexes { get; }
        public ConfigEntry<Toggle> DumpStyleQuirks { get; }
        public ConfigEntry<Toggle> RestartSidecar { get; }

        // ── Section 3: Response Tuning ──────────────────────────────────

        public ConfigEntry<int> TemplateCandidates { get; }
        public ConfigEntry<int> LoreContextCount { get; }
        public ConfigEntry<int> MemoryContextCount { get; }

        // ── Section 4: Re-Ranking Weights ───────────────────────────────

        public ConfigEntry<float> SemanticWeight { get; }
        public ConfigEntry<float> ChannelWeight { get; }
        public ConfigEntry<float> ZoneWeight { get; }
        public ConfigEntry<float> PersonalityWeight { get; }
        public ConfigEntry<float> RelationshipWeight { get; }

        // ── Section 5: LLM (Phase 3) ───────────────────────────────────

        public ConfigEntry<LlmMode> LlmModeEntry { get; }
        public ConfigEntry<int> ShimmyPort { get; }
        public ConfigEntry<string> ShimmyGpuBackend { get; }

        // ── Section 6: Multi-Sim ─────────────────────────────────────────

        public ConfigEntry<int> MaxRequestsPerMinute { get; }
        public ConfigEntry<Toggle> MultiSimEnabled { get; }
        public ConfigEntry<int> MaxAdditionalSay { get; }
        public ConfigEntry<int> MaxAdditionalGuild { get; }
        public ConfigEntry<int> MaxAdditionalShout { get; }
        public ConfigEntry<Toggle> SimToSimEnabled { get; }
        public ConfigEntry<int> SimToSimMaxDepth { get; }
        public ConfigEntry<string> ApiKey { get; }
        public ConfigEntry<string> ApiEndpoint { get; }
        public ConfigEntry<string> LocalModelPath { get; }
        public ConfigEntry<int> MaxTokens { get; }
        public ConfigEntry<float> Temperature { get; }
        public ConfigEntry<float> LlmTimeout { get; }
        public ConfigEntry<float> EnhanceThreshold { get; }

        public SidecarConfig(ConfigFile config)
        {
            // ── Section 2: Sidecar ──────────────────────────────────────

            Port = config.Bind(
                "2 - Sidecar", "Port", 11435,
                "TCP port the sidecar listens on. Default 11435 (avoids Ollama on 11434).");

            AutoStart = config.Bind(
                "2 - Sidecar", "Auto Start", Toggle.On,
                "Automatically start the sidecar binary when the mod loads.");

            BinaryPath = config.Bind(
                "2 - Sidecar", "Binary Path", "",
                "Path to erenshor-llm.exe. Leave empty for default " +
                "(same folder as the mod DLL).");

            DataDir = config.Bind(
                "2 - Sidecar", "Data Directory", "",
                "Path to the sidecar data directory (lore.json, responses.json, etc.). " +
                "Leave empty for default (data/ subfolder next to the binary).");

            MinConfidence = config.Bind(
                "2 - Sidecar", "Min Confidence", 0.3f,
                new ConfigDescription(
                    "Minimum confidence threshold for sidecar responses. " +
                    "Responses below this are discarded and the fallback transform handles the message.",
                    new AcceptableValueRange<float>(0f, 1f)));

            EmbeddingThreads = config.Bind(
                "2 - Sidecar", "Embedding Threads", 2,
                new ConfigDescription(
                    "Number of CPU threads for the ONNX embedding model. " +
                    "More threads = faster embedding but uses more CPU.",
                    new AcceptableValueRange<int>(1, 8)));

            HealthPollInterval = config.Bind(
                "2 - Sidecar", "Health Poll Interval", 5.0f,
                new ConfigDescription(
                    "Seconds between sidecar health checks.",
                    new AcceptableValueRange<float>(1f, 30f)));

            StartupTimeout = config.Bind(
                "2 - Sidecar", "Startup Timeout", 30.0f,
                new ConfigDescription(
                    "Max seconds to wait for sidecar to become healthy on startup.",
                    new AcceptableValueRange<float>(5f, 120f)));

            MaxRestarts = config.Bind(
                "2 - Sidecar", "Max Restarts", 3,
                new ConfigDescription(
                    "Max number of automatic restart attempts before disabling sidecar.",
                    new AcceptableValueRange<int>(0, 10)));

            RebuildIndexes = config.Bind(
                "2 - Sidecar", "Rebuild Indexes", Toggle.Off,
                "Rebuild lore and template vector indexes on next startup. " +
                "Auto-resets to Off after build completes.");

            DumpStyleQuirks = config.Bind(
                "2 - Sidecar", "Dump Style Quirks", Toggle.Off,
                "Read TypesInAllCaps, TypesInThirdPerson, TypoRate, etc. from game " +
                "prefabs at startup and merge into personality JSON files. " +
                "Run once then disable. Auto-resets to Off after dump completes.");

            RestartSidecar = config.Bind(
                "2 - Sidecar", "Restart Sidecar", Toggle.Off,
                "Toggle On to restart the sidecar and shimmy processes. " +
                "Use after changing LLM Mode or other sidecar settings. " +
                "Syncs BepInEx LLM settings to erenshor-llm.toml before restart. " +
                "Auto-resets to Off after restart completes.");

            // ── Section 3: Response Tuning ──────────────────────────────

            TemplateCandidates = config.Bind(
                "3 - Response Tuning", "Template Candidates", 10,
                new ConfigDescription(
                    "Number of template candidates retrieved before re-ranking.",
                    new AcceptableValueRange<int>(1, 50)));

            LoreContextCount = config.Bind(
                "3 - Response Tuning", "Lore Context Count", 2,
                new ConfigDescription(
                    "Number of lore passages to retrieve for context enrichment.",
                    new AcceptableValueRange<int>(0, 5)));

            MemoryContextCount = config.Bind(
                "3 - Response Tuning", "Memory Context Count", 2,
                new ConfigDescription(
                    "Number of memory entries to retrieve.",
                    new AcceptableValueRange<int>(0, 5)));

            // ── Section 4: Re-Ranking Weights ───────────────────────────

            SemanticWeight = config.Bind(
                "4 - Re-Ranking Weights", "Semantic Weight", 0.20f,
                new ConfigDescription(
                    "Weight for semantic similarity in the re-ranking formula. " +
                    "All 5 weights should ideally sum to ~1.0 but this is not enforced.",
                    new AcceptableValueRange<float>(0f, 1f)));

            ChannelWeight = config.Bind(
                "4 - Re-Ranking Weights", "Channel Weight", 0.15f,
                new ConfigDescription(
                    "Weight for channel match (say, whisper, etc.) in the re-ranking formula.",
                    new AcceptableValueRange<float>(0f, 1f)));

            ZoneWeight = config.Bind(
                "4 - Re-Ranking Weights", "Zone Weight", 0.20f,
                new ConfigDescription(
                    "Weight for zone affinity in the re-ranking formula.",
                    new AcceptableValueRange<float>(0f, 1f)));

            PersonalityWeight = config.Bind(
                "4 - Re-Ranking Weights", "Personality Weight", 0.30f,
                new ConfigDescription(
                    "Weight for personality trait matching in the re-ranking formula.",
                    new AcceptableValueRange<float>(0f, 1f)));

            RelationshipWeight = config.Bind(
                "4 - Re-Ranking Weights", "Relationship Weight", 0.15f,
                new ConfigDescription(
                    "Weight for relationship level in the re-ranking formula.",
                    new AcceptableValueRange<float>(0f, 1f)));

            // ── Section 5: LLM (Phase 3) ────────────────────────────────

            LlmModeEntry = config.Bind(
                "5 - LLM (Phase 3)", "LLM Mode", LlmMode.Off,
                "Whether to use an LLM for response generation. " +
                "Off = template-only (Phase 2 behavior). " +
                "Local = shimmy local inference server (GPU auto-detect). " +
                "Cloud = OpenRouter API. " +
                "Hybrid = local first, cloud fallback.");

            ShimmyPort = config.Bind(
                "5 - LLM (Phase 3)", "Shimmy Port", 8012,
                new ConfigDescription(
                    "TCP port for the shimmy local inference server. " +
                    "Must match [llm.local] endpoint in erenshor-llm.toml.",
                    new AcceptableValueRange<int>(1024, 65535)));

            ShimmyGpuBackend = config.Bind(
                "5 - LLM (Phase 3)", "Shimmy GPU Backend", "auto",
                "GPU backend for shimmy inference. " +
                "auto = let shimmy detect available GPU. " +
                "Other values: vulkan, cuda, metal, cpu.");

            ApiKey = config.Bind(
                "5 - LLM (Phase 3)", "API Key", "",
                "OpenRouter API key for Cloud or Hybrid LLM mode. " +
                "Leave empty to disable cloud inference.");

            ApiEndpoint = config.Bind(
                "5 - LLM (Phase 3)", "API Endpoint", "https://openrouter.ai/api/v1",
                "Cloud LLM endpoint URL (OpenAI-compatible).");

            LocalModelPath = config.Bind(
                "5 - LLM (Phase 3)", "Local Model Path", "",
                "Path to a local GGUF model file (relative to data dir). " +
                "Leave empty for default (models/qwen2.5-0.5b-instruct-q4_k_m.gguf).");

            MaxTokens = config.Bind(
                "5 - LLM (Phase 3)", "Max Tokens", 150,
                new ConfigDescription(
                    "Maximum tokens in LLM-generated responses.",
                    new AcceptableValueRange<int>(10, 500)));

            Temperature = config.Bind(
                "5 - LLM (Phase 3)", "Temperature", 0.7f,
                new ConfigDescription(
                    "LLM sampling temperature. Lower = more deterministic, " +
                    "higher = more creative.",
                    new AcceptableValueRange<float>(0f, 2f)));

            LlmTimeout = config.Bind(
                "5 - LLM (Phase 3)", "LLM Timeout", 15.0f,
                new ConfigDescription(
                    "Max seconds to wait for sidecar /v1/respond. " +
                    "With local LLM, set higher (15-30s). Template-only can use 5s.",
                    new AcceptableValueRange<float>(2f, 60f)));

            EnhanceThreshold = config.Bind(
                "5 - LLM (Phase 3)", "Enhance Threshold", 0.85f,
                new ConfigDescription(
                    "Template confidence threshold below which LLM enhancement is triggered. " +
                    "Templates scoring above this skip the LLM entirely (fast path). " +
                    "Lower values = fewer LLM calls, higher values = more LLM personalization.",
                    new AcceptableValueRange<float>(0f, 1f)));

            // ── Section 6: Multi-Sim ─────────────────────────────────────

            MaxRequestsPerMinute = config.Bind(
                "6 - Multi-Sim", "Max Requests Per Minute", 45,
                new ConfigDescription(
                    "Rate limit for sidecar requests (sliding window). " +
                    "Primary responses are never skipped; additional responders " +
                    "are reduced as the limit approaches.",
                    new AcceptableValueRange<int>(10, 120)));

            MultiSimEnabled = config.Bind(
                "6 - Multi-Sim", "Multi-Sim Enabled", Toggle.On,
                "Enable multiple sims responding to say, guild, and shout messages. " +
                "When off, only the primary responder replies (1:1 behavior).");

            MaxAdditionalSay = config.Bind(
                "6 - Multi-Sim", "Max Additional Say", 2,
                new ConfigDescription(
                    "Max additional responders for say channel (nearby sims within 30f).",
                    new AcceptableValueRange<int>(0, 5)));

            MaxAdditionalGuild = config.Bind(
                "6 - Multi-Sim", "Max Additional Guild", 3,
                new ConfigDescription(
                    "Max additional responders for guild channel (guild roster members).",
                    new AcceptableValueRange<int>(0, 6)));

            MaxAdditionalShout = config.Bind(
                "6 - Multi-Sim", "Max Additional Shout", 3,
                new ConfigDescription(
                    "Max additional responders for shout channel (zone-wide sims).",
                    new AcceptableValueRange<int>(0, 6)));

            SimToSimEnabled = config.Bind(
                "6 - Multi-Sim", "Sim-to-Sim Enabled", Toggle.On,
                "Allow sims to respond to other sims' messages, creating " +
                "natural conversation chains. Gated by rate limit budget.");

            SimToSimMaxDepth = config.Bind(
                "6 - Multi-Sim", "Sim-to-Sim Max Depth", 1,
                new ConfigDescription(
                    "Max conversation depth for sim-to-sim chaining. " +
                    "1 = one reaction layer (sim A -> sim B reacts). " +
                    "Higher values risk rate limit exhaustion.",
                    new AcceptableValueRange<int>(0, 3)));
        }

        /// <summary>
        /// Syncs BepInEx LLM config values into the sidecar's erenshor-llm.toml.
        /// Uses line-by-line replacement so comments and non-LLM sections are preserved.
        /// Returns true if the file was updated.
        /// </summary>
        public bool SyncToSidecarToml(string tomlPath, ManualLogSource log)
        {
            if (!File.Exists(tomlPath))
            {
                log.LogWarning("[SidecarConfig] TOML not found at " + tomlPath +
                    ", cannot sync settings.");
                return false;
            }

            try
            {
                string content = File.ReadAllText(tomlPath);

                // Derive sidecar-side values from BepInEx config
                string modeStr = LlmModeEntry.Value.ToString().ToLowerInvariant();
                bool enabled = LlmModeEntry.Value != LlmMode.Off;

                // [llm] section
                content = SetTomlValue(content, "enabled", enabled ? "true" : "false");
                content = SetTomlValue(content, "mode", "\"" + modeStr + "\"");
                content = SetTomlValue(content, "enhance_threshold",
                    EnhanceThreshold.Value.ToString("F2",
                        System.Globalization.CultureInfo.InvariantCulture));
                content = SetTomlValue(content, "max_tokens", MaxTokens.Value.ToString());
                content = SetTomlValue(content, "temperature",
                    Temperature.Value.ToString("F1",
                        System.Globalization.CultureInfo.InvariantCulture));

                // [llm.local] section
                content = SetTomlValue(content, "endpoint",
                    "\"http://127.0.0.1:" + ShimmyPort.Value + "\"");
                content = SetTomlValue(content, "gpu_backend",
                    "\"" + ShimmyGpuBackend.Value + "\"");

                // [llm.cloud] section
                if (!string.IsNullOrEmpty(ApiKey.Value))
                    content = SetTomlValue(content, "api_key",
                        "\"" + ApiKey.Value + "\"");
                if (!string.IsNullOrEmpty(ApiEndpoint.Value))
                    content = SetTomlValue(content, "api_endpoint",
                        "\"" + ApiEndpoint.Value + "\"");

                // [respond] section weights
                content = SetTomlValue(content, "template_candidates",
                    TemplateCandidates.Value.ToString());
                content = SetTomlValue(content, "lore_candidates",
                    LoreContextCount.Value.ToString());
                content = SetTomlValue(content, "memory_candidates",
                    MemoryContextCount.Value.ToString());
                content = SetTomlValue(content, "zone_weight",
                    ZoneWeight.Value.ToString("F2",
                        System.Globalization.CultureInfo.InvariantCulture));
                content = SetTomlValue(content, "personality_weight",
                    PersonalityWeight.Value.ToString("F2",
                        System.Globalization.CultureInfo.InvariantCulture));
                content = SetTomlValue(content, "relationship_weight",
                    RelationshipWeight.Value.ToString("F2",
                        System.Globalization.CultureInfo.InvariantCulture));
                content = SetTomlValue(content, "channel_weight",
                    ChannelWeight.Value.ToString("F2",
                        System.Globalization.CultureInfo.InvariantCulture));

                File.WriteAllText(tomlPath, content);
                log.LogInfo("[SidecarConfig] Synced BepInEx settings to " + tomlPath);
                log.LogInfo("[SidecarConfig] LLM mode=" + modeStr +
                    ", enabled=" + enabled +
                    ", threshold=" + EnhanceThreshold.Value);
                return true;
            }
            catch (System.Exception e)
            {
                log.LogError("[SidecarConfig] Failed to sync TOML: " + e.Message);
                return false;
            }
        }

        /// <summary>
        /// Replace a TOML key = value on its line, preserving inline comments.
        /// Matches "key = ..." at line start (with optional whitespace).
        /// </summary>
        private static string SetTomlValue(string content, string key, string value)
        {
            // Match: optional whitespace, key, optional whitespace, =, value, optional comment
            string pattern = @"(?m)^(\s*" + Regex.Escape(key) + @"\s*=\s*)([^\n#]*)(.*)$";
            return Regex.Replace(content, pattern, "${1}" + value + " ${3}");
        }
    }
}
