using System.Collections;
using System.Collections.Generic;
using BepInEx.Logging;
using ErenshorLLMDialog.Sidecar;
using UnityEngine;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Represents a single memory event queued for ingestion into the sidecar.
    /// </summary>
    public class MemoryEvent
    {
        public string Text;
        public string EventType;
        public string SimName;
        public string Zone;
        public Dictionary<string, string> Metadata;
        public float Timestamp;
    }

    /// <summary>
    /// Singleton manager that queues game events and sends them to the sidecar memory system.
    /// Implements rate limiting and debouncing to avoid flooding the sidecar.
    /// </summary>
    public class MemoryReuptakeManager
    {
        private static MemoryReuptakeManager _instance;
        public static MemoryReuptakeManager Instance => _instance;

        /// <summary>Maximum events that can be sent per minute.</summary>
        private const int MAX_EVENTS_PER_MINUTE = 10;

        /// <summary>Minimum seconds between events of the same type for the same entity.</summary>
        private const float DEBOUNCE_SECONDS = 1.0f;

        /// <summary>How often the drain coroutine runs, in seconds.</summary>
        private const float DRAIN_INTERVAL = 0.5f;

        /// <summary>Maximum events to send per drain cycle.</summary>
        private const int MAX_EVENTS_PER_DRAIN = 3;

        private SidecarClient _client;
        private MonoBehaviour _host;
        private ManualLogSource _log;

        private readonly Queue<MemoryEvent> _eventQueue = new Queue<MemoryEvent>();
        private readonly Dictionary<string, float> _debounceMap = new Dictionary<string, float>();

        private int _eventsSentThisMinute;
        private float _minuteResetTime;

        private bool _draining;

        /// <summary>
        /// Initialize the singleton manager. Must be called once during plugin Awake().
        /// </summary>
        public static void Initialize(SidecarClient client, MonoBehaviour host, ManualLogSource log)
        {
            _instance = new MemoryReuptakeManager
            {
                _client = client,
                _host = host,
                _log = log,
                _eventsSentThisMinute = 0,
                _minuteResetTime = Time.realtimeSinceStartup + 60f
            };

            _instance.StartDrainCoroutine();
            log.LogInfo("[MemoryReuptake] Initialized.");
        }

        /// <summary>
        /// Queue a memory event for ingestion. Returns false if the event was rate-limited or debounced.
        /// </summary>
        public bool QueueEvent(string text, string eventType, string simName, string zone, Dictionary<string, string> metadata = null)
        {
            if (string.IsNullOrEmpty(text))
                return false;

            float now = Time.realtimeSinceStartup;

            // Reset rate limit counter every minute
            if (now >= _minuteResetTime)
            {
                _eventsSentThisMinute = 0;
                _minuteResetTime = now + 60f;
            }

            // Rate limit check
            if (_eventsSentThisMinute >= MAX_EVENTS_PER_MINUTE)
            {
                _log.LogDebug("[MemoryReuptake] Rate limited, dropping event: " + text);
                return false;
            }

            // Debounce check: same event type + entity within DEBOUNCE_SECONDS
            string debounceKey = eventType + ":" + (simName ?? "");
            if (_debounceMap.TryGetValue(debounceKey, out float lastTime))
            {
                if (now - lastTime < DEBOUNCE_SECONDS)
                {
                    _log.LogDebug("[MemoryReuptake] Debounced event: " + text);
                    return false;
                }
            }
            _debounceMap[debounceKey] = now;

            var evt = new MemoryEvent
            {
                Text = text,
                EventType = eventType,
                SimName = simName ?? "",
                Zone = zone ?? "",
                Metadata = metadata,
                Timestamp = now
            };

            _eventQueue.Enqueue(evt);
            _log.LogDebug("[MemoryReuptake] Queued event: " + text);
            return true;
        }

        private void StartDrainCoroutine()
        {
            if (!_draining && _host != null)
            {
                _draining = true;
                _host.StartCoroutine(DrainQueue());
            }
        }

        private IEnumerator DrainQueue()
        {
            while (true)
            {
                yield return new WaitForSeconds(DRAIN_INTERVAL);

                int sent = 0;
                while (_eventQueue.Count > 0 && sent < MAX_EVENTS_PER_DRAIN)
                {
                    float now = Time.realtimeSinceStartup;

                    // Reset rate limit counter every minute
                    if (now >= _minuteResetTime)
                    {
                        _eventsSentThisMinute = 0;
                        _minuteResetTime = now + 60f;
                    }

                    if (_eventsSentThisMinute >= MAX_EVENTS_PER_MINUTE)
                        break;

                    MemoryEvent evt = _eventQueue.Dequeue();
                    _eventsSentThisMinute++;
                    sent++;

                    _host.StartCoroutine(_client.IngestMemory(
                        evt.Text, evt.EventType, evt.SimName, evt.Zone, evt.Metadata,
                        success =>
                        {
                            if (!success)
                                _log.LogWarning("[MemoryReuptake] Failed to ingest: " + evt.Text);
                        }));
                }
            }
        }
    }
}
