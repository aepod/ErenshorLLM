using System;
using System.Collections.Generic;

namespace ErenshorLLMDialog.Sidecar.Models
{
    /// <summary>
    /// Request body for POST /v1/respond.
    /// JsonUtility requires [Serializable] and public fields (no properties).
    /// Personality is a Dictionary which JsonUtility cannot serialize, so we
    /// build the JSON manually in SidecarClient.
    /// </summary>
    [Serializable]
    public class RespondRequest
    {
        public string player_message;
        public string channel;
        public string sim_name;
        public Dictionary<string, bool> personality;
        public string zone;
        public float relationship;
        public string player_name;
        public int player_level;
        public string player_class;
        public string player_guild;
        public string sim_guild;
        public bool sim_is_rival;
        public List<string> group_members;

        // ── Optional overrides (sent to sidecar to override its config defaults) ──

        /// <summary>
        /// Number of template candidates to retrieve before re-ranking.
        /// Null means use sidecar default.
        /// </summary>
        public int? template_candidates;

        /// <summary>
        /// Number of lore passages to retrieve for context enrichment.
        /// Null means use sidecar default.
        /// </summary>
        public int? lore_context_count;

        /// <summary>
        /// Number of memory entries to retrieve.
        /// Null means use sidecar default.
        /// </summary>
        public int? memory_context_count;

        /// <summary>
        /// Re-ranking weight for semantic similarity.
        /// Null means use sidecar default.
        /// </summary>
        public float? w_semantic;

        /// <summary>
        /// Re-ranking weight for channel match.
        /// Null means use sidecar default.
        /// </summary>
        public float? w_channel;

        /// <summary>
        /// Re-ranking weight for zone affinity.
        /// Null means use sidecar default.
        /// </summary>
        public float? w_zone;

        /// <summary>
        /// Re-ranking weight for personality trait matching.
        /// Null means use sidecar default.
        /// </summary>
        public float? w_personality;

        /// <summary>
        /// Re-ranking weight for relationship level.
        /// Null means use sidecar default.
        /// </summary>
        public float? w_relationship;

        public RespondRequest()
        {
            personality = new Dictionary<string, bool>();
            group_members = new List<string>();
            relationship = 5.0f;
        }

        /// <summary>
        /// Manually builds JSON since JsonUtility cannot handle Dictionary or List of strings properly.
        /// Optional override fields are only included when they have a value (non-null).
        /// </summary>
        public string ToJson()
        {
            var sb = new System.Text.StringBuilder(512);
            sb.Append('{');

            sb.Append("\"player_message\":\"").Append(EscapeJson(player_message ?? "")).Append("\",");
            sb.Append("\"channel\":\"").Append(EscapeJson(channel ?? "")).Append("\",");
            sb.Append("\"sim_name\":\"").Append(EscapeJson(sim_name ?? "")).Append("\",");

            // personality object
            sb.Append("\"personality\":{");
            if (personality != null && personality.Count > 0)
            {
                bool first = true;
                foreach (var kv in personality)
                {
                    if (!first) sb.Append(',');
                    sb.Append('"').Append(EscapeJson(kv.Key)).Append("\":").Append(kv.Value ? "true" : "false");
                    first = false;
                }
            }
            sb.Append("},");

            sb.Append("\"zone\":\"").Append(EscapeJson(zone ?? "")).Append("\",");
            sb.Append("\"relationship\":").Append(relationship.ToString("F1", System.Globalization.CultureInfo.InvariantCulture)).Append(',');
            sb.Append("\"player_name\":\"").Append(EscapeJson(player_name ?? "")).Append("\",");
            sb.Append("\"player_level\":").Append(player_level).Append(',');
            sb.Append("\"player_class\":\"").Append(EscapeJson(player_class ?? "")).Append("\",");
            sb.Append("\"player_guild\":\"").Append(EscapeJson(player_guild ?? "")).Append("\",");
            sb.Append("\"sim_guild\":\"").Append(EscapeJson(sim_guild ?? "")).Append("\",");
            sb.Append("\"sim_is_rival\":").Append(sim_is_rival ? "true" : "false").Append(',');

            // group_members array
            sb.Append("\"group_members\":[");
            if (group_members != null && group_members.Count > 0)
            {
                for (int i = 0; i < group_members.Count; i++)
                {
                    if (i > 0) sb.Append(',');
                    sb.Append('"').Append(EscapeJson(group_members[i])).Append('"');
                }
            }
            sb.Append(']');

            // Optional override fields -- only include when non-null
            if (template_candidates.HasValue)
            {
                sb.Append(",\"template_candidates\":").Append(template_candidates.Value);
            }
            if (lore_context_count.HasValue)
            {
                sb.Append(",\"lore_context_count\":").Append(lore_context_count.Value);
            }
            if (memory_context_count.HasValue)
            {
                sb.Append(",\"memory_context_count\":").Append(memory_context_count.Value);
            }
            if (w_semantic.HasValue)
            {
                sb.Append(",\"w_semantic\":").Append(w_semantic.Value.ToString("F4", System.Globalization.CultureInfo.InvariantCulture));
            }
            if (w_channel.HasValue)
            {
                sb.Append(",\"w_channel\":").Append(w_channel.Value.ToString("F4", System.Globalization.CultureInfo.InvariantCulture));
            }
            if (w_zone.HasValue)
            {
                sb.Append(",\"w_zone\":").Append(w_zone.Value.ToString("F4", System.Globalization.CultureInfo.InvariantCulture));
            }
            if (w_personality.HasValue)
            {
                sb.Append(",\"w_personality\":").Append(w_personality.Value.ToString("F4", System.Globalization.CultureInfo.InvariantCulture));
            }
            if (w_relationship.HasValue)
            {
                sb.Append(",\"w_relationship\":").Append(w_relationship.Value.ToString("F4", System.Globalization.CultureInfo.InvariantCulture));
            }

            sb.Append('}');
            return sb.ToString();
        }

        private static string EscapeJson(string s)
        {
            if (string.IsNullOrEmpty(s)) return "";
            return s.Replace("\\", "\\\\")
                    .Replace("\"", "\\\"")
                    .Replace("\n", "\\n")
                    .Replace("\r", "\\r")
                    .Replace("\t", "\\t");
        }
    }
}
