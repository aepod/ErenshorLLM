using System.Collections.Generic;
using ErenshorLLMDialog.Pipeline;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Harmony hooks that intercept canned game dialog and route it through the
    /// sidecar's /v1/paraphrase endpoint for personality-enriched rewording.
    ///
    /// Covers:
    /// 1. Group chat via SimPlayerGrouping.AddStringForDisplay -- all queued group
    ///    dialog (combat callouts, join/leave, pulls, targeting, affirms, etc.)
    /// 2. Direct LogAdd calls with group color -- dismiss dialog, etc.
    /// 3. Shout output from SimPlayerShoutParse.Update queue
    /// 4. Say output from SimPlayerShoutParse.Update queue
    ///
    /// On failure or timeout, original text passes through unchanged (graceful degradation).
    /// </summary>
    public static class DialogParaphraseHooks
    {
        /// <summary>Re-entry guard to prevent paraphrased text from being re-intercepted.</summary>
        private static bool _reentry = false;

        private const string GROUP_SEP = " tells the group: ";
        private const string SHOUT_SEP = " shouts: ";
        private const string SAY_SEP = " says: ";
        private const string GUILD_SEP = " tells the guild: ";

        /// <summary>
        /// Prefix hook on SimPlayerGrouping.AddStringForDisplay.
        /// Intercepts ALL queued group dialog, paraphrases the text portion,
        /// and re-queues with the paraphrased result.
        /// </summary>
        [HarmonyPatch(typeof(SimPlayerGrouping), "AddStringForDisplay")]
        public class AddStringForDisplayPatch
        {
            static bool Prefix(SimPlayerGrouping __instance, string disp, string col)
            {
                // Let our own re-queued text through
                if (_reentry)
                    return true;

                if (!IsEnabled())
                    return true;

                // Parse "SimName tells the group: text"
                int idx = disp.IndexOf(GROUP_SEP);
                if (idx <= 0)
                    return true;

                string simName = disp.Substring(0, idx);
                string text = disp.Substring(idx + GROUP_SEP.Length);

                // Skip pure data messages (mana %, coordinates)
                if (IsDataOnly(text))
                    return true;

                // Skip empty text
                if (string.IsNullOrEmpty(text.Trim()))
                    return true;

                string zone = GetZone();
                string trigger = ClassifyGroupTrigger(text);
                float relationship = GetRelationship(simName);
                var priority = ClassifyGroupPriority(trigger, text);

                ErenshorLLMDialogPlugin.ParaphraseQueue.Enqueue(new ParaphraseJob
                {
                    Text = text,
                    Trigger = trigger,
                    SimName = simName,
                    Zone = zone,
                    Channel = "group",
                    Relationship = relationship,
                    Priority = priority,
                    OnResult = result =>
                    {
                        if (__instance == null)
                            return;

                        string formatted = simName + GROUP_SEP + result;
                        _reentry = true;
                        __instance.AddStringForDisplay(formatted, col);
                        _reentry = false;
                    }
                });

                return false; // suppress original
            }
        }

        /// <summary>
        /// Prefix hook on UpdateSocialLog.LogAdd(string, string) -- the colored overload.
        /// Intercepts direct group/shout/say/guild dialog output from non-queued sources
        /// (DismissMember, shout queue output, etc.) and paraphrases the text.
        /// </summary>
        [HarmonyPatch(typeof(UpdateSocialLog), "LogAdd", new[] { typeof(string), typeof(string) })]
        public class LogAddColoredPatch
        {
            // Note: game method is `static string LogAdd(string _string, string _colorAsString)`
            // Harmony requires param names to match exactly.
            static bool Prefix(ref string __result, string _string, string _colorAsString)
            {
                if (_reentry)
                    return true;

                if (!IsEnabled())
                    return true;

                // Only intercept known sim dialog colors
                string channel;
                string sep;
                switch (_colorAsString)
                {
                    case "#00B2B7": // group
                        channel = "group";
                        sep = GROUP_SEP;
                        break;
                    case "#FF9000": // shout
                        channel = "shout";
                        sep = SHOUT_SEP;
                        break;
                    case "green": // guild
                        channel = "guild";
                        sep = GUILD_SEP;
                        break;
                    default:
                        return true; // not sim dialog color, pass through
                }

                // Parse "SimName <sep> text"
                int idx = _string.IndexOf(sep);
                if (idx <= 0)
                    return true;

                string simName = _string.Substring(0, idx);
                string text = _string.Substring(idx + sep.Length);

                // Verify this is a known sim (not a system message)
                if (!IsKnownSim(simName))
                    return true;

                // Skip data-only messages
                if (IsDataOnly(text))
                    return true;

                if (string.IsNullOrEmpty(text.Trim()))
                    return true;

                string zone = GetZone();
                string trigger = ClassifyTrigger(text, channel);
                float relationship = GetRelationship(simName);

                // Combat callouts must be instant regardless of channel
                var priority = trigger == "combat_callout"
                    ? ParaphrasePriority.Skip
                    : ClassifyChannelPriority(channel);

                // Capture color and sep for use in callback
                string color = _colorAsString;
                string capturedSep = sep;

                ErenshorLLMDialogPlugin.ParaphraseQueue.Enqueue(new ParaphraseJob
                {
                    Text = text,
                    Trigger = trigger,
                    SimName = simName,
                    Zone = zone,
                    Channel = channel,
                    Relationship = relationship,
                    Priority = priority,
                    OnResult = result =>
                    {
                        string formatted = simName + capturedSep + result;
                        _reentry = true;
                        UpdateSocialLog.LogAdd(formatted, color);
                        _reentry = false;
                    }
                });

                // Set return value since we're skipping the original (returns string)
                __result = _string;
                return false; // suppress original
            }
        }

        /// <summary>
        /// Prefix hook on the single-param UpdateSocialLog.LogAdd(string).
        /// Catches say-channel output from QueueSay (no color param).
        /// Format: "SimName says: text"
        /// </summary>
        [HarmonyPatch(typeof(UpdateSocialLog), "LogAdd", new[] { typeof(string) })]
        public class LogAddPlainPatch
        {
            // Note: game method is `static void LogAdd(string _string)`
            static bool Prefix(string _string)
            {
                if (_reentry)
                    return true;

                if (!IsEnabled())
                    return true;

                // Only intercept "Name says: text" pattern
                int idx = _string.IndexOf(SAY_SEP);
                if (idx <= 0)
                    return true;

                // Check for color tags (from DispTxt group queue) -- skip those,
                // they were already paraphrased by AddStringForDisplay hook
                if (_string.StartsWith("<color="))
                    return true;

                string simName = _string.Substring(0, idx);
                string text = _string.Substring(idx + SAY_SEP.Length);

                if (!IsKnownSim(simName))
                    return true;

                if (string.IsNullOrEmpty(text.Trim()))
                    return true;

                string zone = GetZone();
                string trigger = ClassifyTrigger(text, "say");
                float relationship = GetRelationship(simName);

                // Combat callouts must be instant regardless of channel
                var priority = trigger == "combat_callout"
                    ? ParaphrasePriority.Skip
                    : ParaphrasePriority.Low;

                ErenshorLLMDialogPlugin.ParaphraseQueue.Enqueue(new ParaphraseJob
                {
                    Text = text,
                    Trigger = trigger,
                    SimName = simName,
                    Zone = zone,
                    Channel = "say",
                    Relationship = relationship,
                    Priority = priority,
                    OnResult = result =>
                    {
                        string formatted = simName + SAY_SEP + result;
                        _reentry = true;
                        UpdateSocialLog.LogAdd(formatted);
                        _reentry = false;
                    }
                });

                return false; // suppress original
            }
        }

        // --- Helper methods ---

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

        private static string GetZone()
        {
            return SceneManager.GetActiveScene().name;
        }

        /// <summary>
        /// Check if a name matches a known SimPlayer.
        /// </summary>
        private static bool IsKnownSim(string name)
        {
            if (GameData.SimMngr == null || GameData.SimMngr.Sims == null)
                return false;

            foreach (SimPlayerTracking sim in GameData.SimMngr.Sims)
            {
                if (sim != null && sim.SimName == name)
                    return true;
            }

            return false;
        }

        /// <summary>
        /// Get the sim's relationship/opinion of the player.
        /// Returns a 0-10 scale; defaults to 5 if unknown.
        /// </summary>
        private static float GetRelationship(string simName)
        {
            if (GameData.SimMngr == null || GameData.SimMngr.Sims == null)
                return 5f;

            foreach (SimPlayerTracking sim in GameData.SimMngr.Sims)
            {
                if (sim != null && sim.SimName == simName)
                {
                    // OpinionOfPlayer is typically -10 to 10; normalize to 0-10
                    return Mathf.Clamp((sim.OpinionOfPlayer + 10f) / 2f, 0f, 10f);
                }
            }

            return 5f;
        }

        /// <summary>
        /// Skip text that is purely informational data (mana %, coordinates).
        /// These should not be reworded.
        /// </summary>
        private static bool IsDataOnly(string text)
        {
            // Mana percentages: "47% mana"
            if (text.Contains("% mana"))
                return true;

            // Coordinate reports: "I'm at 123.4"
            if (text.StartsWith("I'm at ") && text.Contains(","))
                return true;

            // Pull constant on/off system messages
            if (text.Contains("Auto Pull:"))
                return true;

            return false;
        }

        /// <summary>
        /// Classify group chat text into a paraphrase trigger type.
        /// The sidecar uses this to pick the right prompt template.
        /// Uses shared IsCombatCallout() for combat detection.
        /// </summary>
        private static string ClassifyGroupTrigger(string text)
        {
            string lower = text.ToLowerInvariant();

            // Death / down
            if (lower.Contains("i'm down") || lower.Contains("im down") ||
                lower.Contains("all dead") || lower.Contains("can't revive"))
                return "group_death";

            // Combat callouts -- shared detection
            if (IsCombatCallout(text))
                return "combat_callout";

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

            // Join greetings (Hello list)
            if (lower.Contains("hey") || lower.Contains("hello") ||
                lower.Contains("what's up") || lower.Contains("greetings"))
                return "group_invite";

            // Leave goodbyes
            if (lower.Contains("bye") || lower.Contains("later") ||
                lower.Contains("see ya") || lower.Contains("peace"))
                return "generic";

            // Affirms (Roger!, On it!, etc.)
            if (lower.Length < 25)
                return "generic";

            return "generic";
        }

        /// <summary>
        /// Assign priority for group chat based on trigger type.
        /// Combat callouts are instant (Skip) -- healing, pulling, aggro, etc. must
        /// not be delayed by LLM processing. Death reactions and invites are high priority.
        /// Short affirms skip entirely.
        /// </summary>
        private static ParaphrasePriority ClassifyGroupPriority(string trigger, string text)
        {
            switch (trigger)
            {
                case "group_death":
                    return ParaphrasePriority.High;
                case "combat_callout":
                    return ParaphrasePriority.Skip; // must be instant
                case "loot_request":
                    return ParaphrasePriority.Normal;
                case "group_invite":
                    return ParaphrasePriority.High;
                case "zone_entry":
                    return ParaphrasePriority.Normal;
                default:
                    // Short affirms ("Roger!", "On it!") -- not worth LLM time
                    if (text.Length < 15)
                        return ParaphrasePriority.Skip;
                    return ParaphrasePriority.Normal;
            }
        }

        /// <summary>
        /// Assign priority based on chat channel for non-group output.
        /// Whispers are critical, shout is low (ambient), guild is normal.
        /// </summary>
        private static ParaphrasePriority ClassifyChannelPriority(string channel)
        {
            switch (channel)
            {
                case "whisper":
                    return ParaphrasePriority.Critical;
                case "guild":
                    return ParaphrasePriority.Normal;
                case "shout":
                    return ParaphrasePriority.Low;
                default:
                    return ParaphrasePriority.Low;
            }
        }

        /// <summary>
        /// Check if text is a combat callout that must be delivered instantly.
        /// Shared by all hooks to ensure combat text is never delayed by LLM.
        /// </summary>
        private static bool IsCombatCallout(string text)
        {
            string lower = text.ToLowerInvariant();

            // Pulling
            if (lower.StartsWith("pulling ") || lower.Contains(" is here, attack"))
                return true;

            // Healing / buffing
            if (lower.StartsWith("casting ") || lower.Contains("hot incoming") ||
                lower.Contains("incoming on ") || lower.Contains("regrowth") ||
                lower.Contains("healing "))
                return true;

            // Targeting / assist
            if (lower.Contains("assisting ") || lower.Contains("killing ") ||
                lower.StartsWith("targeting ") || lower.StartsWith("i'm on "))
                return true;

            // Taunt
            if (lower.Contains("taunting ") || lower.Contains("ae taunt"))
                return true;

            // CC
            if (lower.Contains("can't be stunned") || lower.Contains("get on that one"))
                return true;

            // OOM / mana
            if (lower.Contains("oom") || lower.Contains("meditat") ||
                lower.Contains("restoring my mana"))
                return true;

            // Aggro
            if (lower.Contains("have aggro") || lower.Contains("it's on me") ||
                lower.Contains("aggro"))
                return true;

            // Environmental damage
            if (lower.Contains("ow") && lower.Length < 20)
                return true;

            // Close call
            if (lower.Contains("close one"))
                return true;

            // Stance
            if (lower.Contains("stance"))
                return true;

            return false;
        }

        /// <summary>
        /// Classify non-group dialog text into a paraphrase trigger.
        /// </summary>
        private static string ClassifyTrigger(string text, string channel)
        {
            // Combat callouts are instant -- check first
            if (IsCombatCallout(text))
                return "combat_callout";

            string lower = text.ToLowerInvariant();

            // Knowledge responses
            if (lower.Contains("drops from") || lower.Contains("drops in") ||
                lower.Contains("check the wiki") || lower.Contains("haven't seen") ||
                lower.Contains("haven't heard"))
                return "generic";

            // Greetings
            if (lower.Contains("hey") || lower.Contains("hello") ||
                lower.Contains("greetings") || lower.Contains("howdy"))
                return "hail";

            // Farewells
            if (lower.Contains("goodnight") || lower.Contains("night") ||
                lower.Contains("bye"))
                return "generic";

            // LFG
            if (lower.Contains("coming") || lower.Contains("on my way") ||
                lower.Contains("omw"))
                return "generic";

            // Level up congrats
            if (lower.Contains("grats") || lower.Contains("congrat") ||
                lower.Contains("ding") || lower.Contains("nice level"))
                return "level_up";

            // Insults
            if (channel == "say" || channel == "shout")
                return "generic";

            return "generic";
        }
    }
}
