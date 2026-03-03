using System;
using System.Collections;
using System.Collections.Generic;
using ErenshorLLMDialog.Sidecar;
using ErenshorLLMDialog.Sidecar.Models;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline
{
    /// <summary>
    /// Utility for paraphrasing canned game text through the sidecar's
    /// POST /v1/paraphrase endpoint. Used by Harmony hooks to enrich
    /// game-generated dialog (death reactions, loot comments, group invites,
    /// combat callouts, zone entry remarks, etc.) with personality voice,
    /// lore context, and GEPA grounding.
    ///
    /// On failure, the original canned text is used unchanged (graceful degradation).
    /// </summary>
    public class EventParaphraser
    {
        private readonly SidecarClient _client;
        private readonly SidecarManager _manager;
        private readonly MonoBehaviour _coroutineHost;

        public EventParaphraser(SidecarClient client, SidecarManager manager,
            MonoBehaviour coroutineHost)
        {
            _client = client;
            _manager = manager;
            _coroutineHost = coroutineHost;
        }

        /// <summary>
        /// Paraphrase canned text and deliver it to the chat log.
        /// Runs as a coroutine. On failure, delivers the original text unchanged.
        /// </summary>
        /// <param name="cannedText">The game's original canned dialog text.</param>
        /// <param name="trigger">Event type: group_death, loot_drop, loot_request,
        /// group_invite, combat_callout, zone_entry, hail, level_up, trade, revival, generic.</param>
        /// <param name="simName">Name of the SimPlayer speaking.</param>
        /// <param name="zone">Current zone name.</param>
        /// <param name="channel">Chat channel (say, shout, guild, group).</param>
        /// <param name="eventContext">Event-specific key-value pairs.</param>
        /// <param name="sim">Optional live SimPlayer for personalization.</param>
        /// <param name="relationship">Relationship level with player (0-10).</param>
        /// <param name="color">Chat log color (null for default).</param>
        public void Paraphrase(string cannedText, string trigger, string simName,
            string zone, string channel, Dictionary<string, string> eventContext,
            SimPlayer sim = null, float relationship = 5f, string color = null)
        {
            if (!_manager.IsHealthy)
            {
                // Sidecar not available -- deliver original text
                DeliverText(cannedText, simName, channel, sim, color);
                return;
            }

            _coroutineHost.StartCoroutine(
                ParaphraseAsync(cannedText, trigger, simName, zone, channel,
                    eventContext, sim, relationship, color));
        }

        private IEnumerator ParaphraseAsync(string cannedText, string trigger,
            string simName, string zone, string channel,
            Dictionary<string, string> eventContext, SimPlayer sim,
            float relationship, string color)
        {
            var request = new ParaphraseRequest
            {
                text = cannedText,
                trigger = trigger,
                sim_name = simName,
                zone = zone,
                channel = channel,
                relationship = relationship,
                player_name = GameData.PlayerControl != null
                    ? GameData.PlayerControl.transform.name : "Hero",
                context = eventContext ?? new Dictionary<string, string>()
            };

            ParaphraseResponse response = null;
            long latencyMs = 0;

            yield return _client.Paraphrase(request, (resp, ms) =>
            {
                response = resp;
                latencyMs = ms;
            });

            string outputText;
            if (response != null && response.paraphrased && !string.IsNullOrEmpty(response.text))
            {
                outputText = response.text;
                LogDebug("[EventParaphraser] " + simName + " [" + trigger + "]: \"" +
                    cannedText + "\" -> \"" + outputText + "\" (" + latencyMs + "ms, " +
                    response.source + ")");
            }
            else
            {
                outputText = cannedText;
                LogDebug("[EventParaphraser] " + simName + " [" + trigger + "]: " +
                    "using original (" + latencyMs + "ms)");
            }

            DeliverText(outputText, simName, channel, sim, color);
        }

        private static void DeliverText(string text, string simName, string channel,
            SimPlayer sim, string color)
        {
            // Apply game's personality system (ALL CAPS, third person, typos)
            string personalized = sim != null && GameData.SimMngr != null
                ? GameData.SimMngr.PersonalizeString(text, sim)
                : text;

            string formatted;
            switch (channel)
            {
                case "guild":
                    formatted = simName + " tells the guild: " + personalized;
                    UpdateSocialLog.LogAdd(formatted, color ?? "green");
                    break;
                case "shout":
                    formatted = simName + " shouts: " + personalized;
                    UpdateSocialLog.LogAdd(formatted, color ?? "#FF9000");
                    break;
                case "group":
                    formatted = simName + " tells the group: " + personalized;
                    UpdateSocialLog.LogAdd(formatted, color ?? "#87CEEB");
                    break;
                default: // say
                    formatted = simName + " says: " + personalized;
                    UpdateSocialLog.LogAdd(formatted, color);
                    UpdateSocialLog.LocalLogAdd(formatted);
                    break;
            }
        }

        /// <summary>
        /// Paraphrase canned text and return the result via callback.
        /// Does NOT deliver to chat -- caller handles output.
        /// Used by Harmony hooks that need to control delivery themselves
        /// (e.g., re-queueing into AddStringForDisplay).
        /// </summary>
        /// <param name="cannedText">The game's original canned dialog text.</param>
        /// <param name="trigger">Event type for the paraphrase prompt.</param>
        /// <param name="simName">Name of the SimPlayer speaking.</param>
        /// <param name="zone">Current zone name.</param>
        /// <param name="channel">Chat channel (say, shout, guild, group).</param>
        /// <param name="context">Event-specific key-value pairs.</param>
        /// <param name="relationship">Relationship level with player (0-10).</param>
        /// <param name="onResult">Callback receiving the paraphrased (or original) text.</param>
        public void ParaphraseText(string cannedText, string trigger, string simName,
            string zone, string channel, Dictionary<string, string> context,
            float relationship, Action<string> onResult)
        {
            if (!_manager.IsHealthy)
            {
                onResult(cannedText);
                return;
            }

            _coroutineHost.StartCoroutine(
                ParaphraseTextAsync(cannedText, trigger, simName, zone, channel,
                    context, relationship, onResult));
        }

        private IEnumerator ParaphraseTextAsync(string cannedText, string trigger,
            string simName, string zone, string channel,
            Dictionary<string, string> context, float relationship,
            Action<string> onResult)
        {
            var request = new ParaphraseRequest
            {
                text = cannedText,
                trigger = trigger,
                sim_name = simName,
                zone = zone,
                channel = channel,
                relationship = relationship,
                player_name = GameData.PlayerControl != null
                    ? GameData.PlayerControl.transform.name : "Hero",
                context = context ?? new Dictionary<string, string>()
            };

            ParaphraseResponse response = null;
            long latencyMs = 0;

            yield return _client.Paraphrase(request, (resp, ms) =>
            {
                response = resp;
                latencyMs = ms;
            });

            string result;
            if (response != null && response.paraphrased && !string.IsNullOrEmpty(response.text))
            {
                result = response.text;
                LogDebug("[EventParaphraser] " + simName + " [" + trigger + "]: \"" +
                    cannedText + "\" -> \"" + result + "\" (" + latencyMs + "ms, " +
                    response.source + ")");
            }
            else
            {
                result = cannedText;
                LogDebug("[EventParaphraser] " + simName + " [" + trigger + "]: " +
                    "using original (" + latencyMs + "ms)");
            }

            onResult(result);
        }

        /// <summary>Whether the sidecar is healthy and can accept paraphrase requests.</summary>
        public bool IsHealthy => _manager.IsHealthy;

        private static void LogDebug(string message)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging != null &&
                ErenshorLLMDialogPlugin.DebugLogging.Value == Toggle.On)
            {
                ErenshorLLMDialogPlugin.Log.LogInfo(message);
            }
        }
    }
}
