using HarmonyLib;

namespace ErenshorLLMDialog.Hooks
{
    [HarmonyPatch(typeof(SimPlayerShoutParse), "ParseSay")]
    public class ParseSayPatch
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

            // If pipeline has a pending transform for this say, suppress game processing
            // and inject our own response instead
            if (ErenshorLLMDialogPlugin.Pipeline.HasPendingTransform())
            {
                ErenshorLLMDialogPlugin.Pipeline.ExecutePendingTransform();
                return false; // suppress game's say processing
            }

            return true; // let game process normally
        }
    }
}
