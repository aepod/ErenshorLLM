using System.Collections.Generic;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline.Sample
{
    public class GameContextSampler : ISampleModule
    {
        public void Sample(DialogContext ctx)
        {
            // Player info
            if (GameData.PlayerStats != null)
            {
                ctx.PlayerName = GameData.PlayerStats.MyName ?? "";
                ctx.PlayerLevel = GameData.PlayerStats.Level;
                if (GameData.PlayerStats.CharacterClass != null)
                    ctx.PlayerClass = GameData.PlayerStats.CharacterClass.ClassName ?? "";
            }

            // Player guild
            if (GameData.PlayerControl != null &&
                !string.IsNullOrEmpty(GameData.PlayerControl.MyGuild) &&
                GameData.GuildManager != null)
            {
                ctx.PlayerGuild = GameData.GuildManager.GetGuildNameByID(GameData.PlayerControl.MyGuild) ?? "";
            }

            // World info
            ctx.CurrentZone = GameData.SceneName ?? "";

            // Group members
            for (int i = 0; i < GameData.GroupMembers.Length; i++)
            {
                SimPlayerTracking member = GameData.GroupMembers[i];
                if (member != null && !string.IsNullOrEmpty(member.SimName))
                    ctx.GroupMembers.Add(member.SimName);
            }

            // Nearby SimPlayers
            if (GameData.SimMngr != null && GameData.SimMngr.ActiveSimInstances != null)
            {
                foreach (SimPlayer sim in GameData.SimMngr.ActiveSimInstances)
                {
                    if (sim == null) continue;
                    NPC npc = sim.GetComponent<NPC>();
                    if (npc == null) continue;

                    float dist = Vector3.Distance(
                        GameData.PlayerControl.transform.position,
                        sim.transform.position);

                    if (dist <= 30f)
                        ctx.NearbySimPlayers.Add(npc.NPCName);
                }
            }

            // Whisper target fallback: if we have a target name but no SimPlayer,
            // first try active sim instances (zone-local), then all Sims (cross-zone).
            if (!string.IsNullOrEmpty(ctx.TargetNPCName) && GameData.SimMngr != null)
            {
                string targetLower = ctx.TargetNPCName.ToLower();

                // Try zone-local active instances first (gives us a live SimPlayer)
                if (ctx.TargetSimPlayer == null && GameData.SimMngr.ActiveSimInstances != null)
                {
                    foreach (SimPlayer sim in GameData.SimMngr.ActiveSimInstances)
                    {
                        if (sim == null) continue;
                        NPC npc = sim.GetComponent<NPC>();
                        if (npc == null) continue;

                        if (npc.NPCName.ToLower() == targetLower)
                        {
                            ctx.TargetSimPlayer = sim;
                            ctx.TargetDistance = Vector3.Distance(
                                GameData.PlayerControl.transform.position,
                                sim.transform.position);
                            ctx.PipelineLog.Add("[GameContextSampler] Resolved target from active instances: " + npc.NPCName);
                            break;
                        }
                    }
                }

                // Resolve tracking from all Sims if still missing (cross-zone whisper)
                if (ctx.TargetSimTracking == null && GameData.SimMngr.Sims != null)
                {
                    foreach (SimPlayerTracking tracking in GameData.SimMngr.Sims)
                    {
                        if (tracking != null && tracking.SimName != null &&
                            tracking.SimName.ToLower() == targetLower)
                        {
                            ctx.TargetSimTracking = tracking;
                            ctx.TargetNPCName = tracking.SimName; // canonical name
                            ctx.PipelineLog.Add("[GameContextSampler] Resolved tracking: " + tracking.SimName +
                                " (zone: " + tracking.CurScene + ")");
                            break;
                        }
                    }
                }
            }

            // Guild members for multi-sim guild responses
            if (ctx.Channel == ChatChannel.Guild && GameData.SimMngr != null)
            {
                string playerGuildId = GameData.PlayerControl != null
                    ? GameData.PlayerControl.MyGuild : null;

                if (!string.IsNullOrEmpty(playerGuildId) && GameData.SimMngr.Sims != null)
                {
                    foreach (SimPlayerTracking sim in GameData.SimMngr.Sims)
                    {
                        if (sim != null && sim.GuildID == playerGuildId &&
                            !string.IsNullOrEmpty(sim.SimName))
                            ctx.GuildSimNames.Add(sim.SimName);
                    }
                    ctx.PipelineLog.Add("[GameContextSampler] Guild roster: " +
                        ctx.GuildSimNames.Count + " members");
                }
            }

            // Pick a primary target for guild/shout if none is targeted
            if (ctx.TargetSimPlayer == null && ctx.TargetSimTracking == null)
            {
                if (ctx.Channel == ChatChannel.Guild && ctx.GuildSimNames.Count > 0)
                {
                    // Pick a random guild member as the primary responder
                    string primaryName = ctx.GuildSimNames[
                        Random.Range(0, ctx.GuildSimNames.Count)];
                    foreach (SimPlayerTracking t in GameData.SimMngr.Sims)
                    {
                        if (t != null && t.SimName == primaryName)
                        {
                            ctx.TargetSimTracking = t;
                            ctx.TargetNPCName = t.SimName;
                            if (t.MyAvatar != null)
                                ctx.TargetSimPlayer = t.MyAvatar;
                            ctx.PipelineLog.Add("[GameContextSampler] Guild primary: " +
                                t.SimName);
                            break;
                        }
                    }
                }
                else if (ctx.Channel == ChatChannel.Shout &&
                    GameData.SimMngr != null &&
                    GameData.SimMngr.ActiveSimInstances != null &&
                    GameData.SimMngr.ActiveSimInstances.Count > 0)
                {
                    // Pick a random active sim as the primary responder
                    int idx = Random.Range(0, GameData.SimMngr.ActiveSimInstances.Count);
                    SimPlayer sim = GameData.SimMngr.ActiveSimInstances[idx];
                    if (sim != null && !sim.IsGMCharacter)
                    {
                        ctx.TargetSimPlayer = sim;
                        NPC npc = sim.GetComponent<NPC>();
                        if (npc != null) ctx.TargetNPCName = npc.NPCName;

                        foreach (SimPlayerTracking t in GameData.SimMngr.Sims)
                        {
                            if (t != null && t.SimName == ctx.TargetNPCName)
                            {
                                ctx.TargetSimTracking = t;
                                break;
                            }
                        }
                        ctx.PipelineLog.Add("[GameContextSampler] Shout primary: " +
                            ctx.TargetNPCName);
                    }
                }
            }

            // Target guild info
            if (ctx.TargetSimTracking != null)
            {
                ctx.TargetIsRival = ctx.TargetSimTracking.Rival;
                if (!string.IsNullOrEmpty(ctx.TargetSimTracking.GuildID) &&
                    GameData.GuildManager != null)
                {
                    ctx.TargetGuild = GameData.GuildManager.GetGuildNameByID(
                        ctx.TargetSimTracking.GuildID) ?? "";
                }
            }

            ctx.PipelineLog.Add("[GameContextSampler] Sampled game context");
        }
    }
}
