namespace ErenshorLLMDialog.Pipeline
{
    public struct ParsedMessage
    {
        public string Speaker;
        public string Body;
        public string Separator;
        public bool IsValid;
    }

    /// <summary>
    /// Parses chat message strings into speaker + body components.
    /// Handles all channel formats: "Name says: text", "Name shouts: text",
    /// "Name tells the group: text", "Name tells the guild: text",
    /// "Name whispers to you, 'text'"
    /// </summary>
    public static class MessageParser
    {
        private static readonly string[] SEPARATORS = new[]
        {
            " tells the group: ",
            " tells the guild: ",
            " shouts: ",
            " says: ",
            " whispers to you, '",
        };

        public static ParsedMessage Parse(string chatString, ChatChannel channelHint = ChatChannel.None)
        {
            if (string.IsNullOrEmpty(chatString))
                return new ParsedMessage { IsValid = false };

            // Strip color tags if present: "<color=#FF9000>text</color>"
            string clean = chatString;
            if (clean.StartsWith("<color="))
            {
                int closeTag = clean.IndexOf('>');
                if (closeTag > 0)
                {
                    clean = clean.Substring(closeTag + 1);
                    if (clean.EndsWith("</color>"))
                        clean = clean.Substring(0, clean.Length - 8);
                }
            }

            foreach (string sep in SEPARATORS)
            {
                int idx = clean.IndexOf(sep);
                if (idx > 0)
                {
                    string speaker = clean.Substring(0, idx);
                    string body = clean.Substring(idx + sep.Length);

                    // Whisper special: strip trailing quote
                    if (sep.Contains("whispers") && body.EndsWith("'"))
                        body = body.Substring(0, body.Length - 1);

                    return new ParsedMessage
                    {
                        Speaker = speaker,
                        Body = body,
                        Separator = sep.Trim(),
                        IsValid = true
                    };
                }
            }

            return new ParsedMessage { IsValid = false };
        }
    }
}
