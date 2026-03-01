using System;

namespace ErenshorLLMDialog.Sidecar.Models
{
    /// <summary>
    /// Response body from GET /health.
    /// We only parse the fields we need for health checking.
    /// </summary>
    public class HealthResponse
    {
        public string status;
        public string version;
        public long uptime_seconds;
        public bool embedding_model_loaded;

        public bool IsReady => status == "ready";

        /// <summary>
        /// Minimal parser for health response -- we only need the status field.
        /// </summary>
        public static HealthResponse FromJson(string json)
        {
            if (string.IsNullOrEmpty(json))
                return null;

            var resp = new HealthResponse();
            resp.status = ExtractStringField(json, "status");
            resp.version = ExtractStringField(json, "version");
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
            int end = json.IndexOf('"', start);
            if (end < 0) return "";

            return json.Substring(start, end - start);
        }
    }
}
