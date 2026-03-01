using System;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline.Input
{
    public class PlayerChatInput : IInputModule
    {
        public void Process(DialogContext ctx, TypeText typeText)
        {
            string text = typeText.typed.text;
            if (string.IsNullOrEmpty(text))
                return;

            if (text[0] == '/')
            {
                ParseCommand(ctx, text);
            }
            else
            {
                // Say channel
                ctx.Channel = ChatChannel.Say;
                ctx.PlayerMessage = text;

                // Check if player is targeting a SimPlayer within range
                ResolveTarget(ctx);
            }
        }

        private void ParseCommand(DialogContext ctx, string text)
        {
            string lower = text.ToLower();

            if (lower.StartsWith("/whisper ") || lower.StartsWith("/w "))
            {
                ctx.Channel = ChatChannel.Whisper;
                // Extract target name and message: /whisper SimName msg
                string afterCmd = lower.StartsWith("/w ")
                    ? text.Substring(3).TrimStart()
                    : text.Substring(9).TrimStart();

                int spaceIdx = afterCmd.IndexOf(' ');
                if (spaceIdx > 0)
                {
                    string targetName = afterCmd.Substring(0, spaceIdx);
                    ctx.PlayerMessage = afterCmd.Substring(spaceIdx + 1);
                    ctx.TargetNPCName = targetName;

                    // Resolve SimPlayerTracking by name.
                    // FindSimplayerByName is case-sensitive (dict lookup).
                    // Fall back to case-insensitive search through all Sims
                    // so cross-zone whispers and case mismatches work.
                    SimPlayerTracking found = GameData.SimMngr.FindSimplayerByName(targetName);
                    if (found == null && GameData.SimMngr.Sims != null)
                    {
                        string targetLower = targetName.ToLower();
                        foreach (SimPlayerTracking sim in GameData.SimMngr.Sims)
                        {
                            if (sim != null && sim.SimName != null &&
                                sim.SimName.ToLower() == targetLower)
                            {
                                found = sim;
                                break;
                            }
                        }
                    }
                    if (found != null)
                    {
                        ctx.TargetSimTracking = found;
                        ctx.TargetNPCName = found.SimName; // use canonical name
                        if (found.MyAvatar != null)
                            ctx.TargetSimPlayer = found.MyAvatar;
                    }
                }
                else
                {
                    ctx.PlayerMessage = afterCmd;
                }
            }
            else if (lower.StartsWith("/party ") || lower.StartsWith("/group ") || lower.StartsWith("/p "))
            {
                ctx.Channel = ChatChannel.Party;
                if (lower.StartsWith("/p "))
                    ctx.PlayerMessage = text.Substring(3).TrimStart();
                else if (lower.StartsWith("/party "))
                    ctx.PlayerMessage = text.Substring(7).TrimStart();
                else
                    ctx.PlayerMessage = text.Substring(7).TrimStart();
            }
            else if (lower.StartsWith("/guild ") || lower.StartsWith("/g "))
            {
                ctx.Channel = ChatChannel.Guild;
                ctx.PlayerMessage = lower.StartsWith("/g ")
                    ? text.Substring(3).TrimStart()
                    : text.Substring(7).TrimStart();
            }
            else if (lower.StartsWith("/shout "))
            {
                ctx.Channel = ChatChannel.Shout;
                ctx.PlayerMessage = text.Substring(7).TrimStart();
            }
            // Other commands: leave Channel as None so pipeline skips them
        }

        private void ResolveTarget(DialogContext ctx)
        {
            if (GameData.PlayerControl == null || GameData.PlayerControl.CurrentTarget == null)
                return;

            Character target = GameData.PlayerControl.CurrentTarget;
            SimPlayer sim = target.GetComponent<SimPlayer>();
            if (sim == null)
                return;

            float dist = Vector3.Distance(
                GameData.PlayerControl.transform.position,
                target.transform.position);

            ctx.TargetSimPlayer = sim;
            ctx.TargetDistance = dist;

            NPC npc = target.GetComponent<NPC>();
            if (npc != null)
                ctx.TargetNPCName = npc.NPCName;

            // Find matching SimPlayerTracking
            foreach (SimPlayerTracking tracking in GameData.SimMngr.Sims)
            {
                if (tracking.SimName == npc?.NPCName)
                {
                    ctx.TargetSimTracking = tracking;
                    break;
                }
            }
        }
    }
}
