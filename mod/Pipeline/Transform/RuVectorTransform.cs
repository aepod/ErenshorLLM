using System.Collections;
using System.Collections.Generic;
using ErenshorLLMDialog.Sidecar;
using ErenshorLLMDialog.Sidecar.Models;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline.Transform
{
    /// <summary>
    /// Transform module that calls the Rust sidecar's POST /v1/respond endpoint
    /// for semantic template-based dialog responses.
    ///
    /// Uses a two-phase approach that fits the existing pipeline pattern:
    /// 1. Transform() is called synchronously in Observe(). If the sidecar is
    ///    healthy and the message qualifies, it starts an async coroutine and
    ///    returns true (marks as handled) so the pipeline suppresses the game's
    ///    default ParseSay processing.
    /// 2. The coroutine fires the HTTP request. When the response arrives, it
    ///    either updates the context with the sidecar's response, or clears the
    ///    handled flag so the pipeline's output step skips it.
    ///
    /// If the sidecar is unavailable, returns false immediately so the next
    /// transform in the chain (HelloWorldTransform) gets a chance.
    /// </summary>
    public class RuVectorTransform : ITransformModule
    {
        private readonly SidecarClient _client;
        private readonly SidecarManager _manager;
        private readonly MonoBehaviour _coroutineHost;
        private readonly float _minConfidence;
        private readonly MultiSimDispatcher _dispatcher;

        // Response tuning overrides (passed to sidecar per-request)
        private readonly int _templateCandidates;
        private readonly int _loreContextCount;
        private readonly int _memoryContextCount;

        // Re-ranking weight overrides
        private readonly float _wSemantic;
        private readonly float _wChannel;
        private readonly float _wZone;
        private readonly float _wPersonality;
        private readonly float _wRelationship;

        // Tracks the most recent pending async request per channel so we can
        // handle rapid-fire messages (only the latest response per channel is
        // delivered). Separate tracking ensures messages in quick succession
        // across different channels don't clobber each other.
        private DialogContext _pendingSayContext;
        private DialogContext _pendingWhisperContext;
        private DialogContext _pendingGuildContext;
        private DialogContext _pendingShoutContext;

        public RuVectorTransform(SidecarClient client, SidecarManager manager,
            MonoBehaviour coroutineHost, float minConfidence,
            int templateCandidates = 10, int loreContextCount = 2,
            int memoryContextCount = 2,
            float wSemantic = 0.20f, float wChannel = 0.15f,
            float wZone = 0.20f, float wPersonality = 0.30f,
            float wRelationship = 0.15f,
            MultiSimDispatcher dispatcher = null)
        {
            _client = client;
            _manager = manager;
            _coroutineHost = coroutineHost;
            _minConfidence = minConfidence;
            _templateCandidates = templateCandidates;
            _loreContextCount = loreContextCount;
            _memoryContextCount = memoryContextCount;
            _wSemantic = wSemantic;
            _wChannel = wChannel;
            _wZone = wZone;
            _wPersonality = wPersonality;
            _wRelationship = wRelationship;
            _dispatcher = dispatcher;
        }

        /// <summary>
        /// Synchronously called by the pipeline in Observe().
        /// Returns true if this module will handle the message (marks ctx.Handled).
        /// The actual HTTP call runs as a coroutine; the response is injected
        /// into the context asynchronously and output by ChatOutput.
        /// </summary>
        public bool Transform(DialogContext ctx)
        {
            // Guard: sidecar must be healthy
            if (!_manager.IsHealthy)
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Sidecar not healthy (" +
                    _manager.Status + "), skipping");
                return false;
            }

            // Guard: must have a SimPlayer target or at least tracking (for cross-zone whisper)
            if (ctx.TargetSimPlayer == null && ctx.TargetSimTracking == null)
            {
                ctx.PipelineLog.Add("[RuVectorTransform] No SimPlayer target or tracking, skipping");
                return false;
            }

            // Guard: supported channels
            if (ctx.Channel != ChatChannel.Say && ctx.Channel != ChatChannel.Whisper &&
                ctx.Channel != ChatChannel.Guild && ctx.Channel != ChatChannel.Shout)
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Channel " + ctx.Channel +
                    " not supported, skipping");
                return false;
            }

            // Guard: for whispers, let the game handle messages that trigger gameplay
            // mechanics (guild join, group invite, trade, yes/no state machine, etc.).
            // ProcessWhisper has 15 trigger checks; we only intercept the ones that
            // are purely conversational (greetings, chitchat, "didn't understand").
            if (ctx.Channel == ChatChannel.Whisper && IsGameMechanicWhisper(ctx.PlayerMessage))
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Game mechanic trigger detected, " +
                    "deferring to ProcessWhisper");
                return false;
            }

            // Mark as handled and async to suppress game's default ParseSay
            // processing. The pipeline will see Handled=true and suppress ParseSay,
            // but IsAsync=true tells ExecutePendingTransform to skip synchronous
            // output (the coroutine will handle output when the response arrives).
            ctx.Handled = true;
            ctx.IsAsync = true;
            ctx.PipelineLog.Add("[RuVectorTransform] Starting async sidecar request");

            // Fire the async request (tracked per channel)
            if (ctx.Channel == ChatChannel.Whisper)
                _pendingWhisperContext = ctx;
            else if (ctx.Channel == ChatChannel.Guild)
                _pendingGuildContext = ctx;
            else if (ctx.Channel == ChatChannel.Shout)
                _pendingShoutContext = ctx;
            else
                _pendingSayContext = ctx;
            _coroutineHost.StartCoroutine(ProcessAsync(ctx));

            return true;
        }

        /// <summary>
        /// Coroutine that calls the sidecar's /v1/respond endpoint and
        /// processes the response.
        /// </summary>
        private IEnumerator ProcessAsync(DialogContext ctx)
        {
            var request = BuildRespondRequest(ctx);
            RespondResponse sidecarResponse = null;
            long latencyMs = 0;

            yield return _client.Respond(request, (resp, ms) =>
            {
                sidecarResponse = resp;
                latencyMs = ms;
            });

            // Check if this context is still the active one (rapid-fire protection).
            // Each channel is tracked independently so they don't clobber each other.
            DialogContext activePending;
            if (ctx.Channel == ChatChannel.Whisper)
                activePending = _pendingWhisperContext;
            else if (ctx.Channel == ChatChannel.Guild)
                activePending = _pendingGuildContext;
            else if (ctx.Channel == ChatChannel.Shout)
                activePending = _pendingShoutContext;
            else
                activePending = _pendingSayContext;
            if (activePending != ctx)
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Superseded by newer " +
                    ctx.Channel + " request, discarding");
                LogAsyncCompletion(ctx);
                yield break;
            }

            // Handle failure -- deliver fallback response since game was already suppressed
            if (sidecarResponse == null)
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Sidecar request failed (" +
                    latencyMs + "ms), delivering fallback");
                DeliverFallback(ctx);
                LogAsyncCompletion(ctx);
                yield break;
            }

            // Handle empty response
            if (string.IsNullOrEmpty(sidecarResponse.response))
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Sidecar returned empty response (" +
                    latencyMs + "ms), delivering fallback");
                DeliverFallback(ctx);
                LogAsyncCompletion(ctx);
                yield break;
            }

            // Handle low confidence
            if (sidecarResponse.confidence < _minConfidence)
            {
                ctx.PipelineLog.Add("[RuVectorTransform] Below confidence threshold: " +
                    sidecarResponse.confidence.ToString("F3") + " < " +
                    _minConfidence.ToString("F2") + " (" + latencyMs + "ms" +
                    "), delivering fallback");
                DeliverFallback(ctx);
                LogAsyncCompletion(ctx);
                yield break;
            }

            // Success: set the transformed response
            ctx.TransformedResponse = sidecarResponse.response;
            ctx.Handled = true;

            // Debug logging
            string loreStr = sidecarResponse.lore_context.Count > 0
                ? string.Join("; ", sidecarResponse.lore_context)
                : "none";
            string memoryStr = sidecarResponse.memory_context.Count > 0
                ? string.Join("; ", sidecarResponse.memory_context)
                : "none";

            ctx.PipelineLog.Add("[RuVectorTransform] Response set: template=" +
                sidecarResponse.template_id +
                " confidence=" + sidecarResponse.confidence.ToString("F3") +
                " source=" + sidecarResponse.source +
                " latency=" + latencyMs + "ms" +
                " sidecar_total=" + sidecarResponse.timing.total_ms + "ms" +
                " llm_ms=" + sidecarResponse.timing.llm_ms + "ms");

            if (!string.IsNullOrEmpty(sidecarResponse.llm_fallback_reason))
                ctx.PipelineLog.Add("[RuVectorTransform] LLM fallback: " + sidecarResponse.llm_fallback_reason);

            ctx.PipelineLog.Add("[RuVectorTransform] Lore: " + loreStr);
            ctx.PipelineLog.Add("[RuVectorTransform] Memory: " + memoryStr);

            // Output the response directly. Since this is an async transform,
            // the pipeline's ExecutePendingTransform() skipped synchronous output
            // (IsAsync=true). We handle output here when the response arrives.
            OutputResponse(ctx);

            // After primary response, dispatch additional sim responses for
            // multi-sim channels (say, guild, shout). Whisper stays 1:1.
            if (_dispatcher != null && ctx.Channel != ChatChannel.Whisper)
            {
                _dispatcher.DispatchAdditional(ctx);
            }

            // Log the completed async pipeline for debugging
            LogAsyncCompletion(ctx);
        }

        /// <summary>
        /// Directly outputs the response through the chat system.
        /// This is called from the coroutine when the async response arrives.
        /// </summary>
        private void OutputResponse(DialogContext ctx)
        {
            if (!ctx.Handled || string.IsNullOrEmpty(ctx.TransformedResponse))
                return;

            // Determine sim name from SimPlayer (zone-local) or tracking (cross-zone)
            string simName;
            if (ctx.TargetSimPlayer != null)
            {
                NPC npc = ctx.TargetSimPlayer.GetComponent<NPC>();
                simName = npc != null ? npc.NPCName : ctx.TargetNPCName;
            }
            else if (!string.IsNullOrEmpty(ctx.TargetNPCName))
            {
                simName = ctx.TargetNPCName;
            }
            else
            {
                return;
            }

            // Apply the game's personality system when we have a live SimPlayer
            string personalized = ctx.TargetSimPlayer != null
                ? GameData.SimMngr.PersonalizeString(ctx.TransformedResponse, ctx.TargetSimPlayer)
                : ctx.TransformedResponse;

            // Format based on channel
            string formatted;
            if (ctx.Channel == ChatChannel.Whisper)
            {
                formatted = "[WHISPER FROM] " + simName + ": " + personalized;
                UpdateSocialLog.LogAdd(formatted, "#FF62D1");

                // Match game behavior: play receive sound and set reply target
                if (GameData.PlayerAud != null && GameData.Misc != null)
                    GameData.PlayerAud.PlayOneShot(GameData.Misc.ReceiveTell);
                GameData.TextInput.LastPlayerMsg = simName;
            }
            else if (ctx.Channel == ChatChannel.Guild)
            {
                formatted = simName + " tells the guild: " + personalized;
                UpdateSocialLog.LogAdd(formatted, "green");
            }
            else if (ctx.Channel == ChatChannel.Shout)
            {
                formatted = simName + " shouts: " + personalized;
                UpdateSocialLog.LogAdd(formatted, "#FF9000");
            }
            else
            {
                formatted = simName + " says: " + personalized;
                UpdateSocialLog.LogAdd(formatted);
                UpdateSocialLog.LocalLogAdd(formatted);
            }

            ctx.PipelineLog.Add("[RuVectorTransform] Output response from " + simName);
        }

        /// <summary>
        /// Delivers a fallback response when the sidecar fails, returns empty,
        /// or scores below confidence threshold. Since the game's native
        /// ParseSay/ParseShout/ParseGuild was already suppressed, doing nothing
        /// would leave silence. This picks a generic game-appropriate line.
        /// </summary>
        private void DeliverFallback(DialogContext ctx)
        {
            // Pick a simple fallback based on channel. These match the tone of
            // the game's own canned responses.
            string[] fallbacks;
            switch (ctx.Channel)
            {
                case ChatChannel.Whisper:
                    fallbacks = new[]
                    {
                        "Hmm, interesting.",
                        "I see.",
                        "Tell me more.",
                        "I'm not sure what to say to that.",
                        "That's something to think about."
                    };
                    break;
                case ChatChannel.Guild:
                    fallbacks = new[]
                    {
                        "I'm here.",
                        "What's up?",
                        "I'm busy right now.",
                        "Need something?"
                    };
                    break;
                case ChatChannel.Shout:
                    fallbacks = new[]
                    {
                        "Hail!",
                        "Good luck out there!",
                        "Safe travels!",
                        "Indeed!"
                    };
                    break;
                default: // Say
                    fallbacks = new[]
                    {
                        "Hail.",
                        "Hello there.",
                        "Greetings.",
                        "Well met."
                    };
                    break;
            }

            string response = fallbacks[UnityEngine.Random.Range(0, fallbacks.Length)];

            // Apply personalization if we have a live SimPlayer
            if (ctx.TargetSimPlayer != null)
                response = GameData.SimMngr.PersonalizeString(response, ctx.TargetSimPlayer);

            ctx.TransformedResponse = response;
            ctx.Handled = true;
            ctx.PipelineLog.Add("[RuVectorTransform] Fallback: \"" + response + "\"");

            OutputResponse(ctx);
        }

        /// <summary>
        /// Logs the async pipeline completion with all pipeline log entries.
        /// This supplements the pipeline's LogDebug which ran before the
        /// async response arrived.
        /// </summary>
        private void LogAsyncCompletion(DialogContext ctx)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging == null ||
                ErenshorLLMDialogPlugin.DebugLogging.Value != Toggle.On)
                return;

            var log = ErenshorLLMDialogPlugin.Log;
            log.LogInfo("=== RuVectorTransform Async Complete ===");
            log.LogInfo("Target: " + ctx.TargetNPCName);
            log.LogInfo("Message: \"" + ctx.PlayerMessage + "\"");
            foreach (string entry in ctx.PipelineLog)
                log.LogInfo("Pipeline: " + entry);
            log.LogInfo("Response: \"" + ctx.TransformedResponse + "\"");
            log.LogInfo("========================================");
        }

        /// <summary>
        /// Builds a RespondRequest from the pipeline's DialogContext,
        /// including optional override parameters from BepInEx config.
        /// </summary>
        private RespondRequest BuildRespondRequest(DialogContext ctx)
        {
            // Use relationship from SimPlayer (live) or tracking (cross-zone), default 5
            float relationship = 5.0f;
            if (ctx.TargetSimPlayer != null)
                relationship = ctx.TargetSimPlayer.OpinionOfPlayer;
            else if (ctx.TargetSimTracking != null)
                relationship = ctx.TargetSimTracking.OpinionOfPlayer;

            var req = new RespondRequest
            {
                player_message = ctx.PlayerMessage,
                channel = ctx.Channel.ToString().ToLower(),
                sim_name = ctx.TargetNPCName,
                zone = ctx.CurrentZone,
                relationship = relationship,
                player_name = ctx.PlayerName,
                player_level = ctx.PlayerLevel,
                player_class = ctx.PlayerClass,
                player_guild = ctx.PlayerGuild,
                sim_guild = ctx.TargetGuild,
                sim_is_rival = ctx.TargetIsRival,
                group_members = new List<string>(ctx.GroupMembers),

                // Override parameters from BepInEx config
                template_candidates = _templateCandidates,
                lore_context_count = _loreContextCount,
                memory_context_count = _memoryContextCount,
                w_semantic = _wSemantic,
                w_channel = _wChannel,
                w_zone = _wZone,
                w_personality = _wPersonality,
                w_relationship = _wRelationship
            };

            // Build personality dict from SimPlayer traits (live) or SimPlayerTracking (cross-zone).
            // Game PersonalityType values: 1=Nice, 2=Tryhard, 3=Mean, 5=Neutral
            // Behavioral fields (int 0-10): LoreChase, GearChase, SocialChase, Troublemaker
            if (ctx.TargetSimPlayer != null)
            {
                var sim = ctx.TargetSimPlayer;
                req.personality["friendly"] = sim.PersonalityType == 1 || sim.SocialChase >= 6;
                req.personality["aggressive"] = sim.PersonalityType == 3 || sim.Troublemaker >= 5;
                req.personality["scholarly"] = sim.LoreChase >= 5;
                req.personality["social"] = sim.SocialChase >= 4 || sim.PersonalityType == 1;
                req.personality["types_in_all_caps"] = sim.TypesInAllCaps;
                req.personality["types_in_third_person"] = sim.TypesInThirdPerson;
                req.personality["casual"] = sim.TypoRate > 0.1f;
            }
            else if (ctx.TargetSimTracking != null)
            {
                // Cross-zone: build personality from tracking data.
                // SimPlayerTracking uses "Personality" (not PersonalityType), and
                // doesn't store TypesInAllCaps/TypesInThirdPerson/TypoRate (those
                // are on the live SimPlayer component from its prefab's SimPlayerLanguage).
                var t = ctx.TargetSimTracking;
                req.personality["friendly"] = t.Personality == 1 || t.SocialChase >= 6;
                req.personality["aggressive"] = t.Personality == 3 || t.Troublemaker >= 5;
                req.personality["scholarly"] = t.LoreChase >= 5;
                req.personality["social"] = t.SocialChase >= 4 || t.Personality == 1;
                // Style quirks unavailable for cross-zone -- defaults to none
                req.personality["types_in_all_caps"] = false;
                req.personality["types_in_third_person"] = false;
                req.personality["casual"] = false;
            }

            return req;
        }

        /// <summary>
        /// Checks if a whisper message matches a game-mechanic trigger that
        /// ProcessWhisper should handle natively. These triggers have gameplay
        /// side effects (guild invites, group management, trading, state machine
        /// responses) that we must not bypass.
        ///
        /// Triggers we DEFER to the game:
        ///   - Obscenities (opinion penalty, ignore timer, GM jail)
        ///   - Guild join requests (actual guild invite/join mechanics)
        ///   - Help/group requests (zone travel, group management)
        ///   - Affirmations/declinations (yes/no state machine for pending invites)
        ///   - Gear/slot requests (reads sim's actual equipment)
        ///
        /// Triggers we INTERCEPT with LLM:
        ///   - Greetings, what's up, thank yous, apologies, level, location
        ///   - Anything the game would fall through to "didn't understand"
        /// </summary>
        private static bool IsGameMechanicWhisper(string message)
        {
            if (string.IsNullOrEmpty(message))
                return false;

            string lower = message.ToLower();

            // Guild mechanics -- guild invites, joining, recruitment
            if (lower.Contains("guild") || lower.Contains("join"))
                return true;

            // Group/help mechanics -- triggers zone travel and group formation
            if (lower.Contains("group") || lower.Contains("party") ||
                lower.Contains("help me") || lower.Contains("come to") ||
                lower.Contains("come help") || lower.Contains("lfg") ||
                lower.Contains("looking for"))
                return true;

            // Gear/slot queries -- reads actual sim inventory
            if (lower.Contains("where did you get") ||
                lower.Contains("what are you wearing") ||
                lower.Contains("nice gear") || lower.Contains("your equipment"))
                return true;

            // Short affirmation/declination responses -- these drive the game's
            // state machine (accepting/declining pending guild invites, zone
            // invites, group invites). A bare "yes" or "no" must reach the game.
            if (IsStateMachineResponse(lower))
                return true;

            return false;
        }

        /// <summary>
        /// Detects bare affirmation/declination words that drive the game's
        /// pending-invite state machine. These short responses like "yes", "no",
        /// "ok" are used to accept or decline pending guild, group, and zone
        /// invitations. They MUST reach ProcessWhisper to function.
        /// </summary>
        private static bool IsStateMachineResponse(string lower)
        {
            string trimmed = lower.Trim().TrimEnd('!', '?', '.');
            switch (trimmed)
            {
                case "yes": case "yeah": case "yep": case "yea": case "sure":
                case "ok": case "okay": case "k":
                case "no": case "nah": case "nope": case "never": case "pass":
                    return true;
                default:
                    return false;
            }
        }
    }
}
