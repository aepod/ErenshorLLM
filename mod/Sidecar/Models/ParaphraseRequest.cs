using System;
using System.Collections.Generic;

namespace ErenshorLLMDialog.Sidecar.Models
{
    /// <summary>
    /// Request body for POST /v1/paraphrase.
    /// Sends canned game text to the sidecar for LLM-enriched paraphrasing
    /// with personality voice, lore context, and GEPA grounding.
    /// </summary>
    [Serializable]
    public class ParaphraseRequest
    {
        public string text;
        public string trigger;
        public string sim_name;
        public string zone;
        public string channel;
        public float relationship;
        public string player_name;
        public Dictionary<string, string> context;

        public ParaphraseRequest()
        {
            context = new Dictionary<string, string>();
            relationship = 5.0f;
            trigger = "generic";
            channel = "say";
        }

        public string ToJson()
        {
            var sb = new System.Text.StringBuilder(512);
            sb.Append('{');

            sb.Append("\"text\":\"").Append(EscapeJson(text ?? "")).Append("\",");
            sb.Append("\"trigger\":\"").Append(EscapeJson(trigger ?? "generic")).Append("\",");
            sb.Append("\"sim_name\":\"").Append(EscapeJson(sim_name ?? "")).Append("\",");
            sb.Append("\"zone\":\"").Append(EscapeJson(zone ?? "")).Append("\",");
            sb.Append("\"channel\":\"").Append(EscapeJson(channel ?? "say")).Append("\",");
            sb.Append("\"relationship\":").Append(relationship.ToString("F1",
                System.Globalization.CultureInfo.InvariantCulture)).Append(',');
            sb.Append("\"player_name\":\"").Append(EscapeJson(player_name ?? "")).Append("\",");

            // context object
            sb.Append("\"context\":{");
            if (context != null && context.Count > 0)
            {
                bool first = true;
                foreach (var kv in context)
                {
                    if (!first) sb.Append(',');
                    sb.Append('"').Append(EscapeJson(kv.Key)).Append("\":\"")
                      .Append(EscapeJson(kv.Value)).Append('"');
                    first = false;
                }
            }
            sb.Append('}');

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
