using System;
using ErenshorLLMDialog.Pipeline;
using HarmonyLib;
using UnityEngine;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Unified chat intercept hook that patches UpdateSocialLog.GlobalAddLine(ChatLogLine).
    /// This single hook replaces the 3 separate hooks in DialogParaphraseHooks by using
    /// ChatLogLine.LogType for canonical channel detection.
    ///
    /// Currently runs in COMPARISON MODE: logs what it would do, but does not intercept.
    /// The old hooks in DialogParaphraseHooks continue to handle actual interception.
    /// M6 switches this to active mode and removes the old hooks.
    ///
    /// Key invariant: NEVER throws an exception that propagates to the game.
    /// Every code path is wrapped in try/catch.
    /// </summary>
    public static class ChatInterceptHook
    {
        /// <summary>Re-entry guard to prevent re-injected text from being intercepted.</summary>
        private static bool _reentry = false;

        /// <summary>
        /// Whether comparison mode is active. When true, the hook logs its decisions
        /// but returns true (pass through), letting the old hooks handle interception.
        /// Set to false when M6 switches to active mode.
        /// </summary>
        internal static bool ComparisonMode = false;

        /// <summary>
        /// Attempts to patch GlobalAddLine via AccessTools. Returns true if the method
        /// was found and patched successfully. If not found (pre-Mar 10 game version),
        /// returns false and the caller should apply legacy hooks instead.
        /// </summary>
        public static bool TryPatch(Harmony harmony)
        {
            try
            {
                var globalAddLine = AccessTools.Method(
                    typeof(UpdateSocialLog), "GlobalAddLine",
                    new[] { typeof(ChatLogLine) });

                if (globalAddLine == null)
                {
                    ErenshorLLMDialogPlugin.Log.LogWarning(
                        "[ChatInterceptHook] GlobalAddLine not found (pre-Mar 10 game version)");
                    return false;
                }

                harmony.Patch(globalAddLine,
                    prefix: new HarmonyMethod(typeof(ChatInterceptHook), nameof(Prefix)));
                ErenshorLLMDialogPlugin.Log.LogInfo(
                    "[ChatInterceptHook] Patched GlobalAddLine (Mar 10+ chat system)");
                return true;
            }
            catch (Exception ex)
            {
                ErenshorLLMDialogPlugin.Log.LogError(
                    "[ChatInterceptHook] Failed to patch GlobalAddLine: " + ex);
                return false;
            }
        }

        /// <summary>
        /// Prefix hook on UpdateSocialLog.GlobalAddLine(ChatLogLine).
        /// Routes dialog messages to paraphrase pipeline, passes everything else through.
        /// </summary>
        public static bool Prefix(ChatLogLine incoming)
        {
            try
            {
                // Re-entry: this is our own re-injected text, let it through
                if (_reentry)
                    return true;

                // Master toggle
                if (!IsEnabled())
                    return true;

                if (incoming == null || string.IsNullOrEmpty(incoming.MyChatString))
                    return true;

                // Step 1: Check if this is a dialog channel via LogType
                ChatChannel channel = LogTypeMapper.FromLogType(incoming.MyLogType);
                if (channel == ChatChannel.None)
                    return true; // Not dialog (combat log, system, emotes, etc.)

                // Step 2: Parse speaker and body from the chat string
                ParsedMessage parsed = MessageParser.Parse(incoming.MyChatString, channel);
                if (!parsed.IsValid)
                    return true; // Couldn't parse -- not a standard dialog format

                // Step 3: Verify speaker is a known SimPlayer
                if (!ChatClassifiers.IsKnownSim(parsed.Speaker))
                    return true; // System message or real player text

                // Step 4: Skip data-only messages (mana %, coordinates)
                if (ChatClassifiers.IsDataOnly(parsed.Body))
                    return true;

                // Step 5: Skip empty body
                if (string.IsNullOrEmpty(parsed.Body.Trim()))
                    return true;

                // Step 6: Classify combat vs. dialog
                bool isCombat = ChatClassifiers.IsCombatCallout(parsed.Body);

                // --- COMPARISON MODE ---
                if (ComparisonMode)
                {
                    LogComparison(incoming, parsed, channel, isCombat);
                    return true; // Pass through, let old hooks handle
                }

                // --- ACTIVE MODE (M6 enables this) ---
                if (isCombat)
                {
                    return HandleCombatCallout(incoming, parsed, channel);
                }
                else
                {
                    EnqueueParaphrase(incoming, parsed, channel);
                    return false; // Suppress original, paraphrased version re-injected later
                }
            }
            catch (Exception ex)
            {
                // NEVER let an exception propagate to the game
                try
                {
                    ErenshorLLMDialogPlugin.Log.LogError(
                        "[ChatInterceptHook] Exception in Prefix: " + ex);
                }
                catch { }
                return true; // Pass through on error
            }
        }

        /// <summary>
        /// Handle a combat callout synchronously. Combat text must never be delayed.
        /// Uses TemplateCache for instant (&lt;1ms) cached template lookup.
        /// If no cached variant is available, passes through original text unchanged.
        /// </summary>
        private static bool HandleCombatCallout(ChatLogLine incoming, ParsedMessage parsed,
            ChatChannel channel)
        {
            try
            {
                string variant = TemplateCache.FindCombatVariant(
                    parsed.Body, parsed.Speaker, channel);

                if (variant == null)
                    return true; // No cached variant, pass through original

                // Apply the game's personality system (caps, typos, third person, etc.)
                string personalized = variant;
                if (GameData.SimMngr != null && GameData.SimMngr.Sims != null)
                {
                    foreach (var sim in GameData.SimMngr.Sims)
                    {
                        if (sim != null && sim.SimName == parsed.Speaker &&
                            sim.MyAvatar != null)
                        {
                            personalized = GameData.SimMngr.PersonalizeString(
                                variant, sim.MyAvatar);
                            break;
                        }
                    }
                }

                // Reconstruct the formatted message with the template variant
                string formatted = ReformatMessage(parsed.Speaker, parsed.Separator, personalized);

                // Modify the incoming ChatLogLine in place (sync path, no re-injection needed)
                incoming.MyChatString = formatted;
                return true; // Let modified message through
            }
            catch (Exception ex)
            {
                try
                {
                    ErenshorLLMDialogPlugin.Log.LogError(
                        "[ChatInterceptHook] HandleCombatCallout error: " + ex.Message);
                }
                catch { }
                return true; // Pass through on error
            }
        }

        /// <summary>
        /// Enqueue a non-combat dialog message for async paraphrasing via the
        /// existing ParaphraseQueue/EventParaphraser pipeline.
        /// </summary>
        private static void EnqueueParaphrase(ChatLogLine incoming, ParsedMessage parsed,
            ChatChannel channel)
        {
            string zone = ChatClassifiers.GetZone();
            float relationship = ChatClassifiers.GetRelationship(parsed.Speaker);
            string trigger = ClassifyTrigger(parsed.Body, channel);
            var priority = ClassifyPriority(trigger, parsed.Body, channel);

            // Capture values for use in callback closure
            string capturedSep = parsed.Separator;
            string capturedSpeaker = parsed.Speaker;
            ChatLogLine.LogType capturedLogType = incoming.MyLogType;
            string capturedColor = incoming.ColorString;

            ErenshorLLMDialogPlugin.ParaphraseQueue.Enqueue(new ParaphraseJob
            {
                Text = parsed.Body,
                Trigger = trigger,
                SimName = parsed.Speaker,
                Zone = zone,
                Channel = ChannelToString(channel),
                Relationship = relationship,
                Priority = priority,
                OnResult = result =>
                {
                    // Reconstruct the formatted message with paraphrased body
                    string formatted = capturedSpeaker + " " + capturedSep + " " + result;

                    // Fix double-space from separator that already has spaces
                    // Separators from MessageParser are trimmed, so we add spaces
                    // But some separators like "says:" need "Name says: text" format
                    formatted = ReformatMessage(capturedSpeaker, capturedSep, result);

                    // Re-inject with original LogType and color for correct tab routing
                    Reinject(new ChatLogLine(formatted, capturedLogType, capturedColor));
                }
            });
        }

        /// <summary>
        /// Reconstruct a chat message in the correct format for the channel separator.
        /// </summary>
        private static string ReformatMessage(string speaker, string separator, string body)
        {
            // Separators come trimmed from MessageParser (e.g. "says:", "shouts:")
            // We need the original format: "Name says: text"
            switch (separator)
            {
                case "tells the group:":
                    return speaker + " tells the group: " + body;
                case "tells the guild:":
                    return speaker + " tells the guild: " + body;
                case "shouts:":
                    return speaker + " shouts: " + body;
                case "says:":
                    return speaker + " says: " + body;
                case "whispers to you, '":
                case "whispers to you,":
                    return speaker + " whispers to you, '" + body + "'";
                default:
                    return speaker + " " + separator + " " + body;
            }
        }

        /// <summary>
        /// Re-inject a ChatLogLine through GlobalAddLine with the re-entry guard set.
        /// Uses the typed LogAdd(ChatLogLine) overload for correct tab routing.
        /// </summary>
        private static void Reinject(ChatLogLine modified)
        {
            _reentry = true;
            try
            {
                UpdateSocialLog.LogAdd(modified);
            }
            finally
            {
                _reentry = false;
            }
        }

        /// <summary>
        /// Classify dialog text into a paraphrase trigger type.
        /// Mirrors DialogParaphraseHooks logic for compatibility.
        /// </summary>
        private static string ClassifyTrigger(string text, ChatChannel channel)
        {
            if (ChatClassifiers.IsCombatCallout(text))
                return "combat_callout";

            string lower = text.ToLowerInvariant();

            // Death / down
            if (lower.Contains("i'm down") || lower.Contains("im down") ||
                lower.Contains("all dead") || lower.Contains("can't revive"))
                return "group_death";

            // Loot
            if (lower.Contains("loot") || lower.Contains("drop") ||
                lower.Contains("item i mentioned"))
                return "loot_request";

            // XP loss
            if (lower.Contains("xp") || lower.Contains("experience"))
                return "group_death";

            // Zoning
            if (lower.Contains("zoning") || lower.Contains("nowhere to go"))
                return "zone_entry";

            // Greetings
            if (lower.Contains("hey") || lower.Contains("hello") ||
                lower.Contains("what's up") || lower.Contains("greetings") ||
                lower.Contains("howdy"))
                return channel == ChatChannel.Party ? "group_invite" : "hail";

            // Level up congrats
            if (lower.Contains("grats") || lower.Contains("congrat") ||
                lower.Contains("ding") || lower.Contains("nice level"))
                return "level_up";

            // Knowledge responses
            if (lower.Contains("drops from") || lower.Contains("drops in") ||
                lower.Contains("check the wiki") || lower.Contains("haven't seen") ||
                lower.Contains("haven't heard"))
                return "generic";

            return "generic";
        }

        /// <summary>
        /// Assign priority based on trigger type and channel.
        /// </summary>
        private static ParaphrasePriority ClassifyPriority(string trigger, string text,
            ChatChannel channel)
        {
            if (trigger == "combat_callout")
                return ParaphrasePriority.Skip;

            switch (trigger)
            {
                case "group_death":
                    return ParaphrasePriority.High;
                case "group_invite":
                    return ParaphrasePriority.High;
                case "loot_request":
                    return ParaphrasePriority.Normal;
                case "zone_entry":
                    return ParaphrasePriority.Normal;
            }

            // Channel-based priority for generic triggers
            switch (channel)
            {
                case ChatChannel.Whisper:
                    return ParaphrasePriority.Critical;
                case ChatChannel.Party:
                    // Short affirms ("Roger!", "On it!") not worth LLM time
                    if (text.Length < 15)
                        return ParaphrasePriority.Skip;
                    return ParaphrasePriority.Normal;
                case ChatChannel.Guild:
                    return ParaphrasePriority.Normal;
                case ChatChannel.Shout:
                    return ParaphrasePriority.Low;
                case ChatChannel.Say:
                    return ParaphrasePriority.Low;
                case ChatChannel.Trade:
                    return ParaphrasePriority.Low;
                default:
                    return ParaphrasePriority.Low;
            }
        }

        /// <summary>
        /// Convert ChatChannel enum to the string format used by ParaphraseJob.Channel.
        /// </summary>
        private static string ChannelToString(ChatChannel channel)
        {
            switch (channel)
            {
                case ChatChannel.Say: return "say";
                case ChatChannel.Shout: return "shout";
                case ChatChannel.Whisper: return "whisper";
                case ChatChannel.Party: return "group";
                case ChatChannel.Guild: return "guild";
                case ChatChannel.Trade: return "trade";
                case ChatChannel.Hail: return "say";
                default: return "say";
            }
        }

        /// <summary>
        /// Log what the new hook would do, for comparison with old hooks.
        /// Only active in comparison mode with debug logging enabled.
        /// </summary>
        private static void LogComparison(ChatLogLine incoming, ParsedMessage parsed,
            ChatChannel channel, bool isCombat)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging == null ||
                ErenshorLLMDialogPlugin.DebugLogging.Value != Toggle.On)
                return;

            string action = isCombat ? "COMBAT_SYNC" : "PARAPHRASE_ASYNC";
            ErenshorLLMDialogPlugin.Log.LogInfo(
                "[ChatInterceptHook:CMP] " + action +
                " channel=" + channel +
                " logType=" + incoming.MyLogType +
                " speaker=" + parsed.Speaker +
                " body=" + Truncate(parsed.Body, 60));
        }

        private static bool IsEnabled()
        {
            if (ErenshorLLMDialogPlugin.EnableLLMDialog == null ||
                ErenshorLLMDialogPlugin.EnableLLMDialog.Value != Toggle.On)
                return false;

            if (ErenshorLLMDialogPlugin.Paraphraser == null)
                return false;

            if (!ErenshorLLMDialogPlugin.Paraphraser.IsHealthy)
                return false;

            return true;
        }

        private static string Truncate(string s, int max)
        {
            if (s == null) return "";
            return s.Length <= max ? s : s.Substring(0, max) + "...";
        }
    }
}
