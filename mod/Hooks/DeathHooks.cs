using System.Collections.Generic;
using System.Reflection;
using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Harmony patch on Character.DoDeath() to capture death events.
    /// DoDeath is a private method, so we use TargetMethod() with AccessTools.
    /// Tracks rapid group member deaths to detect wipes.
    /// </summary>
    [HarmonyPatch]
    public class DeathHooks
    {
        /// <summary>Recent group member death timestamps for wipe detection.</summary>
        private static readonly List<float> _recentGroupDeaths = new List<float>();

        /// <summary>Time window in seconds to count deaths for wipe detection.</summary>
        private const float WIPE_WINDOW = 30f;

        /// <summary>Number of deaths in the window to trigger a wipe event.</summary>
        private const int WIPE_THRESHOLD = 3;

        static MethodBase TargetMethod()
        {
            return AccessTools.Method(typeof(Character), "DoDeath");
        }

        static void Postfix(Character __instance)
        {
            if (MemoryReuptakeManager.Instance == null)
                return;

            string zone = SceneManager.GetActiveScene().name;

            if (__instance.isNPC)
            {
                HandleNPCDeath(__instance, zone);
            }
            else
            {
                HandlePlayerDeath(__instance, zone);
            }
        }

        private static void HandleNPCDeath(Character character, string zone)
        {
            // Only care about deaths that involve the player or group members
            NPC npc = character.GetComponent<NPC>();
            if (npc == null)
                return;

            string npcName = character.transform.name;
            bool isBoss = character.BossXp > 1f || (npc != null && npc.GroupEncounter);

            // Check if this NPC is a group member SimPlayer that died
            if (npc.SimPlayer && npc.InGroup)
            {
                HandleGroupMemberDeath(character, npcName, zone);
                return;
            }

            // For enemy NPC deaths, only log boss kills or kills by the player
            if (isBoss && !character.Alive)
            {
                string killerName = "";
                if (character.LastHitBy != null)
                    killerName = character.LastHitBy.transform.name;

                string text = MemoryEventFormatter.FormatDeath(npcName, killerName, zone, true, false);
                var metadata = new Dictionary<string, string>
                {
                    { "is_boss", "true" },
                    { "npc_level", character.MyStats != null ? character.MyStats.Level.ToString() : "" }
                };
                MemoryReuptakeManager.Instance.QueueEvent(text, "boss_kill", npcName, zone, metadata);
            }
        }

        private static void HandleGroupMemberDeath(Character character, string simName, string zone)
        {
            string killerName = "";
            if (character.LastHitBy != null)
                killerName = character.LastHitBy.transform.name;

            string text = MemoryEventFormatter.FormatDeath(simName, killerName, zone, false, true);
            MemoryReuptakeManager.Instance.QueueEvent(text, "group_member_death", simName, zone);

            // Track for wipe detection
            float now = Time.realtimeSinceStartup;
            _recentGroupDeaths.Add(now);

            // Clean old entries
            _recentGroupDeaths.RemoveAll(t => now - t > WIPE_WINDOW);

            if (_recentGroupDeaths.Count >= WIPE_THRESHOLD)
            {
                string wipeText = MemoryEventFormatter.FormatWipe(zone);
                MemoryReuptakeManager.Instance.QueueEvent(wipeText, "wipe", "", zone);
                _recentGroupDeaths.Clear();
            }
        }

        private static void HandlePlayerDeath(Character character, string zone)
        {
            string killerName = "";
            if (character.LastHitBy != null)
                killerName = character.LastHitBy.transform.name;

            string text = MemoryEventFormatter.FormatDeath("You", killerName, zone, false, false);
            MemoryReuptakeManager.Instance.QueueEvent(text, "player_death", "Player", zone);

            // Player death also counts toward wipe detection
            float now = Time.realtimeSinceStartup;
            _recentGroupDeaths.Add(now);
            _recentGroupDeaths.RemoveAll(t => now - t > WIPE_WINDOW);

            if (_recentGroupDeaths.Count >= WIPE_THRESHOLD)
            {
                string wipeText = MemoryEventFormatter.FormatWipe(zone);
                MemoryReuptakeManager.Instance.QueueEvent(wipeText, "wipe", "", zone);
                _recentGroupDeaths.Clear();
            }
        }
    }
}
