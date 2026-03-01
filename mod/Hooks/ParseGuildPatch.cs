using System.Collections;
using HarmonyLib;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Patches GuildManager.ParseGuildChatInput to suppress the game's canned
    /// guild chat responses when our LLM pipeline is handling the message.
    ///
    /// ParseGuildChatInput is an IEnumerator (coroutine). Returning false from
    /// the Harmony prefix prevents the enumerator from being created, so the
    /// StartCoroutine call in TypeText.CheckInput receives an empty enumerator.
    /// We set __result to an empty coroutine to avoid null reference exceptions.
    /// </summary>
    [HarmonyPatch(typeof(GuildManager), "ParseGuildChatInput")]
    public class ParseGuildPatch
    {
        static bool Prefix(string _input, string _guildID, string _fromPlayer,
            ref IEnumerator __result)
        {
            // Only intercept player guild chat
            if (_fromPlayer != "Player")
                return true;

            if (ErenshorLLMDialogPlugin.EnableLLMDialog == null ||
                ErenshorLLMDialogPlugin.EnableLLMDialog.Value != Toggle.On)
                return true;

            if (ErenshorLLMDialogPlugin.Pipeline == null)
                return true;

            if (ErenshorLLMDialogPlugin.Pipeline.HasPendingTransform())
            {
                ErenshorLLMDialogPlugin.Pipeline.ExecutePendingTransform();
                __result = EmptyCoroutine();
                return false;
            }

            return true;
        }

        private static IEnumerator EmptyCoroutine()
        {
            yield break;
        }
    }
}
