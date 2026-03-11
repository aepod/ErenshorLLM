using System;
using System.Collections;
using System.Collections.Generic;
using BepInEx.Logging;
using ErenshorLLMDialog.Pipeline;
using ErenshorLLMDialog.Sidecar.Models;
using UnityEngine;
using UnityEngine.Networking;

namespace ErenshorLLMDialog.Sidecar
{
    /// <summary>
    /// HTTP client for communicating with the Rust sidecar.
    /// Uses UnityWebRequest (not System.Net.HttpClient) for Mono/Unity compatibility.
    /// All methods return coroutines that must be started on a MonoBehaviour.
    /// </summary>
    public class SidecarClient
    {
        private readonly string _baseUrl;
        private readonly ManualLogSource _log;

        // Timeout in seconds for each endpoint
        private const int HEALTH_TIMEOUT = 2;
        private const int SHUTDOWN_TIMEOUT = 1;
        private const int INGEST_TIMEOUT = 3;

        // Respond timeout is configurable since LLM generation can take time
        private readonly int _respondTimeout;

        public SidecarClient(int port, ManualLogSource log, float respondTimeoutSeconds = 15f)
        {
            _baseUrl = "http://127.0.0.1:" + port;
            _log = log;
            _respondTimeout = (int)System.Math.Ceiling(respondTimeoutSeconds);
        }

        /// <summary>
        /// Sends GET /health and invokes the callback with the parsed response (or null on failure).
        /// </summary>
        public IEnumerator HealthCheck(Action<HealthResponse> callback)
        {
            string url = _baseUrl + "/health";
            using (var request = UnityWebRequest.Get(url))
            {
                request.timeout = HEALTH_TIMEOUT;

                yield return request.SendWebRequest();

                if (IsRequestError(request))
                {
                    callback?.Invoke(null);
                    yield break;
                }

                string json = request.downloadHandler.text;
                HealthResponse resp = null;
                try
                {
                    resp = HealthResponse.FromJson(json);
                }
                catch (Exception e)
                {
                    _log.LogWarning("[SidecarClient] Failed to parse health response: " + e.Message);
                }
                callback?.Invoke(resp);
            }
        }

        /// <summary>
        /// Sends POST /v1/respond with the given request body.
        /// Invokes the callback with the parsed response (or null on failure).
        /// Also reports round-trip latency in milliseconds.
        /// </summary>
        public IEnumerator Respond(RespondRequest req, Action<RespondResponse, long> callback)
        {
            string url = _baseUrl + "/v1/respond";
            string body = req.ToJson();
            float startTime = Time.realtimeSinceStartup;

            using (var request = new UnityWebRequest(url, "POST"))
            {
                byte[] bodyBytes = System.Text.Encoding.UTF8.GetBytes(body);
                request.uploadHandler = new UploadHandlerRaw(bodyBytes);
                request.downloadHandler = new DownloadHandlerBuffer();
                request.SetRequestHeader("Content-Type", "application/json");
                request.timeout = _respondTimeout;

                yield return request.SendWebRequest();

                long latencyMs = (long)((Time.realtimeSinceStartup - startTime) * 1000f);

                if (IsRequestError(request))
                {
                    _log.LogWarning("[SidecarClient] /v1/respond failed (" + latencyMs +
                        "ms): " + request.error +
                        (request.downloadHandler != null ? " body=" + request.downloadHandler.text : ""));
                    callback?.Invoke(null, latencyMs);
                    yield break;
                }

                string json = request.downloadHandler.text;
                RespondResponse resp = null;
                try
                {
                    resp = RespondResponse.FromJson(json);
                }
                catch (Exception e)
                {
                    _log.LogWarning("[SidecarClient] Failed to parse respond response: " + e.Message);
                }
                callback?.Invoke(resp, latencyMs);
            }
        }

        /// <summary>
        /// Sends POST /v1/paraphrase with the given request body.
        /// Invokes the callback with the parsed response (or null on failure).
        /// </summary>
        public IEnumerator Paraphrase(ParaphraseRequest req, Action<ParaphraseResponse, long> callback)
        {
            string url = _baseUrl + "/v1/paraphrase";
            string body = req.ToJson();
            float startTime = Time.realtimeSinceStartup;

            using (var request = new UnityWebRequest(url, "POST"))
            {
                byte[] bodyBytes = System.Text.Encoding.UTF8.GetBytes(body);
                request.uploadHandler = new UploadHandlerRaw(bodyBytes);
                request.downloadHandler = new DownloadHandlerBuffer();
                request.SetRequestHeader("Content-Type", "application/json");
                request.timeout = _respondTimeout;

                yield return request.SendWebRequest();

                long latencyMs = (long)((Time.realtimeSinceStartup - startTime) * 1000f);

                if (IsRequestError(request))
                {
                    _log.LogWarning("[SidecarClient] /v1/paraphrase failed (" + latencyMs +
                        "ms): " + request.error);
                    callback?.Invoke(null, latencyMs);
                    yield break;
                }

                string json = request.downloadHandler.text;
                ParaphraseResponse resp = null;
                try
                {
                    resp = ParaphraseResponse.FromJson(json);
                }
                catch (Exception e)
                {
                    _log.LogWarning("[SidecarClient] Failed to parse paraphrase response: " + e.Message);
                }
                callback?.Invoke(resp, latencyMs);
            }
        }

        /// <summary>
        /// Sends POST /shutdown to initiate graceful sidecar shutdown.
        /// Invokes the callback with true on success, false on failure.
        /// </summary>
        public IEnumerator Shutdown(Action<bool> callback)
        {
            string url = _baseUrl + "/shutdown";

            using (var request = new UnityWebRequest(url, "POST"))
            {
                request.downloadHandler = new DownloadHandlerBuffer();
                request.SetRequestHeader("Content-Type", "application/json");
                request.timeout = SHUTDOWN_TIMEOUT;

                yield return request.SendWebRequest();

                bool success = !IsRequestError(request);
                if (!success)
                {
                    _log.LogWarning("[SidecarClient] /shutdown failed: " + request.error);
                }
                callback?.Invoke(success);
            }
        }

        /// <summary>
        /// Sends POST /v1/rag/ingest with a memory event for ingestion into the RAG system.
        /// Invokes the callback with true on success, false on failure.
        /// </summary>
        public IEnumerator IngestMemory(
            string text, string eventType, string simName, string zone,
            Dictionary<string, string> extraMetadata = null,
            Action<bool> callback = null)
        {
            string url = _baseUrl + "/v1/rag/ingest";

            var metadata = new Dictionary<string, string>
            {
                { "event_type", eventType ?? "" },
                { "sim_name", simName ?? "" },
                { "zone", zone ?? "" },
                { "timestamp", DateTime.UtcNow.ToString("o") }
            };

            if (extraMetadata != null)
            {
                foreach (var kvp in extraMetadata)
                {
                    if (!metadata.ContainsKey(kvp.Key))
                        metadata[kvp.Key] = kvp.Value;
                }
            }

            string body = BuildIngestJson(text, metadata);

            using (var request = new UnityWebRequest(url, "POST"))
            {
                byte[] bodyBytes = System.Text.Encoding.UTF8.GetBytes(body);
                request.uploadHandler = new UploadHandlerRaw(bodyBytes);
                request.downloadHandler = new DownloadHandlerBuffer();
                request.SetRequestHeader("Content-Type", "application/json");
                request.timeout = INGEST_TIMEOUT;

                yield return request.SendWebRequest();

                bool success = !IsRequestError(request);
                if (!success)
                {
                    _log.LogDebug("[SidecarClient] /v1/rag/ingest failed: " + request.error);
                }
                callback?.Invoke(success);
            }
        }

        /// <summary>
        /// Builds the JSON body for the /v1/rag/ingest endpoint.
        /// Constructs JSON manually to avoid Newtonsoft dependency.
        /// </summary>
        private static string BuildIngestJson(string text, Dictionary<string, string> metadata)
        {
            var sb = new System.Text.StringBuilder();
            sb.Append("{\"text\":\"");
            sb.Append(EscapeJsonString(text));
            sb.Append("\",\"collection\":\"memory\",\"metadata\":{");

            bool first = true;
            foreach (var kvp in metadata)
            {
                if (!first)
                    sb.Append(",");
                sb.Append("\"");
                sb.Append(EscapeJsonString(kvp.Key));
                sb.Append("\":\"");
                sb.Append(EscapeJsonString(kvp.Value));
                sb.Append("\"");
                first = false;
            }

            sb.Append("}}");
            return sb.ToString();
        }

        // ---- Template API methods (M3/M4) ----

        private const int TEMPLATE_TIMEOUT = 5;

        /// <summary>
        /// GET /v1/templates/lookup -- Look up a template variant by trigger and personality.
        /// Invokes the callback with a TemplateLookupResult (or null on failure).
        /// </summary>
        public IEnumerator LookupTemplate(string trigger, string personalityStyle,
            string personalityClassRole, Action<TemplateLookupResult> callback)
        {
            string url = _baseUrl + "/v1/templates/lookup?trigger=" +
                UnityWebRequest.EscapeURL(trigger);
            if (!string.IsNullOrEmpty(personalityStyle))
                url += "&personality_style=" + UnityWebRequest.EscapeURL(personalityStyle);
            if (!string.IsNullOrEmpty(personalityClassRole))
                url += "&personality_class_role=" + UnityWebRequest.EscapeURL(personalityClassRole);

            using (var request = UnityWebRequest.Get(url))
            {
                request.timeout = TEMPLATE_TIMEOUT;

                yield return request.SendWebRequest();

                if (IsRequestError(request))
                {
                    callback?.Invoke(null);
                    yield break;
                }

                string json = request.downloadHandler.text;
                try
                {
                    var result = ParseLookupResponse(json);
                    callback?.Invoke(result);
                }
                catch (Exception e)
                {
                    _log.LogDebug("[SidecarClient] Template lookup parse error: " + e.Message);
                    callback?.Invoke(null);
                }
            }
        }

        /// <summary>
        /// GET /v1/templates/stats -- Get template store statistics.
        /// Invokes the callback with a dictionary of stat key-value pairs (or null on failure).
        /// </summary>
        public IEnumerator GetTemplateStats(Action<Dictionary<string, string>> callback)
        {
            string url = _baseUrl + "/v1/templates/stats";

            using (var request = UnityWebRequest.Get(url))
            {
                request.timeout = TEMPLATE_TIMEOUT;

                yield return request.SendWebRequest();

                if (IsRequestError(request))
                {
                    callback?.Invoke(null);
                    yield break;
                }

                string json = request.downloadHandler.text;
                try
                {
                    var stats = ParseStatsResponse(json);
                    callback?.Invoke(stats);
                }
                catch (Exception e)
                {
                    _log.LogDebug("[SidecarClient] Template stats parse error: " + e.Message);
                    callback?.Invoke(null);
                }
            }
        }

        /// <summary>
        /// POST /v1/templates/queue -- Queue a template generation request.
        /// Fire-and-forget with completion callbacks.
        /// </summary>
        public IEnumerator QueueTemplateGeneration(string trigger, string originalText,
            string channel, PersonalityData personality,
            Action onComplete = null, Action onError = null)
        {
            string url = _baseUrl + "/v1/templates/queue";
            string body = BuildQueueJson(trigger, originalText, channel, personality);

            using (var request = new UnityWebRequest(url, "POST"))
            {
                byte[] bodyBytes = System.Text.Encoding.UTF8.GetBytes(body);
                request.uploadHandler = new UploadHandlerRaw(bodyBytes);
                request.downloadHandler = new DownloadHandlerBuffer();
                request.SetRequestHeader("Content-Type", "application/json");
                request.timeout = TEMPLATE_TIMEOUT;

                yield return request.SendWebRequest();

                if (IsRequestError(request))
                {
                    _log.LogDebug("[SidecarClient] Template queue failed: " + request.error);
                    onError?.Invoke();
                    yield break;
                }

                onComplete?.Invoke();
            }
        }

        /// <summary>
        /// Parse the JSON response from /v1/templates/lookup.
        /// Manual parsing to avoid Newtonsoft dependency.
        /// </summary>
        private static TemplateLookupResult ParseLookupResponse(string json)
        {
            var result = new TemplateLookupResult();

            // Check "found" field
            result.Found = json.Contains("\"found\":true") || json.Contains("\"found\": true");

            if (result.Found)
            {
                result.Text = ExtractJsonString(json, "text");
                result.PersonalityStyle = ExtractJsonString(json, "personality_style");
                result.PersonalityClassRole = ExtractJsonString(json, "personality_class_role");
            }

            return result;
        }

        /// <summary>
        /// Parse the JSON response from /v1/templates/stats.
        /// </summary>
        private static Dictionary<string, string> ParseStatsResponse(string json)
        {
            var stats = new Dictionary<string, string>();

            // Extract known fields
            string enabled = ExtractJsonValue(json, "enabled");
            if (enabled != null) stats["enabled"] = enabled;

            string triggerCount = ExtractJsonValue(json, "trigger_count");
            if (triggerCount != null) stats["trigger_count"] = triggerCount;

            string variantCount = ExtractJsonValue(json, "variant_count");
            if (variantCount != null) stats["variant_count"] = variantCount;

            return stats;
        }

        /// <summary>
        /// Build JSON body for POST /v1/templates/queue.
        /// </summary>
        private static string BuildQueueJson(string trigger, string originalText,
            string channel, PersonalityData personality)
        {
            var sb = new System.Text.StringBuilder();
            sb.Append("{\"trigger\":\"");
            sb.Append(EscapeJsonString(trigger));
            sb.Append("\",\"context\":\"");
            sb.Append(EscapeJsonString(originalText));
            sb.Append("\",\"channel\":\"");
            sb.Append(EscapeJsonString(channel));
            sb.Append("\",\"sim_name\":\"");
            sb.Append(EscapeJsonString(personality.SourceSim));
            sb.Append("\",\"personality\":{\"style\":\"");
            sb.Append(EscapeJsonString(personality.Style));
            sb.Append("\",\"class_role\":\"");
            sb.Append(EscapeJsonString(personality.ClassRole));
            sb.Append("\",\"traits\":[");
            if (personality.Traits != null)
            {
                for (int i = 0; i < personality.Traits.Length; i++)
                {
                    if (i > 0) sb.Append(",");
                    sb.Append("\"");
                    sb.Append(EscapeJsonString(personality.Traits[i]));
                    sb.Append("\"");
                }
            }
            sb.Append("]}}");
            return sb.ToString();
        }

        /// <summary>
        /// Extract a JSON string value by key. Simple parser for flat JSON objects.
        /// </summary>
        private static string ExtractJsonString(string json, string key)
        {
            string search = "\"" + key + "\":\"";
            int idx = json.IndexOf(search);
            if (idx < 0)
            {
                // Try with space after colon
                search = "\"" + key + "\": \"";
                idx = json.IndexOf(search);
                if (idx < 0) return null;
            }

            int start = idx + search.Length;
            int end = start;
            while (end < json.Length)
            {
                if (json[end] == '"' && (end == start || json[end - 1] != '\\'))
                    break;
                end++;
            }
            if (end >= json.Length) return null;

            return json.Substring(start, end - start)
                .Replace("\\\"", "\"")
                .Replace("\\n", "\n")
                .Replace("\\\\", "\\");
        }

        /// <summary>
        /// Extract a JSON value (string, number, or boolean) by key.
        /// Returns the raw value as a string.
        /// </summary>
        private static string ExtractJsonValue(string json, string key)
        {
            // Try string value first
            string strVal = ExtractJsonString(json, key);
            if (strVal != null) return strVal;

            // Try non-string value (number, bool)
            string search = "\"" + key + "\":";
            int idx = json.IndexOf(search);
            if (idx < 0)
            {
                search = "\"" + key + "\": ";
                idx = json.IndexOf(search);
                if (idx < 0) return null;
            }

            int start = idx + search.Length;
            while (start < json.Length && json[start] == ' ') start++;

            int end = start;
            while (end < json.Length && json[end] != ',' && json[end] != '}' && json[end] != ' ')
                end++;

            if (end <= start) return null;
            return json.Substring(start, end - start);
        }

        /// <summary>
        /// Escapes special characters for JSON string values.
        /// </summary>
        private static string EscapeJsonString(string input)
        {
            if (string.IsNullOrEmpty(input))
                return "";

            return input
                .Replace("\\", "\\\\")
                .Replace("\"", "\\\"")
                .Replace("\n", "\\n")
                .Replace("\r", "\\r")
                .Replace("\t", "\\t");
        }

        /// <summary>
        /// Checks whether a UnityWebRequest encountered an error.
        /// Uses the modern result property (Unity 2020.1+).
        /// </summary>
        private static bool IsRequestError(UnityWebRequest request)
        {
            return request.result != UnityWebRequest.Result.Success;
        }
    }
}
