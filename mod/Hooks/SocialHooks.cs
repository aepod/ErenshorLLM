using HarmonyLib;
using UnityEngine.SceneManagement;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Harmony patches for social/group events:
    /// - SimPlayerGrouping.InviteToGroup() for group join
    /// - SimPlayerGrouping.DismissMember1-4() for group leave
    /// - SimPlayerGrouping.ForceDismissFromGroup() for forced leave
    /// - GuildManager.AddPlayerCharacterToGuild() for guild join
    /// - GuildManagerUI.Leave() for guild leave
    /// </summary>
    public static class SocialHooks
    {
        [HarmonyPatch(typeof(SimPlayerGrouping), "InviteToGroup", new System.Type[] { typeof(Character) })]
        public class InviteToGroupPatch
        {
            static void Postfix(SimPlayerGrouping __instance, Character _simPlayer)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                // Determine which character was invited
                Character target = _simPlayer;
                if (target == null)
                    target = GameData.PlayerControl.CurrentTarget;

                if (target == null)
                    return;

                SimPlayer sim = target.GetComponent<SimPlayer>();
                if (sim == null || !sim.InGroup)
                    return;

                string simName = target.transform.name;
                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGroupJoin(simName);
                MemoryReuptakeManager.Instance.QueueEvent(text, "group_join", simName, zone);
            }
        }

        [HarmonyPatch(typeof(SimPlayerGrouping), "DismissMember1")]
        public class DismissMember1Patch
        {
            private static string _simName;
            private static bool _wasAlive;

            static void Prefix()
            {
                _simName = null;
                _wasAlive = true;
                if (GameData.GroupMembers[0] != null && GameData.GroupMembers[0].MyAvatar != null)
                {
                    _simName = GameData.GroupMembers[0].SimName;
                    _wasAlive = GameData.GroupMembers[0].MyAvatar.MyStats.Myself.Alive;
                }
            }

            static void Postfix()
            {
                if (MemoryReuptakeManager.Instance == null || _simName == null)
                    return;

                // If the member was actually dismissed (slot is now null)
                if (GameData.GroupMembers[0] != null)
                    return;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGroupLeave(_simName, _wasAlive);
                MemoryReuptakeManager.Instance.QueueEvent(text, "group_leave", _simName, zone);
            }
        }

        [HarmonyPatch(typeof(SimPlayerGrouping), "DismissMember2")]
        public class DismissMember2Patch
        {
            private static string _simName;
            private static bool _wasAlive;

            static void Prefix()
            {
                _simName = null;
                _wasAlive = true;
                if (GameData.GroupMembers[1] != null && GameData.GroupMembers[1].MyAvatar != null)
                {
                    _simName = GameData.GroupMembers[1].SimName;
                    _wasAlive = GameData.GroupMembers[1].MyAvatar.MyStats.Myself.Alive;
                }
            }

            static void Postfix()
            {
                if (MemoryReuptakeManager.Instance == null || _simName == null)
                    return;

                if (GameData.GroupMembers[1] != null)
                    return;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGroupLeave(_simName, _wasAlive);
                MemoryReuptakeManager.Instance.QueueEvent(text, "group_leave", _simName, zone);
            }
        }

        [HarmonyPatch(typeof(SimPlayerGrouping), "DismissMember3")]
        public class DismissMember3Patch
        {
            private static string _simName;
            private static bool _wasAlive;

            static void Prefix()
            {
                _simName = null;
                _wasAlive = true;
                if (GameData.GroupMembers[2] != null && GameData.GroupMembers[2].MyAvatar != null)
                {
                    _simName = GameData.GroupMembers[2].SimName;
                    _wasAlive = GameData.GroupMembers[2].MyAvatar.MyStats.Myself.Alive;
                }
            }

            static void Postfix()
            {
                if (MemoryReuptakeManager.Instance == null || _simName == null)
                    return;

                if (GameData.GroupMembers[2] != null)
                    return;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGroupLeave(_simName, _wasAlive);
                MemoryReuptakeManager.Instance.QueueEvent(text, "group_leave", _simName, zone);
            }
        }

        [HarmonyPatch(typeof(SimPlayerGrouping), "DismissMember4")]
        public class DismissMember4Patch
        {
            private static string _simName;
            private static bool _wasAlive;

            static void Prefix()
            {
                _simName = null;
                _wasAlive = true;
                if (GameData.GroupMembers[3] != null && GameData.GroupMembers[3].MyAvatar != null)
                {
                    _simName = GameData.GroupMembers[3].SimName;
                    _wasAlive = GameData.GroupMembers[3].MyAvatar.MyStats.Myself.Alive;
                }
            }

            static void Postfix()
            {
                if (MemoryReuptakeManager.Instance == null || _simName == null)
                    return;

                if (GameData.GroupMembers[3] != null)
                    return;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGroupLeave(_simName, _wasAlive);
                MemoryReuptakeManager.Instance.QueueEvent(text, "group_leave", _simName, zone);
            }
        }

        [HarmonyPatch(typeof(SimPlayerGrouping), "ForceDismissFromGroup")]
        public class ForceDismissPatch
        {
            static void Postfix(Character _sim)
            {
                if (MemoryReuptakeManager.Instance == null || _sim == null)
                    return;

                SimPlayer sim = _sim.GetComponent<SimPlayer>();
                if (sim == null)
                    return;

                // If the sim is no longer in group, the dismiss succeeded
                if (sim.InGroup)
                    return;

                string simName = _sim.transform.name;
                bool wasAlive = _sim.Alive;
                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGroupLeave(simName, wasAlive);
                MemoryReuptakeManager.Instance.QueueEvent(text, "group_leave", simName, zone);
            }
        }

        [HarmonyPatch(typeof(GuildManager), "AddPlayerCharacterToGuild")]
        public class GuildJoinPatch
        {
            static void Postfix(string _guildID)
            {
                if (MemoryReuptakeManager.Instance == null)
                    return;

                string guildName = GameData.GuildManager.GetGuildNameByID(_guildID);
                if (string.IsNullOrEmpty(guildName))
                    guildName = _guildID;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGuildJoin(guildName);
                var metadata = new System.Collections.Generic.Dictionary<string, string>
                {
                    { "guild_id", _guildID },
                    { "guild_name", guildName }
                };
                MemoryReuptakeManager.Instance.QueueEvent(text, "guild_join", "Player", zone, metadata);
            }
        }

        [HarmonyPatch(typeof(GuildManagerUI), "Leave")]
        public class GuildLeavePatch
        {
            private static string _guildName;

            static void Prefix()
            {
                _guildName = null;
                // Capture guild name before Leave() clears it
                if (GameData.PlayerControl != null && !string.IsNullOrEmpty(GameData.PlayerControl.MyGuild))
                {
                    _guildName = GameData.GuildManager.GetGuildNameByID(GameData.PlayerControl.MyGuild);
                    if (string.IsNullOrEmpty(_guildName))
                        _guildName = GameData.PlayerControl.MyGuild;
                }
            }

            static void Postfix()
            {
                if (MemoryReuptakeManager.Instance == null || _guildName == null)
                    return;

                string zone = SceneManager.GetActiveScene().name;
                string text = MemoryEventFormatter.FormatGuildLeave(_guildName);
                MemoryReuptakeManager.Instance.QueueEvent(text, "guild_leave", "Player", zone);
            }
        }
    }
}
