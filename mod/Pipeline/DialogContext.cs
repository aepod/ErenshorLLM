using System.Collections.Generic;

namespace ErenshorLLMDialog.Pipeline
{
    public class DialogContext
    {
        // Input
        public string PlayerMessage { get; set; } = "";
        public ChatChannel Channel { get; set; } = ChatChannel.None;

        // Target
        public SimPlayerTracking TargetSimTracking { get; set; }
        public SimPlayer TargetSimPlayer { get; set; }
        public string TargetNPCName { get; set; } = "";
        public float TargetDistance { get; set; } = -1f;

        // Target guild
        public string TargetGuild { get; set; } = "";
        public bool TargetIsRival { get; set; }

        // Player
        public string PlayerName { get; set; } = "";
        public int PlayerLevel { get; set; }
        public string PlayerClass { get; set; } = "";
        public string PlayerGuild { get; set; } = "";

        // World
        public string CurrentZone { get; set; } = "";
        public List<string> GroupMembers { get; } = new List<string>();
        public List<string> NearbySimPlayers { get; } = new List<string>();

        /// <summary>
        /// Guild members for multi-sim guild responses.
        /// Populated by GameContextSampler for guild channel messages.
        /// </summary>
        public List<string> GuildSimNames { get; } = new List<string>();

        // Response
        public string GameDefaultResponse { get; set; } = "";
        public string TransformedResponse { get; set; } = "";
        public bool Handled { get; set; }

        /// <summary>
        /// When set, this context is for an additional responder (not the primary).
        /// The primary context is referenced for conversation threading.
        /// </summary>
        public DialogContext PrimaryContext { get; set; }

        /// <summary>
        /// The message this sim is responding to. For additional responders in
        /// sim-to-sim chaining, this is the previous sim's response text.
        /// </summary>
        public string RespondingTo { get; set; } = "";

        /// <summary>
        /// When true, indicates the transform is async (e.g., RuVectorTransform).
        /// The pipeline will suppress ParseSay but skip synchronous output,
        /// letting the async transform handle output when the response arrives.
        /// </summary>
        public bool IsAsync { get; set; }

        // Debug
        public List<string> PipelineLog { get; } = new List<string>();
    }
}
