using System;

namespace ErenshorLLMDialog.Sidecar.Models
{
    /// <summary>
    /// Response body from POST /v1/paraphrase.
    /// </summary>
    [Serializable]
    public class ParaphraseResponse
    {
        public string text;
        public string original;
        public bool paraphrased;
        public string source;

        public static ParaphraseResponse FromJson(string json)
        {
            if (string.IsNullOrEmpty(json))
                return null;

            var resp = new ParaphraseResponse();
            resp.text = ExtractStringField(json, "text");
            resp.original = ExtractStringField(json, "original");
            resp.paraphrased = ExtractBoolField(json, "paraphrased");
            resp.source = ExtractStringField(json, "source");
            return resp;
        }

        private static string ExtractStringField(string json, string field)
        {
            string key = "\"" + field + "\"";
            int idx = json.IndexOf(key, StringComparison.Ordinal);
            if (idx < 0) return "";

            int colonIdx = json.IndexOf(':', idx + key.Length);
            if (colonIdx < 0) return "";

            int start = colonIdx + 1;
            while (start < json.Length && (json[start] == ' ' || json[start] == '\t'))
                start++;

            if (start >= json.Length || json[start] != '"')
                return "";

            start++;
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
    }
}
