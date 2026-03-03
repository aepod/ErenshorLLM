using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Text;
using BepInEx.Logging;
using UnityEngine;
using UnityEngine.SceneManagement;

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
            log.LogInfo("[PersonalityDumper] Coroutine started, waiting for game initialization...");

            // Wait for GameData.SimMngr to be populated
            float waited = 0f;
            while (true)
            {
                try
                {
                    if (GameData.SimMngr != null && GameData.SimMngr.ActualSims != null &&
                        GameData.SimMngr.ActualSims.Count > 0)
                        break;
                }
                catch (Exception ex)
                {
                    log.LogDebug("[PersonalityDumper] Waiting... (" + ex.GetType().Name + ")");
                }

                waited += 1f;
                if (waited > 90f)
                {
                    log.LogWarning("[PersonalityDumper] Timed out (90s) waiting for GameData.SimMngr");
                    yield break;
                }
                yield return new WaitForSeconds(1f);
            }

            log.LogInfo("[PersonalityDumper] SimMngr ready. Waiting for scene load...");

            // Wait until we're past the Menu scene -- sims are fully loaded in-game
            float sceneWait = 0f;
            while (true)
            {
                string sceneName = "";
                try { sceneName = SceneManager.GetActiveScene().name ?? ""; } catch { }
                if (sceneName != "" && sceneName != "Menu" && sceneName != "LoadScene")
                    break;
                sceneWait += 1f;
                if (sceneWait > 120f)
                {
                    log.LogWarning("[PersonalityDumper] Timed out (120s) waiting for game scene");
                    yield break;
                }
                yield return new WaitForSeconds(1f);
            }

            // Wait 10 seconds after scene load so all SimPlayers are fully spawned
            log.LogInfo("[PersonalityDumper] Game scene loaded. Waiting 10s for full SimPlayer init...");
            yield return new WaitForSeconds(10f);

            // Snapshot the ActualSims list to a safe array (avoid concurrent modification)
            GameObject[] actualSims;
            try
            {
                var list = GameData.SimMngr.ActualSims;
                if (list == null || list.Count == 0)
                {
                    log.LogWarning("[PersonalityDumper] ActualSims is empty after wait");
                    yield break;
                }
                actualSims = new GameObject[list.Count];
                list.CopyTo(actualSims);
                log.LogInfo("[PersonalityDumper] Starting full personality dump from " +
                    actualSims.Length + " ActualSims prefabs");
            }
            catch (Exception ex)
            {
                log.LogError("[PersonalityDumper] Failed to snapshot ActualSims: " + ex);
                yield break;
            }

            if (!Directory.Exists(personalitiesDir))
            {
                try { Directory.CreateDirectory(personalitiesDir); }
                catch (Exception ex)
                {
                    log.LogError("[PersonalityDumper] Failed to create dir: " + ex.Message);
                    yield break;
                }
            }

            var sb = new StringBuilder();
            sb.AppendLine("{");

            // --- Dump ActualSims prefab data ---
            sb.AppendLine("  \"actual_sims\": {");
            int simCount = 0;
            var simNames = new List<string>(); // track written names for comma logic

            for (int idx = 0; idx < actualSims.Length; idx++)
            {
                try
                {
                    GameObject simGO = actualSims[idx];
                    if (simGO == null) continue;

                    SimPlayer sp = simGO.GetComponent<SimPlayer>();
                    if (sp == null) continue;

                    // Get sim name -- try NPC component first, then SimPlayerTracking, then GO name
                    string simName = null;
                    try
                    {
                        NPC npc = simGO.GetComponent<NPC>();
                        if (npc != null) simName = npc.NPCName;
                    }
                    catch { }

                    if (string.IsNullOrEmpty(simName))
                    {
                        try
                        {
                            SimPlayerTracking spt = simGO.GetComponent<SimPlayerTracking>();
                            if (spt != null) simName = spt.SimName;
                        }
                        catch { }
                    }

                    if (string.IsNullOrEmpty(simName))
                        simName = simGO.name;

                    if (string.IsNullOrEmpty(simName)) continue;

                    // Comma before new entry (except first)
                    if (simCount > 0) sb.AppendLine(",");
                    simCount++;

                    sb.AppendLine("    \"" + Esc(simName) + "\": {");

                    // -- SimPlayer personality fields --
                    sb.AppendLine("      \"personality\": {");
                    sb.AppendLine("        \"personality_type\": " + Safe(() => sp.PersonalityType) + ",");
                    sb.AppendLine("        \"bio_index\": " + Safe(() => sp.BioIndex) + ",");
                    sb.AppendLine("        \"bio\": \"" + Esc(SafeStr(() => sp.Bio)) + "\",");
                    sb.AppendLine("        \"troublemaker\": " + Safe(() => sp.Troublemaker) + ",");
                    sb.AppendLine("        \"rival\": " + SafeBool(() => sp.Rival) + ",");
                    sb.AppendLine("        \"is_gm_character\": " + SafeBool(() => sp.IsGMCharacter) + ",");
                    sb.AppendLine("        \"lore_chase\": " + Safe(() => sp.LoreChase) + ",");
                    sb.AppendLine("        \"gear_chase\": " + Safe(() => sp.GearChase) + ",");
                    sb.AppendLine("        \"social_chase\": " + Safe(() => sp.SocialChase) + ",");
                    sb.AppendLine("        \"dedication_level\": " + Safe(() => sp.DedicationLevel) + ",");
                    sb.AppendLine("        \"greed\": " + SafeFloat(() => sp.Greed) + ",");
                    sb.AppendLine("        \"patience\": " + Safe(() => sp.Patience) + ",");
                    sb.AppendLine("        \"abbreviates\": " + SafeBool(() => sp.Abbreviates) + ",");
                    sb.AppendLine("        \"caution\": false");
                    sb.AppendLine("      },");

                    // -- Speech modifiers --
                    sb.AppendLine("      \"speech\": {");
                    sb.AppendLine("        \"types_in_all_caps\": " + SafeBool(() => sp.TypesInAllCaps) + ",");
                    sb.AppendLine("        \"types_in_all_lowers\": " + SafeBool(() => sp.TypesInAllLowers) + ",");
                    sb.AppendLine("        \"types_in_third_person\": " + SafeBool(() => sp.TypesInThirdPerson) + ",");
                    sb.AppendLine("        \"typo_rate\": " + SafeFloat(() => sp.TypoRate) + ",");
                    sb.AppendLine("        \"typo_chance\": " + SafeFloat(() => sp.TypoChance) + ",");
                    sb.AppendLine("        \"loves_emojis\": " + SafeBool(() => sp.LovesEmojis) + ",");
                    sb.AppendLine("        \"refers_to_self_as\": \"" + Esc(SafeStr(() => sp.RefersToSelfAs)) + "\",");
                    sb.Append("        \"sign_off_lines\": ");
                    SafeAppendList(sb, () => sp.SignOffLine);
                    sb.AppendLine();
                    sb.AppendLine("      },");

                    // -- Dialog lists from SimPlayerLanguage --
                    sb.AppendLine("      \"dialog\": {");
                    SimPlayerLanguage lang = null;
                    try { lang = simGO.GetComponent<SimPlayerLanguage>(); } catch { }
                    if (lang != null)
                    {
                        SafeAppendDialog(sb, "greetings", () => lang.Greetings, true);
                        SafeAppendDialog(sb, "return_greeting", () => lang.ReturnGreeting, true);
                        SafeAppendDialog(sb, "invites", () => lang.Invites, true);
                        SafeAppendDialog(sb, "justifications", () => lang.Justifications, true);
                        SafeAppendDialog(sb, "confirms", () => lang.Confirms, true);
                        SafeAppendDialog(sb, "generic_lines", () => lang.GenericLines, true);
                        SafeAppendDialog(sb, "aggro", () => lang.Aggro, true);
                        SafeAppendDialog(sb, "died", () => lang.Died, true);
                        SafeAppendDialog(sb, "insults_fun", () => lang.InsultsFun, true);
                        SafeAppendDialog(sb, "retorts_fun", () => lang.RetortsFun, true);
                        SafeAppendDialog(sb, "exclamations", () => lang.Exclamations, true);
                        SafeAppendDialog(sb, "denials", () => lang.Denials, true);
                        SafeAppendDialog(sb, "decline_group", () => lang.DeclineGroup, true);
                        SafeAppendDialog(sb, "negative", () => lang.Negative, true);
                        SafeAppendDialog(sb, "lfg_public", () => lang.LFGPublic, true);
                        SafeAppendDialog(sb, "otw", () => lang.OTW, true);
                        SafeAppendDialog(sb, "goodnight", () => lang.Goodnight, true);
                        SafeAppendDialog(sb, "hello", () => lang.Hello, true);
                        SafeAppendDialog(sb, "local_friend_hello", () => lang.LocalFriendHello, true);
                        SafeAppendDialog(sb, "unsure_response", () => lang.UnsureResponse, true);
                        SafeAppendDialog(sb, "anger_response", () => lang.AngerResponse, true);
                        SafeAppendDialog(sb, "affirms", () => lang.Affirms, true);
                        SafeAppendDialog(sb, "env_dmg", () => lang.EnvDmg, true);
                        SafeAppendDialog(sb, "wants_drop", () => lang.WantsDrop, true);
                        SafeAppendDialog(sb, "gratitude", () => lang.Gratitude, true);
                        SafeAppendDialog(sb, "impressed", () => lang.Impressed, true);
                        SafeAppendDialog(sb, "impressed_end", () => lang.ImpressedEnd, true);
                        SafeAppendDialog(sb, "acknowledge_gratitude", () => lang.AcknowledgeGratitude, true);
                        SafeAppendDialog(sb, "level_up_celebration", () => lang.LevelUpCelebration, true);
                        SafeAppendDialog(sb, "good_last_outing", () => lang.GoodLastOuting, true);
                        SafeAppendDialog(sb, "bad_last_outing", () => lang.BadLastOuting, true);
                        SafeAppendDialog(sb, "got_an_item_last_outing", () => lang.GotAnItemLastOuting, true);
                        SafeAppendDialog(sb, "return_to_zone", () => lang.ReturnToZone, true);
                        SafeAppendDialog(sb, "been_a_while", () => lang.BeenAWhile, true);
                        SafeAppendDialog(sb, "unsure", () => lang.Unsure, false);
                    }
                    sb.AppendLine("      }");

                    // Close this sim
                    sb.Append("    }");
                }
                catch (Exception ex)
                {
                    log.LogWarning("[PersonalityDumper] Error on sim index " + idx + ": " + ex.Message);
                }

                // Yield every 10 sims to avoid frame hitching
                if (idx > 0 && idx % 10 == 0)
                    yield return null;
            }

            sb.AppendLine();
            sb.AppendLine("  },");

            // --- Dump SimPlayerTracking data for all Sims (runtime state) ---
            sb.AppendLine("  \"sim_tracking\": {");
            try
            {
                if (GameData.SimMngr.Sims != null)
                {
                    int trackCount = 0;
                    // Snapshot to avoid concurrent modification
                    var simsList = new List<SimPlayerTracking>(GameData.SimMngr.Sims);
                    for (int i = 0; i < simsList.Count; i++)
                    {
                        try
                        {
                            SimPlayerTracking spt = simsList[i];
                            if (spt == null || string.IsNullOrEmpty(spt.SimName)) continue;

                            if (trackCount > 0) sb.AppendLine(",");
                            trackCount++;

                            sb.AppendLine("    \"" + Esc(spt.SimName) + "\": {");
                            sb.AppendLine("      \"level\": " + Safe(() => spt.Level) + ",");
                            sb.AppendLine("      \"class\": \"" + Esc(SafeStr(() => spt.ClassName)) + "\",");
                            sb.AppendLine("      \"gender\": \"" + Esc(SafeStr(() => spt.Gender) == "" ? "Male" : SafeStr(() => spt.Gender)) + "\",");
                            sb.AppendLine("      \"guild_id\": \"" + Esc(SafeStr(() => spt.GuildID)) + "\",");
                            sb.AppendLine("      \"personality\": " + Safe(() => spt.Personality) + ",");
                            sb.AppendLine("      \"bio_index\": " + Safe(() => spt.BioIndex) + ",");
                            sb.AppendLine("      \"rival\": " + SafeBool(() => spt.Rival) + ",");
                            sb.AppendLine("      \"is_gm_character\": " + SafeBool(() => spt.IsGMCharacter) + ",");
                            sb.AppendLine("      \"troublemaker\": " + Safe(() => spt.Troublemaker) + ",");
                            sb.AppendLine("      \"lore_chase\": " + Safe(() => spt.LoreChase) + ",");
                            sb.AppendLine("      \"gear_chase\": " + Safe(() => spt.GearChase) + ",");
                            sb.AppendLine("      \"social_chase\": " + Safe(() => spt.SocialChase) + ",");
                            sb.AppendLine("      \"dedication_level\": " + Safe(() => spt.DedicationLevel) + ",");
                            sb.AppendLine("      \"greed\": " + SafeFloat(() => spt.Greed) + ",");
                            sb.AppendLine("      \"caution\": " + SafeBool(() => spt.Caution) + ",");
                            sb.AppendLine("      \"opinion_of_player\": " + SafeFloat(() => spt.OpinionOfPlayer) + ",");
                            sb.AppendLine("      \"cur_scene\": \"" + Esc(SafeStr(() => spt.CurScene)) + "\"");
                            sb.Append("    }");
                        }
                        catch (Exception ex)
                        {
                            log.LogWarning("[PersonalityDumper] Error on tracking index " + i + ": " + ex.Message);
                        }
                    }
                }
            }
            catch (Exception ex)
            {
                log.LogWarning("[PersonalityDumper] Error dumping sim tracking: " + ex.Message);
            }
            sb.AppendLine();
            sb.AppendLine("  },");

            // --- Dump global bio description lists ---
            sb.AppendLine("  \"bio_descriptions\": {");
            try
            {
                sb.Append("    \"nice\": ");
                SafeAppendList(sb, () => GameData.SimMngr.NiceDesciptions);
                sb.AppendLine(",");
                sb.Append("    \"tryhard\": ");
                SafeAppendList(sb, () => GameData.SimMngr.TryhardDescriptions);
                sb.AppendLine(",");
                sb.Append("    \"mean\": ");
                SafeAppendList(sb, () => GameData.SimMngr.MeanDescriptions);
                sb.AppendLine();
            }
            catch (Exception ex)
            {
                log.LogWarning("[PersonalityDumper] Error dumping bio descriptions: " + ex.Message);
            }
            sb.AppendLine("  },");

            // --- Dump rival name lists ---
            sb.AppendLine("  \"rival_names\": {");
            try
            {
                sb.Append("    \"male\": ");
                SafeAppendList(sb, () => GameData.SimMngr.RivalMales);
                sb.AppendLine(",");
                sb.Append("    \"female\": ");
                SafeAppendList(sb, () => GameData.SimMngr.RivalFemales);
                sb.AppendLine();
            }
            catch (Exception ex)
            {
                log.LogWarning("[PersonalityDumper] Error dumping rival names: " + ex.Message);
            }
            sb.AppendLine("  },");

            // --- Dump global dialog pools (the "Public" SimPlayerLanguage) ---
            sb.AppendLine("  \"global_dialog\": {");
            try
            {
                SimPlayerLanguage globalLang = GameData.SimLang;
                if (globalLang != null)
                {
                    SafeAppendDialog(sb, "greetings", () => globalLang.Greetings, true);
                    SafeAppendDialog(sb, "return_greeting", () => globalLang.ReturnGreeting, true);
                    SafeAppendDialog(sb, "invites", () => globalLang.Invites, true);
                    SafeAppendDialog(sb, "justifications", () => globalLang.Justifications, true);
                    SafeAppendDialog(sb, "confirms", () => globalLang.Confirms, true);
                    SafeAppendDialog(sb, "generic_lines", () => globalLang.GenericLines, true);
                    SafeAppendDialog(sb, "aggro", () => globalLang.Aggro, true);
                    SafeAppendDialog(sb, "died", () => globalLang.Died, true);
                    SafeAppendDialog(sb, "insults_fun", () => globalLang.InsultsFun, true);
                    SafeAppendDialog(sb, "retorts_fun", () => globalLang.RetortsFun, true);
                    SafeAppendDialog(sb, "exclamations", () => globalLang.Exclamations, true);
                    SafeAppendDialog(sb, "denials", () => globalLang.Denials, true);
                    SafeAppendDialog(sb, "decline_group", () => globalLang.DeclineGroup, true);
                    SafeAppendDialog(sb, "negative", () => globalLang.Negative, true);
                    SafeAppendDialog(sb, "lfg_public", () => globalLang.LFGPublic, true);
                    SafeAppendDialog(sb, "otw", () => globalLang.OTW, true);
                    SafeAppendDialog(sb, "goodnight", () => globalLang.Goodnight, true);
                    SafeAppendDialog(sb, "hello", () => globalLang.Hello, true);
                    SafeAppendDialog(sb, "local_friend_hello", () => globalLang.LocalFriendHello, true);
                    SafeAppendDialog(sb, "unsure_response", () => globalLang.UnsureResponse, true);
                    SafeAppendDialog(sb, "anger_response", () => globalLang.AngerResponse, true);
                    SafeAppendDialog(sb, "affirms", () => globalLang.Affirms, true);
                    SafeAppendDialog(sb, "env_dmg", () => globalLang.EnvDmg, true);
                    SafeAppendDialog(sb, "wants_drop", () => globalLang.WantsDrop, true);
                    SafeAppendDialog(sb, "gratitude", () => globalLang.Gratitude, true);
                    SafeAppendDialog(sb, "impressed", () => globalLang.Impressed, true);
                    SafeAppendDialog(sb, "impressed_end", () => globalLang.ImpressedEnd, true);
                    SafeAppendDialog(sb, "acknowledge_gratitude", () => globalLang.AcknowledgeGratitude, true);
                    SafeAppendDialog(sb, "level_up_celebration", () => globalLang.LevelUpCelebration, true);
                    SafeAppendDialog(sb, "good_last_outing", () => globalLang.GoodLastOuting, true);
                    SafeAppendDialog(sb, "bad_last_outing", () => globalLang.BadLastOuting, true);
                    SafeAppendDialog(sb, "got_an_item_last_outing", () => globalLang.GotAnItemLastOuting, true);
                    SafeAppendDialog(sb, "return_to_zone", () => globalLang.ReturnToZone, true);
                    SafeAppendDialog(sb, "been_a_while", () => globalLang.BeenAWhile, true);
                    SafeAppendDialog(sb, "unsure", () => globalLang.Unsure, false);
                }
            }
            catch (Exception ex)
            {
                log.LogWarning("[PersonalityDumper] Error dumping global dialog: " + ex.Message);
            }
            sb.AppendLine("  }");

            sb.AppendLine("}");

            // Write the dump file
            string dumpPath = Path.Combine(personalitiesDir, "_full_personality_dump.json");
            try
            {
                File.WriteAllText(dumpPath, sb.ToString());
                log.LogInfo("[PersonalityDumper] Full personality dump written to " + dumpPath +
                    " (" + simCount + " ActualSims prefabs, " + sb.Length + " bytes)");
            }
            catch (Exception ex)
            {
                log.LogError("[PersonalityDumper] Failed to write dump: " + ex);
            }
        }

        // --- Safe accessor helpers (catch all exceptions, return defaults) ---

        private static string Safe(Func<int> getter)
        {
            try { return getter().ToString(); }
            catch { return "0"; }
        }

        private static string SafeFloat(Func<float> getter)
        {
            try { return getter().ToString("F1"); }
            catch { return "0.0"; }
        }

        private static string SafeBool(Func<bool> getter)
        {
            try { return getter() ? "true" : "false"; }
            catch { return "false"; }
        }

        private static string SafeStr(Func<string> getter)
        {
            try { return getter() ?? ""; }
            catch { return ""; }
        }

        private static void SafeAppendList(StringBuilder sb, Func<List<string>> getter)
        {
            try
            {
                var list = getter();
                AppendStringList(sb, list);
            }
            catch
            {
                sb.Append("[]");
            }
        }

        private static void SafeAppendDialog(StringBuilder sb, string name,
            Func<List<string>> getter, bool comma)
        {
            sb.Append("        \"" + name + "\": ");
            SafeAppendList(sb, getter);
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
                try
                {
                    sb.Append("\"" + Esc(list[i] ?? "") + "\"");
                }
                catch
                {
                    sb.Append("\"\"");
                }
            }
            sb.Append("]");
        }

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
