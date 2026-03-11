using System;
using System.Collections;
using System.Collections.Generic;
using ErenshorLLMDialog.Sidecar;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline
{
    /// <summary>
    /// In-process cache for combat dialog templates. Provides &lt;1ms sync lookups
    /// backed by the sidecar's /v1/templates/lookup endpoint.
    ///
    /// On cache miss, fires a non-blocking POST to /v1/templates/queue for
    /// background LLM generation. Refreshes from sidecar every 60 seconds.
    ///
    /// With sidecar down: all methods fail silently, cache stays empty,
    /// and all combat callouts pass through unchanged (zero delay).
    /// </summary>
    public static class TemplateCache
    {
        /// <summary>Cached variants keyed by trigger (e.g. "pulling", "aggro", "oom").</summary>
        private static Dictionary<string, List<TemplateVariant>> _variants
            = new Dictionary<string, List<TemplateVariant>>();

        /// <summary>Triggers that already have a pending generation request.</summary>
        private static HashSet<string> _pendingTriggers = new HashSet<string>();

        private static System.Random _rng = new System.Random();
        private static SidecarClient _client;
        private static MonoBehaviour _coroutineRunner;
        private static bool _initialized;

        /// <summary>Number of cached trigger keys.</summary>
        public static int TriggerCount => _variants.Count;

        /// <summary>
        /// Initialize the template cache. Attempts to load existing templates
        /// from the sidecar. If sidecar is down, starts with an empty cache.
        /// </summary>
        public static void Initialize(SidecarClient client, MonoBehaviour runner)
        {
            _client = client;
            _coroutineRunner = runner;
            _initialized = true;

            // Start periodic refresh coroutine
            runner.StartCoroutine(PeriodicRefresh());

            LogDebug("[TemplateCache] Initialized");
        }

        /// <summary>
        /// Look up a combat template variant for the given body text.
        /// Returns null on cache miss (original text should be used).
        /// This method is synchronous and completes in &lt;1ms.
        /// </summary>
        public static string FindCombatVariant(string body, string speaker, ChatChannel channel)
        {
            if (!_initialized || string.IsNullOrEmpty(body))
                return null;

            string trigger = ClassifyCallout(body);
            if (trigger == null)
                return null;

            if (_variants.TryGetValue(trigger, out var variants) && variants.Count > 0)
            {
                return PickVariant(variants, speaker);
            }

            // Cache miss -- queue background generation (fire-and-forget)
            QueueGeneration(trigger, body, channel, speaker);
            return null;
        }

        /// <summary>
        /// Classify a combat callout into a trigger key.
        /// Returns null if the text doesn't match any known pattern.
        /// </summary>
        public static string ClassifyCallout(string body)
        {
            if (string.IsNullOrEmpty(body))
                return null;

            string lower = body.ToLowerInvariant().Trim();

            // Pulling
            if (lower.StartsWith("pulling ") || lower.Contains("pull!") ||
                lower == "inc" || lower.Contains(" is here, attack"))
                return "pulling";

            // Aggro
            if (lower.Contains("have aggro") || lower.Contains("it's on me") ||
                lower.Contains("aggro on me"))
                return "aggro";

            // OOM / mana
            if (lower.Contains("oom") || lower.Contains("out of mana") ||
                lower.Contains("restoring my mana"))
                return "oom";

            // Healing
            if (lower.Contains("hot incoming") || lower.Contains("incoming on ") ||
                lower.Contains("healing ") || lower.Contains("regrowth"))
                return "healing";

            // Heal request
            if (lower.Contains("heal") && (lower.Contains("need") || lower.Contains("please")))
                return "heal_request";

            // Retreat / wipe
            if (lower.Contains("run!") || lower.Contains("flee!") || lower.Contains("wipe"))
                return "retreat";

            // Ready check
            if (lower.Contains("ready") || lower.Contains("rdy"))
                return "ready_check";

            // Taunting
            if (lower.Contains("taunting ") || lower.Contains("ae taunt"))
                return "taunting";

            // Targeting / assist
            if (lower.Contains("assisting ") || lower.Contains("killing ") ||
                lower.StartsWith("targeting ") || lower.StartsWith("i'm on "))
                return "targeting";

            // Casting
            if (lower.StartsWith("casting "))
                return "casting";

            // CC
            if (lower.Contains("can't be stunned") || lower.Contains("get on that one"))
                return "cc_notice";

            // Stance
            if (lower.Contains("stance"))
                return "stance";

            // Close call
            if (lower.Contains("close one"))
                return "close_call";

            // LFG
            if (lower.Contains("lfg") || lower.Contains("looking for group"))
                return "lfg";

            return null;
        }

        /// <summary>
        /// Pick a variant from the list, preferring personality-matched variants.
        /// Replaces {speaker} placeholder with the actual speaker name.
        /// </summary>
        private static string PickVariant(List<TemplateVariant> variants, string speaker)
        {
            // Try to find a personality-matched variant
            var personality = ChatClassifiers.GetPersonalityHint(speaker);
            var matched = new List<TemplateVariant>();

            foreach (var v in variants)
            {
                if (v.PersonalityStyle == personality.Style ||
                    v.PersonalityClassRole == personality.ClassRole)
                {
                    matched.Add(v);
                }
            }

            var pool = matched.Count > 0 ? matched : variants;
            var chosen = pool[_rng.Next(pool.Count)];

            string text = chosen.Text;
            if (text.Contains("{speaker}"))
                text = text.Replace("{speaker}", speaker);

            return text;
        }

        /// <summary>
        /// Queue a template generation request with the sidecar.
        /// Non-blocking, fire-and-forget. Suppresses duplicates.
        /// </summary>
        private static void QueueGeneration(string trigger, string original,
            ChatChannel channel, string speaker)
        {
            if (_client == null || _coroutineRunner == null)
                return;

            if (_pendingTriggers.Contains(trigger))
                return; // Already queued

            _pendingTriggers.Add(trigger);

            try
            {
                var personalityData = ChatClassifiers.GetPersonalityData(speaker);
                _coroutineRunner.StartCoroutine(
                    _client.QueueTemplateGeneration(
                        trigger,
                        original,
                        ChannelToString(channel),
                        personalityData,
                        onComplete: () => _pendingTriggers.Remove(trigger),
                        onError: () => _pendingTriggers.Remove(trigger)
                    ));
            }
            catch (Exception ex)
            {
                _pendingTriggers.Remove(trigger);
                LogDebug("[TemplateCache] QueueGeneration error: " + ex.Message);
            }
        }

        /// <summary>
        /// Periodic refresh coroutine. Runs every 60 seconds to pick up
        /// newly generated templates from the sidecar.
        /// </summary>
        private static IEnumerator PeriodicRefresh()
        {
            // Wait a bit for sidecar to start up
            yield return new WaitForSeconds(10f);

            // Initial load
            if (_coroutineRunner != null)
                _coroutineRunner.StartCoroutine(RefreshFromSidecar());

            while (true)
            {
                yield return new WaitForSeconds(60f);
                if (_coroutineRunner != null)
                    _coroutineRunner.StartCoroutine(RefreshFromSidecar());
            }
        }

        /// <summary>
        /// Refresh the cache from the sidecar's template store.
        /// Gets stats first to discover triggers, then looks up each one.
        /// Fails silently if sidecar is down.
        /// </summary>
        private static IEnumerator RefreshFromSidecar()
        {
            if (_client == null)
                yield break;

            // Step 1: Get stats to discover available triggers
            List<string> triggers = null;
            yield return _client.GetTemplateStats(stats =>
            {
                if (stats != null && stats.ContainsKey("enabled"))
                {
                    bool enabled;
                    if (bool.TryParse(stats["enabled"], out enabled) && enabled)
                    {
                        // We need the trigger list -- stats gives us count but not names.
                        // For now, look up our known trigger keys.
                        triggers = GetKnownTriggerKeys();
                    }
                }
            });

            if (triggers == null || triggers.Count == 0)
                yield break;

            // Step 2: Look up each trigger and cache the results
            int loaded = 0;
            foreach (string trigger in triggers)
            {
                yield return _client.LookupTemplate(trigger, "", "", result =>
                {
                    if (result != null && result.Found)
                    {
                        // Add to cache if not already present
                        if (!_variants.ContainsKey(trigger))
                            _variants[trigger] = new List<TemplateVariant>();

                        // Check for duplicates before adding
                        bool exists = false;
                        foreach (var existing in _variants[trigger])
                        {
                            if (existing.Text == result.Text)
                            {
                                exists = true;
                                break;
                            }
                        }

                        if (!exists)
                        {
                            _variants[trigger].Add(new TemplateVariant
                            {
                                Text = result.Text,
                                PersonalityStyle = result.PersonalityStyle ?? "",
                                PersonalityClassRole = result.PersonalityClassRole ?? ""
                            });
                            loaded++;
                        }
                    }
                });
            }

            if (loaded > 0)
                LogDebug("[TemplateCache] Refreshed: +" + loaded + " variants, total triggers=" +
                    _variants.Count);
        }

        /// <summary>
        /// Returns the list of known combat trigger keys for refresh polling.
        /// </summary>
        private static List<string> GetKnownTriggerKeys()
        {
            return new List<string>
            {
                "pulling", "aggro", "oom", "healing", "heal_request",
                "retreat", "ready_check", "taunting", "targeting",
                "casting", "cc_notice", "stance", "close_call", "lfg"
            };
        }

        private static string ChannelToString(ChatChannel channel)
        {
            switch (channel)
            {
                case ChatChannel.Say: return "say";
                case ChatChannel.Shout: return "shout";
                case ChatChannel.Whisper: return "whisper";
                case ChatChannel.Party: return "group";
                case ChatChannel.Guild: return "guild";
                case ChatChannel.Trade: return "trade";
                default: return "say";
            }
        }

        private static void LogDebug(string message)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging != null &&
                ErenshorLLMDialogPlugin.DebugLogging.Value == Toggle.On)
            {
                ErenshorLLMDialogPlugin.Log.LogInfo(message);
            }
        }
    }

    /// <summary>
    /// A cached template variant with personality metadata.
    /// </summary>
    public struct TemplateVariant
    {
        public string Text;
        public string PersonalityStyle;
        public string PersonalityClassRole;
    }

    /// <summary>
    /// Result from a sidecar template lookup.
    /// </summary>
    public class TemplateLookupResult
    {
        public bool Found;
        public string Text;
        public string PersonalityStyle;
        public string PersonalityClassRole;
    }
}
