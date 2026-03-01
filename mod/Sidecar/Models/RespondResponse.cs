using System;
using System.Collections.Generic;

namespace ErenshorLLMDialog.Sidecar.Models
{
    /// <summary>
    /// Response body from POST /v1/respond.
    /// Parsed manually from JSON since JsonUtility cannot handle arrays of strings
    /// or nested objects with unknown fields well.
    /// </summary>
    public class RespondResponse
    {
        public string response;
        public string template_id;
        public float confidence;
        public string source;
        public string llm_fallback_reason;
        public List<string> lore_context;
        public List<string> memory_context;
        public RespondTiming timing;
        public bool sona_enhanced;

        public RespondResponse()
        {
            lore_context = new List<string>();
            memory_context = new List<string>();
        }

        /// <summary>
        /// Simple JSON parser for the respond response.
        /// This is a minimal parser that handles the known response structure.
        /// </summary>
        public static RespondResponse FromJson(string json)
        {
            if (string.IsNullOrEmpty(json))
                return null;

            var resp = new RespondResponse();

            resp.response = ExtractStringField(json, "response");
            resp.template_id = ExtractStringField(json, "template_id");
            resp.confidence = ExtractFloatField(json, "confidence");
            resp.source = ExtractStringField(json, "source");
            resp.llm_fallback_reason = ExtractStringField(json, "llm_fallback_reason");
            resp.lore_context = ExtractStringArray(json, "lore_context");
            resp.memory_context = ExtractStringArray(json, "memory_context");
            resp.sona_enhanced = ExtractBoolField(json, "sona_enhanced");

            // Parse timing if present
            resp.timing = new RespondTiming();
            int timingIdx = json.IndexOf("\"timing\"", StringComparison.Ordinal);
            if (timingIdx >= 0)
            {
                int braceStart = json.IndexOf('{', timingIdx);
                if (braceStart >= 0)
                {
                    int braceEnd = json.IndexOf('}', braceStart);
                    if (braceEnd >= 0)
                    {
                        string timingJson = json.Substring(braceStart, braceEnd - braceStart + 1);
                        resp.timing.embed_ms = (long)ExtractFloatField(timingJson, "embed_ms");
                        resp.timing.sona_transform_ms = (long)ExtractFloatField(timingJson, "sona_transform_ms");
                        resp.timing.template_search_ms = (long)ExtractFloatField(timingJson, "template_search_ms");
                        resp.timing.rerank_ms = (long)ExtractFloatField(timingJson, "rerank_ms");
                        resp.timing.lore_search_ms = (long)ExtractFloatField(timingJson, "lore_search_ms");
                        resp.timing.memory_search_ms = (long)ExtractFloatField(timingJson, "memory_search_ms");
                        resp.timing.llm_ms = (long)ExtractFloatField(timingJson, "llm_ms");
                        resp.timing.total_ms = (long)ExtractFloatField(timingJson, "total_ms");
                    }
                }
            }

            return resp;
        }

        private static string ExtractStringField(string json, string field)
        {
            string key = "\"" + field + "\"";
            int idx = json.IndexOf(key, StringComparison.Ordinal);
            if (idx < 0) return "";

            int colonIdx = json.IndexOf(':', idx + key.Length);
            if (colonIdx < 0) return "";

            // Skip whitespace after colon
            int start = colonIdx + 1;
            while (start < json.Length && (json[start] == ' ' || json[start] == '\t'))
                start++;

            if (start >= json.Length || json[start] != '"')
                return "";

            start++; // skip opening quote
            var sb = new System.Text.StringBuilder();
            for (int i = start; i < json.Length; i++)
            {
                if (json[i] == '\\' && i + 1 < json.Length)
                {
                    i++;
                    switch (json[i])
                    {
                        case '"': sb.Append('"'); break;
                        case '\\': sb.Append('\\'); break;
                        case 'n': sb.Append('\n'); break;
                        case 'r': sb.Append('\r'); break;
                        case 't': sb.Append('\t'); break;
                        default: sb.Append(json[i]); break;
                    }
                }
                else if (json[i] == '"')
                {
                    break;
                }
                else
                {
                    sb.Append(json[i]);
                }
            }
            return sb.ToString();
        }

        private static bool ExtractBoolField(string json, string field)
        {
            string key = "\"" + field + "\"";
            int idx = json.IndexOf(key, StringComparison.Ordinal);
            if (idx < 0) return false;

            int colonIdx = json.IndexOf(':', idx + key.Length);
            if (colonIdx < 0) return false;

            int start = colonIdx + 1;
            while (start < json.Length && (json[start] == ' ' || json[start] == '\t'))
                start++;

            if (start + 4 <= json.Length && json.Substring(start, 4) == "true")
                return true;

            return false;
        }

        private static float ExtractFloatField(string json, string field)
        {
            string key = "\"" + field + "\"";
            int idx = json.IndexOf(key, StringComparison.Ordinal);
            if (idx < 0) return 0f;

            int colonIdx = json.IndexOf(':', idx + key.Length);
            if (colonIdx < 0) return 0f;

            int start = colonIdx + 1;
            while (start < json.Length && (json[start] == ' ' || json[start] == '\t'))
                start++;

            int end = start;
            while (end < json.Length && (char.IsDigit(json[end]) || json[end] == '.' || json[end] == '-'))
                end++;

            string numStr = json.Substring(start, end - start);
            if (float.TryParse(numStr, System.Globalization.NumberStyles.Float,
                System.Globalization.CultureInfo.InvariantCulture, out float result))
                return result;

            return 0f;
        }

        private static List<string> ExtractStringArray(string json, string field)
        {
            var result = new List<string>();
            string key = "\"" + field + "\"";
            int idx = json.IndexOf(key, StringComparison.Ordinal);
            if (idx < 0) return result;

            int colonIdx = json.IndexOf(':', idx + key.Length);
            if (colonIdx < 0) return result;

            int bracketStart = json.IndexOf('[', colonIdx);
            if (bracketStart < 0) return result;

            int bracketEnd = json.IndexOf(']', bracketStart);
            if (bracketEnd < 0) return result;

            string arrContent = json.Substring(bracketStart + 1, bracketEnd - bracketStart - 1);

            // Parse string elements from the array
            int pos = 0;
            while (pos < arrContent.Length)
            {
                int quoteStart = arrContent.IndexOf('"', pos);
                if (quoteStart < 0) break;

                var sb = new System.Text.StringBuilder();
                int i = quoteStart + 1;
                while (i < arrContent.Length)
                {
                    if (arrContent[i] == '\\' && i + 1 < arrContent.Length)
                    {
                        i++;
                        switch (arrContent[i])
                        {
                            case '"': sb.Append('"'); break;
                            case '\\': sb.Append('\\'); break;
                            case 'n': sb.Append('\n'); break;
                            default: sb.Append(arrContent[i]); break;
                        }
                    }
                    else if (arrContent[i] == '"')
                    {
                        break;
                    }
                    else
                    {
                        sb.Append(arrContent[i]);
                    }
                    i++;
                }
                result.Add(sb.ToString());
                pos = i + 1;
            }

            return result;
        }
    }

    public class RespondTiming
    {
        public long embed_ms;
        public long sona_transform_ms;
        public long template_search_ms;
        public long rerank_ms;
        public long lore_search_ms;
        public long memory_search_ms;
        public long llm_ms;
        public long total_ms;
    }
}
