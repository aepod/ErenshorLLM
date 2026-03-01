using HarmonyLib;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Patches SimPlayerMngr.SimReceiveMsg to suppress the game's canned whisper
    /// response when our LLM pipeline is handling the message.
    ///
    /// The game's SimReceiveMsg:
    ///   1. Finds the target sim (case-insensitive)
    ///   2. Validates (not GM, not ignored)
    ///   3. Shows "[WHISPER TO] target: msg" in chat
    ///   4. Starts ProcessWhisper coroutine (generates canned response)
    ///
    /// We want to keep steps 1-3 (validation + echo) but suppress step 4
    /// when our pipeline has a pending whisper transform. We do this by showing
    /// the echo ourselves and returning false to skip the original method entirely.
    /// </summary>
    [HarmonyPatch(typeof(SimPlayerMngr), "SimReceiveMsg")]
    public class SimReceiveMsgPatch
    {
        static bool Prefix(SimPlayerMngr __instance, string targ, string incomingMsg)
        {
            if (ErenshorLLMDialogPlugin.EnableLLMDialog == null ||
                ErenshorLLMDialogPlugin.EnableLLMDialog.Value != Toggle.On)
                return true;

            if (ErenshorLLMDialogPlugin.Pipeline == null)
                return true;

            // Only suppress when we have a pending whisper transform
            if (!ErenshorLLMDialogPlugin.Pipeline.HasPendingWhisper())
                return true;

            // Validate the target exists (same as game does)
            SimPlayerTracking found = null;
            foreach (SimPlayerTracking sim in GameData.SimMngr.Sims)
            {
                if (sim.SimName.ToLower() == targ.ToLower())
                {
                    found = sim;
                    break;
                }
            }

            if (found == null)
            {
                UpdateSocialLog.LogAdd(targ + " is not currently online.", "#FF62D1");
                return false;
            }

            if (found.IsGMCharacter)
            {
                UpdateSocialLog.LogAdd("[SERVER] Please do not send direct messages to game staff.", "yellow");
                return false;
            }

            // Show the whisper echo (same format as game)
            UpdateSocialLog.LogAdd("[WHISPER TO] " + targ + ": " + incomingMsg, "#FF62D1");

            // Track last whisper target for game's reply feature
            GameData.TextInput.LastPlayerMsg = found.SimName;

            // Skip the original method (suppresses ProcessWhisper / canned response)
            return false;
        }
    }
}
