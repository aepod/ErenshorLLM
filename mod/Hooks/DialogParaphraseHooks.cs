namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Legacy paraphrase hooks (pre-March 2026 update).
    ///
    /// The three Harmony patch classes that were here have been replaced by the
    /// unified ChatInterceptHook which patches UpdateSocialLog.GlobalAddLine.
    /// See ChatInterceptHook.cs for the active implementation.
    ///
    /// Classification and helper logic has been consolidated into
    /// Pipeline/ChatClassifiers.cs for shared use.
    ///
    /// Removed classes:
    /// - AddStringForDisplayPatch (SimPlayerGrouping.AddStringForDisplay)
    /// - LogAddColoredPatch (UpdateSocialLog.LogAdd(string, string))
    /// - LogAddPlainPatch (UpdateSocialLog.LogAdd(string))
    /// </summary>
    public static class DialogParaphraseHooks
    {
        // Kept as empty shell for reference. No active patches.
        // All interception is handled by ChatInterceptHook.Prefix().
    }
}
