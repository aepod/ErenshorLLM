using System;
using System.Collections;
using System.Collections.Generic;
using BepInEx.Logging;
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
