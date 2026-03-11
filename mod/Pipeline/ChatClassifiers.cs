using System.Collections.Generic;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Pipeline
{
    public struct PersonalityHint
    {
        public string Style;
        public string ClassRole;
    }

    public struct PersonalityData
    {
        public string Style;
        public string ClassRole;
        public string[] Traits;
        public string SourceSim;
    }

    /// <summary>
    /// Consolidated classification utilities for chat messages.
    /// Extracted from DialogParaphraseHooks for shared use by ChatInterceptHook (M2).
    /// All methods are static and null-safe.
    /// </summary>
    public static class ChatClassifiers
    {
        /// <summary>
        /// Check if text is a combat callout that must be delivered instantly.
        /// Combat callouts should never be delayed by LLM processing.
        /// </summary>
        public static bool IsCombatCallout(string text)
        {
            if (string.IsNullOrEmpty(text))
                return false;

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
        /// Skip text that is purely informational data (mana %, coordinates).
        /// These should not be reworded.
        /// </summary>
        public static bool IsDataOnly(string text)
        {
            if (string.IsNullOrEmpty(text))
                return false;

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
        /// Check if a name matches a known SimPlayer.
        /// </summary>
        public static bool IsKnownSim(string name)
        {
            if (string.IsNullOrEmpty(name))
                return false;

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
        public static float GetRelationship(string simName)
        {
            if (string.IsNullOrEmpty(simName))
                return 5f;

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
        /// Get the current zone/scene name.
        /// </summary>
        public static string GetZone()
        {
            return SceneManager.GetActiveScene().name;
        }

        /// <summary>
        /// Get a lightweight personality hint for a sim, suitable for template selection.
        /// Returns style (nice/tryhard/mean/neutral) and class role (tank/healer/dps).
        /// </summary>
        public static PersonalityHint GetPersonalityHint(string simName)
        {
            var hint = new PersonalityHint
            {
                Style = "neutral",
                ClassRole = "dps"
            };

            if (string.IsNullOrEmpty(simName))
                return hint;

            SimPlayerTracking tracking = FindTracking(simName);
            if (tracking == null)
                return hint;

            hint.Style = PersonalityToStyle(tracking.Personality);
            hint.ClassRole = ClassNameToRole(tracking.ClassName);

            return hint;
        }

        /// <summary>
        /// Get full personality data for a sim, including behavioral traits.
        /// Used for template generation requests where richer context is needed.
        /// </summary>
        public static PersonalityData GetPersonalityData(string simName)
        {
            var data = new PersonalityData
            {
                Style = "neutral",
                ClassRole = "dps",
                Traits = new string[0],
                SourceSim = simName ?? ""
            };

            if (string.IsNullOrEmpty(simName))
                return data;

            SimPlayerTracking tracking = FindTracking(simName);
            if (tracking == null)
                return data;

            data.Style = PersonalityToStyle(tracking.Personality);
            data.ClassRole = ClassNameToRole(tracking.ClassName);
            data.Traits = GatherTraits(tracking);

            return data;
        }

        /// <summary>
        /// Find a SimPlayerTracking by name. Returns null if not found.
        /// </summary>
        private static SimPlayerTracking FindTracking(string simName)
        {
            if (GameData.SimMngr == null || GameData.SimMngr.Sims == null)
                return null;

            foreach (SimPlayerTracking sim in GameData.SimMngr.Sims)
            {
                if (sim != null && sim.SimName == simName)
                    return sim;
            }

            return null;
        }

        /// <summary>
        /// Map the game's Personality int to a style string.
        /// Personality values correspond to bio description pools:
        /// 1 = Nice, 2 = Tryhard, 3 = Mean.
        /// </summary>
        private static string PersonalityToStyle(int personality)
        {
            switch (personality)
            {
                case 1: return "nice";
                case 2: return "tryhard";
                case 3: return "mean";
                default: return "neutral";
            }
        }

        /// <summary>
        /// Map a class name to a combat role (tank/healer/dps).
        /// Based on Erenshor class design:
        /// - Paladin: tank/healer hybrid
        /// - Druid: healer/support
        /// - Duelist, Reaver: melee DPS
        /// - Arcanist, Stormcaller: caster DPS
        /// </summary>
        private static string ClassNameToRole(string className)
        {
            if (string.IsNullOrEmpty(className))
                return "dps";

            switch (className)
            {
                case "Paladin": return "tank";
                case "Druid": return "healer";
                case "Duelist": return "dps";
                case "Reaver": return "dps";
                case "Arcanist": return "dps";
                case "Stormcaller": return "dps";
                default: return "dps";
            }
        }

        /// <summary>
        /// Gather behavioral trait strings from SimPlayerTracking fields.
        /// These describe the sim's playstyle tendencies.
        /// </summary>
        private static string[] GatherTraits(SimPlayerTracking tracking)
        {
            var traits = new List<string>();

            if (tracking.LoreChase > 5)
                traits.Add("lore-driven");
            else if (tracking.LoreChase > 0)
                traits.Add("lore-curious");

            if (tracking.GearChase > 5)
                traits.Add("gear-focused");
            else if (tracking.GearChase > 0)
                traits.Add("gear-interested");

            if (tracking.SocialChase > 5)
                traits.Add("social-butterfly");
            else if (tracking.SocialChase > 0)
                traits.Add("sociable");

            if (tracking.Troublemaker > 0)
                traits.Add("troublemaker");

            if (tracking.DedicationLevel > 5)
                traits.Add("dedicated");

            if (tracking.Caution)
                traits.Add("cautious");

            if (tracking.Rival)
                traits.Add("rival");

            if (tracking.IsGMCharacter)
                traits.Add("gm-character");

            return traits.ToArray();
        }
    }
}
