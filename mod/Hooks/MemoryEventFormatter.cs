namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Static methods that format game events as short, factual natural-language sentences
    /// for ingestion into the sidecar memory system.
    /// </summary>
    public static class MemoryEventFormatter
    {
        public static string FormatDeath(string characterName, string killerName, string zone, bool isBoss, bool isGroupMember)
        {
            string prefix = isGroupMember ? "Group member " : "";
            string suffix = isBoss ? " (boss)" : "";

            if (!string.IsNullOrEmpty(killerName))
                return prefix + characterName + " was slain by " + killerName + suffix + " in " + zone;

            return prefix + characterName + " was slain in " + zone;
        }

        public static string FormatLevelUp(string name, int newLevel)
        {
            if (name == "Player" || string.IsNullOrEmpty(name))
                return "You reached level " + newLevel + "!";

            return name + " reached level " + newLevel;
        }

        public static string FormatQuestComplete(string questName)
        {
            return "Completed quest: " + questName;
        }

        public static string FormatZoneEnter(string zoneName)
        {
            return "Entered " + zoneName;
        }

        public static string FormatGroupJoin(string simName)
        {
            return simName + " joined the group";
        }

        public static string FormatGroupLeave(string simName, bool wasAlive)
        {
            if (wasAlive)
                return simName + " left the group";

            return simName + " was dismissed while dead";
        }

        public static string FormatLoot(string itemName, int quality)
        {
            string qualityPrefix = "";
            if (quality >= 3)
                qualityPrefix = "Blessed ";
            else if (quality >= 2)
                qualityPrefix = "Sparkling ";

            return "Looted " + qualityPrefix + itemName;
        }

        public static string FormatAchievement(string name)
        {
            return "Unlocked achievement: " + name;
        }

        public static string FormatGuildJoin(string guildName)
        {
            return "Joined " + guildName;
        }

        public static string FormatGuildLeave(string guildName)
        {
            return "Left " + guildName;
        }

        public static string FormatWipe(string zone)
        {
            return "The group wiped in " + zone + " after multiple deaths";
        }
    }
}
