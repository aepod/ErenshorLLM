using HarmonyLib;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Harmony patch on LootWindow.LootAll() to capture notable loot events.
    /// Only logs items of quality >= 2 (sparkling/blue or better).
    /// Uses a Prefix to snapshot loot slot contents before LootAll clears them.
    /// </summary>
    public static class LootHooks
    {
        [HarmonyPatch(typeof(LootWindow), "LootAll")]
        public class LootAllPatch
        {
            /// <summary>
            /// Prefix captures items before LootAll() processes and clears them.
            /// Stores notable items in __state for the Postfix to consume.
            /// </summary>
            static void Prefix(LootWindow __instance, out System.Collections.Generic.List<LootSnapshot> __state)
            {
                __state = new System.Collections.Generic.List<LootSnapshot>();

                if (MemoryReuptakeManager.Instance == null)
                    return;

                if (__instance.LootSlots == null)
                    return;

                foreach (ItemIcon slot in __instance.LootSlots)
                {
                    if (slot.MyItem == null)
                        continue;

                    // GameData.PlayerInv.Empty is the sentinel for an empty slot
                    if (GameData.PlayerInv != null && slot.MyItem == GameData.PlayerInv.Empty)
                        continue;

                    // Only log quality >= 2 (sparkling or better)
                    if (slot.Quantity >= 2)
                    {
                        __state.Add(new LootSnapshot
                        {
                            ItemName = slot.MyItem.ItemName,
                            Quality = slot.Quantity
                        });
                    }
                }
            }

            /// <summary>
            /// Postfix queues memory events for each notable item that was looted.
            /// </summary>
            static void Postfix(System.Collections.Generic.List<LootSnapshot> __state)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                if (__state == null || __state.Count == 0)
                    return;

                string zone = SceneManager.GetActiveScene().name;

                foreach (var item in __state)
                {
                    string text = MemoryEventFormatter.FormatLoot(item.ItemName, item.Quality);
                    var metadata = new System.Collections.Generic.Dictionary<string, string>
                    {
                        { "item_name", item.ItemName },
                        { "quality", item.Quality.ToString() }
                    };
                    MemoryReuptakeManager.Instance.QueueEvent(text, "loot", "Player", zone, metadata);
                }
            }
        }

        /// <summary>
        /// Snapshot of a loot item captured in the Prefix for use in the Postfix.
        /// </summary>
        public struct LootSnapshot
        {
            public string ItemName;
            public int Quality;
        }
    }
}
