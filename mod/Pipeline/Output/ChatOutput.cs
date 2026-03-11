using UnityEngine;

namespace ErenshorLLMDialog.Pipeline.Output
{
    public class ChatOutput : IOutputModule
    {
        public void Output(DialogContext ctx)
        {
            if (!ctx.Handled || string.IsNullOrEmpty(ctx.TransformedResponse))
                return;

            if (ctx.Channel == ChatChannel.Say && ctx.TargetSimPlayer != null)
            {
                NPC npc = ctx.TargetSimPlayer.GetComponent<NPC>();
                string simName = npc != null ? npc.NPCName : "SimPlayer";

                // Apply the game's personality system to our response
                string personalized = GameData.SimMngr.PersonalizeString(
                    ctx.TransformedResponse, ctx.TargetSimPlayer);

                // Queue the response through the say system for natural timing.
                // Use typed LogAdd(ChatLogLine) with the context's LogType for correct tab routing.
                // The old LogAdd(string) wraps as SystemMessages which shows in wrong tab.
                string formatted = simName + " says: " + personalized;
                var logType = ctx.LogType != ChatLogLine.LogType.None
                    ? ctx.LogType
                    : ChatLogLine.LogType.Say;
                UpdateSocialLog.LogAdd(new ChatLogLine(formatted, logType));
                UpdateSocialLog.LocalLogAdd(formatted);

                ctx.PipelineLog.Add("[ChatOutput] Injected say response from " + simName);
            }
        }
    }
}
