using HarmonyLib;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Harmony patches for zone transitions and quest completions:
    /// - SceneChange.ChangeScene() prefix to capture destination zone name
    /// - GameData.FinishQuest() postfix to capture quest completion
    /// </summary>
    public static class ZoneHooks
    {
        [HarmonyPatch(typeof(SceneChange), "ChangeScene")]
        public class ChangeScenePatch
        {
            static void Prefix(string _dest)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                if (string.IsNullOrEmpty(_dest))
                    return;

                string text = MemoryEventFormatter.FormatZoneEnter(_dest);
                // Use the destination zone as the zone context
                MemoryReuptakeManager.Instance.QueueEvent(text, "zone_enter", "Player", _dest);
            }
        }

        [HarmonyPatch(typeof(GameData), "FinishQuest")]
        public class FinishQuestPatch
        {
            static void Postfix(string _questName)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                if (string.IsNullOrEmpty(_questName))
                    return;

                // Get the human-readable quest name from the quest database
                string displayName = _questName;
                Quest quest = GameData.QuestDB != null ? GameData.QuestDB.GetQuestByName(_questName) : null;
                if (quest != null && !string.IsNullOrEmpty(quest.QuestName))
                    displayName = quest.QuestName;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatQuestComplete(displayName);
                var metadata = new System.Collections.Generic.Dictionary<string, string>
                {
                    { "quest_id", _questName },
                    { "quest_name", displayName }
                };
                MemoryReuptakeManager.Instance.QueueEvent(text, "quest_complete", "Player", zone, metadata);
            }
        }
    }
}
