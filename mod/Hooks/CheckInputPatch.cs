using HarmonyLib;

namespace ErenshorLLMDialog.Hooks
{
    [HarmonyPatch(typeof(TypeText), "CheckInput")]
    public class CheckInputPatch
    {
        static void Prefix(TypeText __instance)
        {
            if (ErenshorLLMDialogPlugin.EnableLLMDialog == null ||
                ErenshorLLMDialogPlugin.EnableLLMDialog.Value != Toggle.On)
                return;

            string text = __instance.typed.text;
            if (string.IsNullOrEmpty(text))
                return;

            ErenshorLLMDialogPlugin.Pipeline.Observe(__instance);
        }
    }
}
