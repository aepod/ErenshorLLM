namespace ErenshorLLMDialog.Pipeline
{
    /// <summary>
    /// Maps the game's ChatLogLine.LogType [Flags] enum to our ChatChannel enum.
    /// LogType is a bitmask; we check the dialog-relevant bits in priority order.
    /// </summary>
    public static class LogTypeMapper
    {
        public static ChatChannel FromLogType(ChatLogLine.LogType logType)
        {
            // Check dialog types in priority order (most specific first)
            if ((logType & ChatLogLine.LogType.Whisper) != 0) return ChatChannel.Whisper;
            if ((logType & ChatLogLine.LogType.Party) != 0) return ChatChannel.Party;
            if ((logType & ChatLogLine.LogType.Guild) != 0) return ChatChannel.Guild;
            if ((logType & ChatLogLine.LogType.Shout) != 0) return ChatChannel.Shout;
            if ((logType & ChatLogLine.LogType.Say) != 0) return ChatChannel.Say;
            if ((logType & ChatLogLine.LogType.WTB) != 0) return ChatChannel.Trade;
            return ChatChannel.None;
        }

        /// <summary>
        /// Returns true if the LogType represents a dialog channel we should intercept.
        /// Combat hits, system messages, emotes etc. are NOT dialog.
        /// </summary>
        public static bool IsDialogChannel(ChatLogLine.LogType logType)
        {
            var channel = FromLogType(logType);
            return channel != ChatChannel.None;
        }
    }
}
