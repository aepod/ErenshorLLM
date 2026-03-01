using HarmonyLib;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Patches SimPlayerShoutParse.ParseShout to suppress the game's canned
    /// shout responses when our LLM pipeline is handling the message.
    ///
    /// Only suppresses player shouts (_isPlayer=true). Sim-initiated shouts
    /// (_isPlayer=false) pass through to the game normally.
    ///
    /// Note: NPCShoutListeners (for NPC events triggered by shouts) are called
    /// separately in TypeText.CheckInput AFTER ParseShout, so they are not
    /// affected by this patch.
    /// </summary>
    [HarmonyPatch(typeof(SimPlayerShoutParse), "ParseShout")]
    public class ParseShoutPatch
    {
        static bool Prefix(string _name, string _shout, bool _isPlayer)
        {
            if (!_isPlayer)
                return true;

            if (ErenshorLLMDialogPlugin.EnableLLMDialog == null ||
                ErenshorLLMDialogPlugin.EnableLLMDialog.Value != Toggle.On)
                return true;

            if (ErenshorLLMDialogPlugin.Pipeline == null)
                return true;

            if (ErenshorLLMDialogPlugin.Pipeline.HasPendingTransform())
            {
                ErenshorLLMDialogPlugin.Pipeline.ExecutePendingTransform();
                return false;
            }

            return true;
        }
    }
}
