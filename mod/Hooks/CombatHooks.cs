using System.Collections.Generic;
using HarmonyLib;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Harmony patches for combat-related memory events:
    /// - Stats.DoLevelUp() for level-up events (player and group member SimPlayers)
    /// - SetAchievement.Unlock() for achievement events
    /// </summary>
    public static class CombatHooks
    {
        [HarmonyPatch(typeof(Stats), "DoLevelUp")]
        public class DoLevelUpPatch
        {
            static void Postfix(Stats __instance)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                string zone = SceneManager.GetActiveScene().name;
                Character myself = __instance.Myself;

                if (myself == null)
                    return;

                if (!myself.isNPC)
                {
                    // Player level up
                    string text = MemoryEventFormatter.FormatLevelUp("Player", __instance.Level);
                    var metadata = new Dictionary<string, string>
                    {
                        { "level", __instance.Level.ToString() },
                        { "class", __instance.CharacterClass.ToString() }
                    };
                    MemoryReuptakeManager.Instance.QueueEvent(text, "level_up", "Player", zone, metadata);
                }
                else
                {
                    // Only log group member SimPlayer level ups
                    SimPlayer sim = __instance.GetComponent<SimPlayer>();
                    if (sim == null || !sim.InGroup)
                        return;

                    string simName = __instance.MyName;
                    string text = MemoryEventFormatter.FormatLevelUp(simName, __instance.Level);
                    var metadata = new Dictionary<string, string>
                    {
                        { "level", __instance.Level.ToString() }
                    };
                    MemoryReuptakeManager.Instance.QueueEvent(text, "level_up", simName, zone, metadata);
                }
            }
        }

        [HarmonyPatch(typeof(SetAchievement), "Unlock")]
        public class AchievementUnlockPatch
        {
            static void Postfix(string achievementID)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                if (string.IsNullOrEmpty(achievementID))
                    return;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatAchievement(achievementID);
                var metadata = new Dictionary<string, string>
                {
                    { "achievement_id", achievementID }
                };
                MemoryReuptakeManager.Instance.QueueEvent(text, "achievement", "Player", zone, metadata);
            }
        }
    }
}
