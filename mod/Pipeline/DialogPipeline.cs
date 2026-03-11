using System.Collections.Generic;
using BepInEx.Logging;

namespace ErenshorLLMDialog.Pipeline
{
    public class DialogPipeline
    {
        private readonly IInputModule _input;
        private readonly ISampleModule _sampler;
        private readonly List<ITransformModule> _transforms;
        private readonly IOutputModule _output;

        // Pending transform state for the two-phase approach:
        // Phase 1: CheckInput prefix observes and builds context
        // Phase 2: ParseSay prefix checks for pending suppression
        private DialogContext _pendingContext;
        private readonly object _lock = new object();

        public DialogPipeline(IInputModule input, ISampleModule sampler,
            List<ITransformModule> transforms, IOutputModule output)
        {
            _input = input;
            _sampler = sampler;
            _transforms = transforms;
            _output = output;
        }

        /// <summary>
        /// Observe and process a player message. Called from CheckInput prefix.
        /// Builds DialogContext, runs transforms. If handled, stores as pending
        /// for ParseSay to suppress.
        /// </summary>
        public void Observe(TypeText typeText)
        {
            lock (_lock)
            {
                _pendingContext = null;
            }

            var ctx = new DialogContext();

            // 1. Input: parse player text, detect channel and target
            _input.Process(ctx, typeText);

            // Map ChatChannel to LogType for correct tab routing on re-injection
            ctx.LogType = ChannelToLogType(ctx.Channel);

            if (ctx.Channel == ChatChannel.None || string.IsNullOrEmpty(ctx.PlayerMessage))
            {
                LogDebug(ctx);
                return;
            }

            // 2. Sample: gather game context
            _sampler.Sample(ctx);

            // 3. Transform chain: first handler wins
            foreach (var transform in _transforms)
            {
                if (transform.Transform(ctx))
                    break;
            }

            // 4. Debug log (always, for all channels)
            LogDebug(ctx);

            // 5. If handled, store pending context for ParseSay suppression
            if (ctx.Handled)
            {
                lock (_lock)
                {
                    _pendingContext = ctx;
                }
            }
        }

        /// <summary>
        /// Check if there's a pending transform that wants to suppress ParseSay.
        /// </summary>
        public bool HasPendingTransform()
        {
            lock (_lock)
            {
                return _pendingContext != null && _pendingContext.Handled;
            }
        }

        /// <summary>
        /// Check if there's a pending whisper transform.
        /// Used by SimReceiveMsgPatch to suppress game's default whisper response.
        /// </summary>
        public bool HasPendingWhisper()
        {
            lock (_lock)
            {
                return _pendingContext != null && _pendingContext.Handled
                    && _pendingContext.Channel == ChatChannel.Whisper;
            }
        }

        /// <summary>
        /// Execute the pending transform's output and clear the state.
        /// For async transforms (IsAsync=true), the suppression of ParseSay
        /// still happens, but output is deferred to the async handler.
        /// </summary>
        public void ExecutePendingTransform()
        {
            DialogContext ctx;
            lock (_lock)
            {
                ctx = _pendingContext;
                _pendingContext = null;
            }

            if (ctx == null) return;

            // Async transforms handle their own output when the response arrives.
            // We still suppress ParseSay, but skip synchronous output here.
            if (ctx.IsAsync)
                return;

            // Synchronous transforms: run output module to inject the response now.
            _output.Output(ctx);
        }

        /// <summary>
        /// Clear pending state (called if ParseSay wasn't reached, e.g., command messages).
        /// </summary>
        public void ClearPending()
        {
            lock (_lock)
            {
                _pendingContext = null;
            }
        }

        /// <summary>
        /// Map our ChatChannel enum to the game's ChatLogLine.LogType for re-injection.
        /// </summary>
        private static ChatLogLine.LogType ChannelToLogType(ChatChannel channel)
        {
            switch (channel)
            {
                case ChatChannel.Say: return ChatLogLine.LogType.Say;
                case ChatChannel.Shout: return ChatLogLine.LogType.Shout;
                case ChatChannel.Whisper: return ChatLogLine.LogType.Whisper;
                case ChatChannel.Party: return ChatLogLine.LogType.Party;
                case ChatChannel.Guild: return ChatLogLine.LogType.Guild;
                case ChatChannel.Trade: return ChatLogLine.LogType.WTB;
                case ChatChannel.Hail: return ChatLogLine.LogType.Say;
                default: return ChatLogLine.LogType.None;
            }
        }

        private void LogDebug(DialogContext ctx)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging == null ||
                ErenshorLLMDialogPlugin.DebugLogging.Value != Toggle.On)
                return;

            ManualLogSource log = ErenshorLLMDialogPlugin.Log;

            log.LogInfo("=== DialogContext ===");
            log.LogInfo("Channel: " + ctx.Channel);
            log.LogInfo("Player: " + ctx.PlayerName +
                " (Level " + ctx.PlayerLevel + " " + ctx.PlayerClass + ")" +
                (string.IsNullOrEmpty(ctx.PlayerGuild) ? "" : " <" + ctx.PlayerGuild + ">"));
            log.LogInfo("Zone: " + ctx.CurrentZone);
            log.LogInfo("Message: \"" + ctx.PlayerMessage + "\"");

            if (ctx.TargetSimPlayer != null || ctx.TargetSimTracking != null)
            {
                string targetInfo = "Target: " + ctx.TargetNPCName;
                if (ctx.TargetSimTracking != null)
                {
                    targetInfo += " (Level " + ctx.TargetSimTracking.Level + " " +
                        ctx.TargetSimTracking.ClassName + ")";
                    if (ctx.TargetSimPlayer == null)
                        targetInfo += " [cross-zone: " + ctx.TargetSimTracking.CurScene + "]";
                }
                if (!string.IsNullOrEmpty(ctx.TargetGuild))
                    targetInfo += " <" + ctx.TargetGuild + ">";
                if (ctx.TargetIsRival)
                    targetInfo += " [RIVAL]";
                if (ctx.TargetDistance >= 0)
                    targetInfo += " - " + ctx.TargetDistance.ToString("F1") + "f away";
                log.LogInfo(targetInfo);

                if (ctx.TargetSimPlayer != null)
                {
                    log.LogInfo("  Personality: TypesInAllCaps=" + ctx.TargetSimPlayer.TypesInAllCaps +
                        ", TypoRate=" + ctx.TargetSimPlayer.TypoRate +
                        ", ThirdPerson=" + ctx.TargetSimPlayer.TypesInThirdPerson);
                }
                else if (ctx.TargetSimTracking != null)
                {
                    log.LogInfo("  Personality: Type=" + ctx.TargetSimTracking.Personality +
                        ", Lore=" + ctx.TargetSimTracking.LoreChase +
                        ", Social=" + ctx.TargetSimTracking.SocialChase);
                }
            }
            else if (!string.IsNullOrEmpty(ctx.TargetNPCName))
            {
                log.LogInfo("Target: " + ctx.TargetNPCName + " [UNRESOLVED]");
            }

            if (ctx.GroupMembers.Count > 0)
                log.LogInfo("Group: [" + string.Join(", ", ctx.GroupMembers) + "]");

            if (ctx.NearbySimPlayers.Count > 0)
                log.LogInfo("Nearby: [" + string.Join(", ", ctx.NearbySimPlayers) + "]");

            foreach (string entry in ctx.PipelineLog)
                log.LogInfo("Pipeline: " + entry);

            log.LogInfo("Handled: " + ctx.Handled);
            if (ctx.Handled)
                log.LogInfo("Response: \"" + ctx.TransformedResponse + "\"");
            log.LogInfo("===================");
        }
    }
}
