using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Text;
using BepInEx.Logging;
using UnityEngine;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// Comprehensive personality data dumper that extracts ALL personality-related
    /// fields from ActualSims prefabs at runtime:
    /// - SimPlayer fields: TypesInAllCaps, TypoRate, Troublemaker, SignOffLine, Bio, etc.
    /// - SimPlayerLanguage dialog lists: Greetings, InsultsFun, Died, etc.
    /// - NPC metadata: class, level, guild, gender, rival status
    /// - Global bio descriptions (Nice, Tryhard, Mean)
    /// - Rival name lists
    ///
    /// Output: personalities/_full_personality_dump.json
    /// This file is used to fix/merge real game personality traits into our
    /// personality JSON files.
    ///
    /// Controlled by the "Dump Full Personalities" config toggle.
    /// </summary>
    public static class PersonalityDumper
    {
        public static IEnumerator DumpCoroutine(string personalitiesDir, ManualLogSource log)
        {
            // Wait for GameData.SimMngr to be populated
            float waited = 0f;
            while (GameData.SimMngr == null || GameData.SimMngr.ActualSims == null ||
                   GameData.SimMngr.ActualSims.Count == 0)
            {
                waited += 1f;
                if (waited > 60f)
                {
                    log.LogWarning("[PersonalityDumper] Timed out waiting for GameData.SimMngr");
                    yield break;
                }
                yield return new WaitForSeconds(1f);
            }

            // Extra wait for full initialization
            yield return new WaitForSeconds(3f);

            log.LogInfo("[PersonalityDumper] Starting full personality dump from " +
                GameData.SimMngr.ActualSims.Count + " ActualSims prefabs");

            if (!Directory.Exists(personalitiesDir))
            {
                Directory.CreateDirectory(personalitiesDir);
            }

            var sb = new StringBuilder();
            sb.AppendLine("{");

            // --- Dump ActualSims prefab data ---
            sb.AppendLine("  \"actual_sims\": {");
            int simCount = 0;
            int totalSims = GameData.SimMngr.ActualSims.Count;

            foreach (GameObject simGO in GameData.SimMngr.ActualSims)
            {
                if (simGO == null) continue;

                SimPlayer sp = simGO.GetComponent<SimPlayer>();
                NPC npc = simGO.GetComponent<NPC>();
                SimPlayerLanguage lang = simGO.GetComponent<SimPlayerLanguage>();
                if (sp == null) continue;

                string simName = npc != null ? npc.NPCName : simGO.name;
                if (string.IsNullOrEmpty(simName)) continue;

                simCount++;
                sb.AppendLine("    \"" + Esc(simName) + "\": {");

                // -- SimPlayer personality fields --
                sb.AppendLine("      \"personality\": {");
                sb.AppendLine("        \"personality_type\": " + sp.PersonalityType + ",");
                sb.AppendLine("        \"bio_index\": " + sp.BioIndex + ",");
                sb.AppendLine("        \"bio\": \"" + Esc(sp.Bio ?? "") + "\",");
                sb.AppendLine("        \"troublemaker\": " + sp.Troublemaker + ",");
                sb.AppendLine("        \"rival\": " + Bool(sp.Rival) + ",");
                sb.AppendLine("        \"is_gm_character\": " + Bool(sp.IsGMCharacter) + ",");
                sb.AppendLine("        \"lore_chase\": " + sp.LoreChase + ",");
                sb.AppendLine("        \"gear_chase\": " + sp.GearChase + ",");
                sb.AppendLine("        \"social_chase\": " + sp.SocialChase + ",");
                sb.AppendLine("        \"dedication_level\": " + sp.DedicationLevel + ",");
                sb.AppendLine("        \"greed\": " + sp.Greed.ToString("F1") + ",");
                sb.AppendLine("        \"patience\": " + sp.Patience + ",");
                sb.AppendLine("        \"abbreviates\": " + Bool(sp.Abbreviates) + ",");
                sb.AppendLine("        \"caution\": false");
                sb.AppendLine("      },");

                // -- Speech modifiers --
                sb.AppendLine("      \"speech\": {");
                sb.AppendLine("        \"types_in_all_caps\": " + Bool(sp.TypesInAllCaps) + ",");
                sb.AppendLine("        \"types_in_all_lowers\": " + Bool(sp.TypesInAllLowers) + ",");
                sb.AppendLine("        \"types_in_third_person\": " + Bool(sp.TypesInThirdPerson) + ",");
                sb.AppendLine("        \"typo_rate\": " + sp.TypoRate.ToString("F2") + ",");
                sb.AppendLine("        \"typo_chance\": " + sp.TypoChance.ToString("F2") + ",");
                sb.AppendLine("        \"loves_emojis\": " + Bool(sp.LovesEmojis) + ",");
                sb.AppendLine("        \"refers_to_self_as\": \"" + Esc(sp.RefersToSelfAs ?? "") + "\",");
                sb.Append("        \"sign_off_lines\": ");
                AppendStringList(sb, sp.SignOffLine);
                sb.AppendLine();
                sb.AppendLine("      },");

                // -- Dialog lists from SimPlayerLanguage --
                sb.AppendLine("      \"dialog\": {");
                if (lang != null)
                {
                    AppendDialogField(sb, "greetings", lang.Greetings, true);
                    AppendDialogField(sb, "return_greeting", lang.ReturnGreeting, true);
                    AppendDialogField(sb, "invites", lang.Invites, true);
                    AppendDialogField(sb, "justifications", lang.Justifications, true);
                    AppendDialogField(sb, "confirms", lang.Confirms, true);
                    AppendDialogField(sb, "generic_lines", lang.GenericLines, true);
                    AppendDialogField(sb, "aggro", lang.Aggro, true);
                    AppendDialogField(sb, "died", lang.Died, true);
                    AppendDialogField(sb, "insults_fun", lang.InsultsFun, true);
                    AppendDialogField(sb, "retorts_fun", lang.RetortsFun, true);
                    AppendDialogField(sb, "exclamations", lang.Exclamations, true);
                    AppendDialogField(sb, "denials", lang.Denials, true);
                    AppendDialogField(sb, "decline_group", lang.DeclineGroup, true);
                    AppendDialogField(sb, "negative", lang.Negative, true);
                    AppendDialogField(sb, "lfg_public", lang.LFGPublic, true);
                    AppendDialogField(sb, "otw", lang.OTW, true);
                    AppendDialogField(sb, "goodnight", lang.Goodnight, true);
                    AppendDialogField(sb, "hello", lang.Hello, true);
                    AppendDialogField(sb, "local_friend_hello", lang.LocalFriendHello, true);
                    AppendDialogField(sb, "unsure_response", lang.UnsureResponse, true);
                    AppendDialogField(sb, "anger_response", lang.AngerResponse, true);
                    AppendDialogField(sb, "affirms", lang.Affirms, true);
                    AppendDialogField(sb, "env_dmg", lang.EnvDmg, true);
                    AppendDialogField(sb, "wants_drop", lang.WantsDrop, true);
                    AppendDialogField(sb, "gratitude", lang.Gratitude, true);
                    AppendDialogField(sb, "impressed", lang.Impressed, true);
                    AppendDialogField(sb, "impressed_end", lang.ImpressedEnd, true);
                    AppendDialogField(sb, "acknowledge_gratitude", lang.AcknowledgeGratitude, true);
                    AppendDialogField(sb, "level_up_celebration", lang.LevelUpCelebration, true);
                    AppendDialogField(sb, "good_last_outing", lang.GoodLastOuting, true);
                    AppendDialogField(sb, "bad_last_outing", lang.BadLastOuting, true);
                    AppendDialogField(sb, "got_an_item_last_outing", lang.GotAnItemLastOuting, true);
                    AppendDialogField(sb, "return_to_zone", lang.ReturnToZone, true);
                    AppendDialogField(sb, "been_a_while", lang.BeenAWhile, true);
                    AppendDialogField(sb, "unsure", lang.Unsure, false);
                }
                sb.AppendLine("      }");

                // Close this sim
                sb.Append("    }");
                if (simCount < totalSims) sb.AppendLine(",");
                else sb.AppendLine();
            }
            sb.AppendLine("  },");

            // --- Dump SimPlayerTracking data for all Sims (runtime state) ---
            sb.AppendLine("  \"sim_tracking\": {");
            if (GameData.SimMngr.Sims != null)
            {
                int trackCount = 0;
                int totalTrack = GameData.SimMngr.Sims.Count;
                foreach (SimPlayerTracking spt in GameData.SimMngr.Sims)
                {
                    if (spt == null || string.IsNullOrEmpty(spt.SimName)) continue;
                    trackCount++;
                    sb.AppendLine("    \"" + Esc(spt.SimName) + "\": {");
                    sb.AppendLine("      \"level\": " + spt.Level + ",");
                    sb.AppendLine("      \"class\": \"" + Esc(spt.ClassName ?? "") + "\",");
                    sb.AppendLine("      \"gender\": \"" + Esc(spt.Gender ?? "Male") + "\",");
                    sb.AppendLine("      \"guild_id\": \"" + Esc(spt.GuildID ?? "") + "\",");
                    sb.AppendLine("      \"personality\": " + spt.Personality + ",");
                    sb.AppendLine("      \"bio_index\": " + spt.BioIndex + ",");
                    sb.AppendLine("      \"rival\": " + Bool(spt.Rival) + ",");
                    sb.AppendLine("      \"is_gm_character\": " + Bool(spt.IsGMCharacter) + ",");
                    sb.AppendLine("      \"troublemaker\": " + spt.Troublemaker + ",");
                    sb.AppendLine("      \"lore_chase\": " + spt.LoreChase + ",");
                    sb.AppendLine("      \"gear_chase\": " + spt.GearChase + ",");
                    sb.AppendLine("      \"social_chase\": " + spt.SocialChase + ",");
                    sb.AppendLine("      \"dedication_level\": " + spt.DedicationLevel + ",");
                    sb.AppendLine("      \"greed\": " + spt.Greed.ToString("F1") + ",");
                    sb.AppendLine("      \"caution\": " + Bool(spt.Caution) + ",");
                    sb.AppendLine("      \"opinion_of_player\": " + spt.OpinionOfPlayer.ToString("F1") + ",");
                    sb.AppendLine("      \"cur_scene\": \"" + Esc(spt.CurScene ?? "") + "\"");
                    sb.Append("    }");
                    if (trackCount < totalTrack) sb.AppendLine(",");
                    else sb.AppendLine();
                }
            }
            sb.AppendLine("  },");

            // --- Dump global bio description lists ---
            sb.AppendLine("  \"bio_descriptions\": {");
            sb.Append("    \"nice\": ");
            AppendStringList(sb, GameData.SimMngr.NiceDesciptions);
            sb.AppendLine(",");
            sb.Append("    \"tryhard\": ");
            AppendStringList(sb, GameData.SimMngr.TryhardDescriptions);
            sb.AppendLine(",");
            sb.Append("    \"mean\": ");
            AppendStringList(sb, GameData.SimMngr.MeanDescriptions);
            sb.AppendLine();
            sb.AppendLine("  },");

            // --- Dump rival name lists ---
            sb.AppendLine("  \"rival_names\": {");
            sb.Append("    \"male\": ");
            AppendStringList(sb, GameData.SimMngr.RivalMales);
            sb.AppendLine(",");
            sb.Append("    \"female\": ");
            AppendStringList(sb, GameData.SimMngr.RivalFemales);
            sb.AppendLine();
            sb.AppendLine("  },");

            // --- Dump global dialog pools (the "Public" SimPlayerLanguage) ---
            sb.AppendLine("  \"global_dialog\": {");
            SimPlayerLanguage globalLang = GameData.SimLang;
            if (globalLang != null)
            {
                AppendDialogField(sb, "greetings", globalLang.Greetings, true);
                AppendDialogField(sb, "return_greeting", globalLang.ReturnGreeting, true);
                AppendDialogField(sb, "invites", globalLang.Invites, true);
                AppendDialogField(sb, "justifications", globalLang.Justifications, true);
                AppendDialogField(sb, "confirms", globalLang.Confirms, true);
                AppendDialogField(sb, "generic_lines", globalLang.GenericLines, true);
                AppendDialogField(sb, "aggro", globalLang.Aggro, true);
                AppendDialogField(sb, "died", globalLang.Died, true);
                AppendDialogField(sb, "insults_fun", globalLang.InsultsFun, true);
                AppendDialogField(sb, "retorts_fun", globalLang.RetortsFun, true);
                AppendDialogField(sb, "exclamations", globalLang.Exclamations, true);
                AppendDialogField(sb, "denials", globalLang.Denials, true);
                AppendDialogField(sb, "decline_group", globalLang.DeclineGroup, true);
                AppendDialogField(sb, "negative", globalLang.Negative, true);
                AppendDialogField(sb, "lfg_public", globalLang.LFGPublic, true);
                AppendDialogField(sb, "otw", globalLang.OTW, true);
                AppendDialogField(sb, "goodnight", globalLang.Goodnight, true);
                AppendDialogField(sb, "hello", globalLang.Hello, true);
                AppendDialogField(sb, "local_friend_hello", globalLang.LocalFriendHello, true);
                AppendDialogField(sb, "unsure_response", globalLang.UnsureResponse, true);
                AppendDialogField(sb, "anger_response", globalLang.AngerResponse, true);
                AppendDialogField(sb, "affirms", globalLang.Affirms, true);
                AppendDialogField(sb, "env_dmg", globalLang.EnvDmg, true);
                AppendDialogField(sb, "wants_drop", globalLang.WantsDrop, true);
                AppendDialogField(sb, "gratitude", globalLang.Gratitude, true);
                AppendDialogField(sb, "impressed", globalLang.Impressed, true);
                AppendDialogField(sb, "impressed_end", globalLang.ImpressedEnd, true);
                AppendDialogField(sb, "acknowledge_gratitude", globalLang.AcknowledgeGratitude, true);
                AppendDialogField(sb, "level_up_celebration", globalLang.LevelUpCelebration, true);
                AppendDialogField(sb, "good_last_outing", globalLang.GoodLastOuting, true);
                AppendDialogField(sb, "bad_last_outing", globalLang.BadLastOuting, true);
                AppendDialogField(sb, "got_an_item_last_outing", globalLang.GotAnItemLastOuting, true);
                AppendDialogField(sb, "return_to_zone", globalLang.ReturnToZone, true);
                AppendDialogField(sb, "been_a_while", globalLang.BeenAWhile, true);
                AppendDialogField(sb, "unsure", globalLang.Unsure, false);
            }
            sb.AppendLine("  }");

            sb.AppendLine("}");

            // Write the dump file
            string dumpPath = Path.Combine(personalitiesDir, "_full_personality_dump.json");
            try
            {
                File.WriteAllText(dumpPath, sb.ToString());
                log.LogInfo("[PersonalityDumper] Full personality dump written to " + dumpPath +
                    " (" + simCount + " ActualSims prefabs)");
            }
            catch (System.Exception ex)
            {
                log.LogError("[PersonalityDumper] Failed to write dump: " + ex.Message);
            }
        }

        private static void AppendDialogField(StringBuilder sb, string name,
            List<string> list, bool comma)
        {
            sb.Append("        \"" + name + "\": ");
            AppendStringList(sb, list);
            if (comma) sb.AppendLine(",");
            else sb.AppendLine();
        }

        private static void AppendStringList(StringBuilder sb, List<string> list)
        {
            if (list == null || list.Count == 0)
            {
                sb.Append("[]");
                return;
            }

            sb.Append("[");
            for (int i = 0; i < list.Count; i++)
            {
                if (i > 0) sb.Append(", ");
                sb.Append("\"" + Esc(list[i] ?? "") + "\"");
            }
            sb.Append("]");
        }

        private static string Bool(bool v) => v ? "true" : "false";

        private static string Esc(string s)
        {
            if (string.IsNullOrEmpty(s)) return "";
            return s.Replace("\\", "\\\\")
                    .Replace("\"", "\\\"")
                    .Replace("\n", "\\n")
                    .Replace("\r", "\\r")
                    .Replace("\t", "\\t");
        }
    }
}
