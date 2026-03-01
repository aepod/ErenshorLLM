using System;
using System.Collections;
using System.Collections.Generic;
using BepInEx.Logging;
using ErenshorLLMDialog.Sidecar;
using ErenshorLLMDialog.Sidecar.Models;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline
{
    /// <summary>
    /// Dispatches additional sim responses after the primary responder.
    /// Handles responder selection (per-channel), rate limiting, staggered
    /// output timing, and sim-to-sim conversation chaining.
    ///
    /// Called by RuVectorTransform after the primary response succeeds.
    /// Each additional responder gets its own sidecar /v1/respond call.
    /// </summary>
    public class MultiSimDispatcher
    {
        private readonly SidecarClient _client;
        private readonly SidecarManager _manager;
        private readonly RateLimiter _rateLimiter;
        private readonly MonoBehaviour _coroutineHost;
        private readonly ManualLogSource _log;
        private readonly SidecarConfig _config;

        public MultiSimDispatcher(SidecarClient client, SidecarManager manager,
            RateLimiter rateLimiter, MonoBehaviour coroutineHost,
            ManualLogSource log, SidecarConfig config)
        {
            _client = client;
            _manager = manager;
            _rateLimiter = rateLimiter;
            _coroutineHost = coroutineHost;
            _log = log;
            _config = config;
        }

        /// <summary>
        /// Dispatches additional sim responses after the primary.
        /// Skips whisper (always 1:1) and respects rate limits.
        /// </summary>
        public void DispatchAdditional(DialogContext primaryCtx)
        {
            if (_config.MultiSimEnabled.Value != Toggle.On)
                return;
            if (primaryCtx.Channel == ChatChannel.Whisper)
                return;
            if (!_manager.IsHealthy)
                return;

            var candidates = SelectCandidates(primaryCtx);
            if (candidates.Count == 0)
            {
                LogDebug("[MultiSimDispatcher] No additional candidates for " +
                    primaryCtx.Channel);
                return;
            }

            int maxAdditional = GetAdaptiveMax(primaryCtx.Channel);
            if (maxAdditional <= 0)
            {
                LogDebug("[MultiSimDispatcher] Rate limit conserving, skipping " +
                    "additional responders (remaining=" + _rateLimiter.Remaining + ")");
                return;
            }

            int dispatched = 0;
            float cumulativeDelay = 0f;

            for (int i = 0; i < candidates.Count && dispatched < maxAdditional; i++)
            {
                if (!_rateLimiter.TryConsume())
                {
                    LogDebug("[MultiSimDispatcher] Rate limit hit after " +
                        dispatched + " additional dispatches");
                    break;
                }

                cumulativeDelay += GetStaggerDelay(primaryCtx.Channel);
                _coroutineHost.StartCoroutine(
                    DispatchOne(primaryCtx, candidates[i], cumulativeDelay, 0));
                dispatched++;
            }

            LogDebug("[MultiSimDispatcher] Dispatched " + dispatched +
                " additional responders for " + primaryCtx.Channel +
                " (budget remaining: " + _rateLimiter.Remaining + ")");
        }

        private List<SimCandidate> SelectCandidates(DialogContext ctx)
        {
            var candidates = new List<SimCandidate>();
            string primaryName = ctx.TargetNPCName;

            switch (ctx.Channel)
            {
                case ChatChannel.Say:
                    SelectSayCandidates(primaryName, candidates);
                    break;
                case ChatChannel.Guild:
                    SelectGuildCandidates(ctx, primaryName, candidates);
                    break;
                case ChatChannel.Shout:
                    SelectShoutCandidates(primaryName, candidates);
                    break;
            }

            return candidates;
        }

        /// <summary>
        /// Say: nearby sims (30f range), excluding primary, ~50% chance each.
        /// </summary>
        private void SelectSayCandidates(string primaryName, List<SimCandidate> candidates)
        {
            if (GameData.SimMngr == null || GameData.SimMngr.ActiveSimInstances == null)
                return;

            foreach (SimPlayer sim in GameData.SimMngr.ActiveSimInstances)
            {
                if (sim == null || sim.IsGMCharacter) continue;
                NPC npc = sim.GetComponent<NPC>();
                if (npc == null) continue;
                if (npc.NPCName == primaryName) continue;

                float dist = Vector3.Distance(
                    GameData.PlayerControl.transform.position,
                    sim.transform.position);
                if (dist > 30f) continue;

                // 40-60% chance to respond (matching game's variable rates)
                if (UnityEngine.Random.Range(0f, 1f) > 0.55f) continue;

                candidates.Add(new SimCandidate
                {
                    Name = npc.NPCName,
                    Tracking = FindTracking(npc.NPCName),
                    Player = sim
                });
            }
        }

        /// <summary>
        /// Guild: from guild roster (zone-independent), shuffled, excluding primary.
        /// </summary>
        private void SelectGuildCandidates(DialogContext ctx, string primaryName,
            List<SimCandidate> candidates)
        {
            if (ctx.GuildSimNames.Count == 0) return;

            var shuffled = new List<string>(ctx.GuildSimNames);
            ShuffleList(shuffled);

            foreach (string name in shuffled)
            {
                if (name == primaryName) continue;

                SimPlayerTracking tracking = FindTracking(name);
                if (tracking == null) continue;

                candidates.Add(new SimCandidate
                {
                    Name = name,
                    Tracking = tracking,
                    Player = tracking.MyAvatar
                });
            }
        }

        /// <summary>
        /// Shout: zone-wide from ActiveSimInstances, ~35% chance each.
        /// </summary>
        private void SelectShoutCandidates(string primaryName, List<SimCandidate> candidates)
        {
            if (GameData.SimMngr == null || GameData.SimMngr.ActiveSimInstances == null)
                return;

            foreach (SimPlayer sim in GameData.SimMngr.ActiveSimInstances)
            {
                if (sim == null || sim.IsGMCharacter) continue;
                NPC npc = sim.GetComponent<NPC>();
                if (npc == null) continue;
                if (npc.NPCName == primaryName) continue;

                // 30-50% chance
                if (UnityEngine.Random.Range(0f, 1f) > 0.40f) continue;

                candidates.Add(new SimCandidate
                {
                    Name = npc.NPCName,
                    Tracking = FindTracking(npc.NPCName),
                    Player = sim
                });
            }
        }

        /// <summary>
        /// Coroutine: waits for stagger delay, fires a sidecar request for one
        /// additional sim, outputs the response, and optionally triggers sim-to-sim.
        /// </summary>
        private IEnumerator DispatchOne(DialogContext primaryCtx, SimCandidate candidate,
            float delay, int simToSimDepth)
        {
            yield return new WaitForSeconds(delay);

            if (!_manager.IsHealthy)
                yield break;

            var request = BuildRequest(primaryCtx, candidate, simToSimDepth);
            RespondResponse sidecarResponse = null;
            long latencyMs = 0;

            yield return _client.Respond(request, (resp, ms) =>
            {
                sidecarResponse = resp;
                latencyMs = ms;
            });

            if (sidecarResponse == null || string.IsNullOrEmpty(sidecarResponse.response))
            {
                LogDebug("[MultiSimDispatcher] " + candidate.Name +
                    ": no response (" + latencyMs + "ms)");
                yield break;
            }

            OutputResponse(primaryCtx.Channel, candidate, sidecarResponse.response);

            LogDebug("[MultiSimDispatcher] " + candidate.Name + " responded (" +
                latencyMs + "ms, confidence=" + sidecarResponse.confidence.ToString("F3") +
                ", source=" + sidecarResponse.source + ")");

            // Sim-to-sim: chance for another sim to react to this sim's response
            if (_config.SimToSimEnabled.Value == Toggle.On &&
                simToSimDepth < _config.SimToSimMaxDepth.Value &&
                _rateLimiter.Remaining > 10)
            {
                TrySimToSim(primaryCtx, candidate, sidecarResponse.response,
                    simToSimDepth + 1);
            }
        }

        /// <summary>
        /// Attempts to trigger a sim-to-sim reaction. A random nearby sim
        /// responds to the previous sim's message, creating natural chatter.
        /// </summary>
        private void TrySimToSim(DialogContext primaryCtx, SimCandidate respondent,
            string response, int depth)
        {
            // 30-40% chance for a sim-to-sim reaction
            if (UnityEngine.Random.Range(0f, 1f) > 0.35f)
                return;

            if (!_rateLimiter.TryConsume())
                return;

            SimCandidate reactor = PickReactor(primaryCtx, respondent.Name);
            if (reactor == null) return;

            // Build context where the "player" is the previous sim
            var simCtx = new DialogContext
            {
                PlayerMessage = response,
                PlayerName = respondent.Name,
                Channel = primaryCtx.Channel,
                CurrentZone = primaryCtx.CurrentZone,
                PlayerLevel = respondent.Tracking != null ? respondent.Tracking.Level : 1,
                PlayerClass = respondent.Tracking != null
                    ? (respondent.Tracking.ClassName ?? "") : "",
                PlayerGuild = primaryCtx.PlayerGuild,
                PrimaryContext = primaryCtx,
                RespondingTo = response
            };

            // Copy guild sim names for guild channel sim-to-sim
            foreach (string name in primaryCtx.GuildSimNames)
                simCtx.GuildSimNames.Add(name);

            float delay = GetStaggerDelay(primaryCtx.Channel);
            _coroutineHost.StartCoroutine(DispatchOne(simCtx, reactor, delay, depth));

            LogDebug("[MultiSimDispatcher] Sim-to-sim: " + reactor.Name +
                " reacting to " + respondent.Name + " (depth=" + depth + ")");
        }

        /// <summary>
        /// Picks a random sim to react in a sim-to-sim chain, excluding
        /// the primary responder and the most recent respondent.
        /// </summary>
        private SimCandidate PickReactor(DialogContext ctx, string excludeName)
        {
            string primaryName = ctx.TargetNPCName;

            if (ctx.Channel == ChatChannel.Guild)
            {
                if (ctx.GuildSimNames.Count == 0) return null;

                var eligible = new List<string>();
                foreach (string name in ctx.GuildSimNames)
                {
                    if (name != primaryName && name != excludeName)
                        eligible.Add(name);
                }
                if (eligible.Count == 0) return null;

                string picked = eligible[UnityEngine.Random.Range(0, eligible.Count)];
                SimPlayerTracking tracking = FindTracking(picked);
                if (tracking == null) return null;
                return new SimCandidate
                {
                    Name = picked,
                    Tracking = tracking,
                    Player = tracking.MyAvatar
                };
            }

            // Say and Shout: pick from active sim instances
            if (GameData.SimMngr == null || GameData.SimMngr.ActiveSimInstances == null)
                return null;

            float maxDist = ctx.Channel == ChatChannel.Say ? 30f : float.MaxValue;
            var eligibleSims = new List<SimCandidate>();

            foreach (SimPlayer sim in GameData.SimMngr.ActiveSimInstances)
            {
                if (sim == null || sim.IsGMCharacter) continue;
                NPC npc = sim.GetComponent<NPC>();
                if (npc == null) continue;
                if (npc.NPCName == primaryName || npc.NPCName == excludeName) continue;

                if (ctx.Channel == ChatChannel.Say)
                {
                    float dist = Vector3.Distance(
                        GameData.PlayerControl.transform.position,
                        sim.transform.position);
                    if (dist > maxDist) continue;
                }

                eligibleSims.Add(new SimCandidate
                {
                    Name = npc.NPCName,
                    Tracking = FindTracking(npc.NPCName),
                    Player = sim
                });
            }

            return eligibleSims.Count > 0
                ? eligibleSims[UnityEngine.Random.Range(0, eligibleSims.Count)]
                : null;
        }

        private RespondRequest BuildRequest(DialogContext primaryCtx,
            SimCandidate candidate, int simToSimDepth)
        {
            float relationship = 5.0f;
            if (candidate.Player != null)
                relationship = candidate.Player.OpinionOfPlayer;
            else if (candidate.Tracking != null)
                relationship = candidate.Tracking.OpinionOfPlayer;

            string simGuild = "";
            if (candidate.Tracking != null &&
                !string.IsNullOrEmpty(candidate.Tracking.GuildID) &&
                GameData.GuildManager != null)
            {
                simGuild = GameData.GuildManager.GetGuildNameByID(
                    candidate.Tracking.GuildID) ?? "";
            }

            bool simIsRival = candidate.Tracking != null && candidate.Tracking.Rival;

            var req = new RespondRequest
            {
                player_message = primaryCtx.PlayerMessage,
                channel = primaryCtx.Channel.ToString().ToLower(),
                sim_name = candidate.Name,
                zone = primaryCtx.CurrentZone,
                relationship = relationship,
                player_name = primaryCtx.PlayerName,
                player_level = primaryCtx.PlayerLevel,
                player_class = primaryCtx.PlayerClass,
                player_guild = primaryCtx.PlayerGuild,
                sim_guild = simGuild,
                sim_is_rival = simIsRival,
                group_members = new List<string>(primaryCtx.GroupMembers)
            };

            // For sim-to-sim, override player_message with the sim's response
            if (simToSimDepth > 0 && !string.IsNullOrEmpty(primaryCtx.RespondingTo))
            {
                req.player_message = primaryCtx.RespondingTo;
                req.player_name = primaryCtx.PlayerName;
            }

            // Build personality from SimPlayer (live) or SimPlayerTracking (cross-zone)
            if (candidate.Player != null)
            {
                var sim = candidate.Player;
                req.personality["friendly"] = sim.PersonalityType == 1 || sim.SocialChase >= 6;
                req.personality["aggressive"] = sim.PersonalityType == 3 || sim.Troublemaker >= 5;
                req.personality["scholarly"] = sim.LoreChase >= 5;
                req.personality["social"] = sim.SocialChase >= 4 || sim.PersonalityType == 1;
                req.personality["types_in_all_caps"] = sim.TypesInAllCaps;
                req.personality["types_in_third_person"] = sim.TypesInThirdPerson;
                req.personality["casual"] = sim.TypoRate > 0.1f;
            }
            else if (candidate.Tracking != null)
            {
                var t = candidate.Tracking;
                req.personality["friendly"] = t.Personality == 1 || t.SocialChase >= 6;
                req.personality["aggressive"] = t.Personality == 3 || t.Troublemaker >= 5;
                req.personality["scholarly"] = t.LoreChase >= 5;
                req.personality["social"] = t.SocialChase >= 4 || t.Personality == 1;
                req.personality["types_in_all_caps"] = false;
                req.personality["types_in_third_person"] = false;
                req.personality["casual"] = false;
            }

            return req;
        }

        /// <summary>
        /// Outputs a response to the chat log with channel-appropriate formatting.
        /// </summary>
        private void OutputResponse(ChatChannel channel, SimCandidate candidate,
            string response)
        {
            string personalized = candidate.Player != null
                ? GameData.SimMngr.PersonalizeString(response, candidate.Player)
                : response;

            string formatted;
            switch (channel)
            {
                case ChatChannel.Guild:
                    formatted = candidate.Name + " tells the guild: " + personalized;
                    UpdateSocialLog.LogAdd(formatted, "green");
                    break;
                case ChatChannel.Shout:
                    formatted = candidate.Name + " shouts: " + personalized;
                    UpdateSocialLog.LogAdd(formatted, "#FF9000");
                    break;
                default: // Say
                    formatted = candidate.Name + " says: " + personalized;
                    UpdateSocialLog.LogAdd(formatted);
                    UpdateSocialLog.LocalLogAdd(formatted);
                    break;
            }
        }

        /// <summary>
        /// Returns the max number of additional responders, adapting based
        /// on remaining rate limit budget.
        /// </summary>
        private int GetAdaptiveMax(ChatChannel channel)
        {
            int configMax;
            switch (channel)
            {
                case ChatChannel.Say:
                    configMax = _config.MaxAdditionalSay.Value;
                    break;
                case ChatChannel.Guild:
                    configMax = _config.MaxAdditionalGuild.Value;
                    break;
                case ChatChannel.Shout:
                    configMax = _config.MaxAdditionalShout.Value;
                    break;
                default:
                    return 0;
            }

            int remaining = _rateLimiter.Remaining;
            if (remaining > 20) return configMax;
            if (remaining > 10) return Math.Min(configMax, 1);
            return 0;
        }

        /// <summary>
        /// Returns a random stagger delay matching the game's natural
        /// response timing for each channel.
        /// </summary>
        private static float GetStaggerDelay(ChatChannel channel)
        {
            switch (channel)
            {
                case ChatChannel.Say:
                    // Game: 35-160 frames at ~60fps = 0.6-2.7s
                    return UnityEngine.Random.Range(0.6f, 2.7f);
                case ChatChannel.Guild:
                    // Game: WaitForSeconds(0.5-15.5)
                    return UnityEngine.Random.Range(0.5f, 15.5f);
                case ChatChannel.Shout:
                    // Game: 40-140 frames at ~60fps = 0.7-2.3s
                    return UnityEngine.Random.Range(0.7f, 2.3f);
                default:
                    return 1.0f;
            }
        }

        private static SimPlayerTracking FindTracking(string simName)
        {
            if (GameData.SimMngr == null || GameData.SimMngr.Sims == null)
                return null;

            foreach (SimPlayerTracking t in GameData.SimMngr.Sims)
            {
                if (t != null && t.SimName == simName)
                    return t;
            }
            return null;
        }

        private static void ShuffleList<T>(List<T> list)
        {
            for (int i = list.Count - 1; i > 0; i--)
            {
                int j = UnityEngine.Random.Range(0, i + 1);
                T temp = list[i];
                list[i] = list[j];
                list[j] = temp;
            }
        }

        private void LogDebug(string message)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging != null &&
                ErenshorLLMDialogPlugin.DebugLogging.Value == Toggle.On)
            {
                _log.LogInfo(message);
            }
        }

        private class SimCandidate
        {
            public string Name;
            public SimPlayerTracking Tracking;
            public SimPlayer Player;
        }
    }
}
