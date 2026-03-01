using System.Collections;
using System.Collections.Generic;
using System.IO;
using BepInEx.Logging;
using UnityEngine;

namespace ErenshorLLMDialog.Hooks
{
    /// <summary>
    /// One-shot utility that reads TypesInAllCaps, TypesInThirdPerson, TypoRate,
    /// LovesEmojis, etc. from the game's ActualSims prefabs and merges them into
    /// the sidecar's personality JSON files as a "style_quirks" object.
    ///
    /// These fields are serialized on SimPlayer MonoBehaviours in Unity's asset
    /// bundles and are only accessible at runtime. This dumper captures them
    /// so the sidecar can use them for cross-zone sims that don't have a live
    /// SimPlayer component.
    ///
    /// Usage: Call DumpCoroutine() from the plugin after game init.
    /// Controlled by the "Dump Style Quirks" config toggle.
    /// </summary>
    public static class StyleQuirksDumper
    {
        /// <summary>
        /// Coroutine that waits for GameData.SimMngr to be ready, then dumps
        /// style quirks from ActualSims into personality JSON files.
        /// </summary>
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
                    log.LogWarning("[StyleQuirksDumper] Timed out waiting for GameData.SimMngr");
                    yield break;
                }
                yield return new WaitForSeconds(1f);
            }

            // Extra wait for full initialization
            yield return new WaitForSeconds(2f);

            log.LogInfo("[StyleQuirksDumper] Starting style quirks dump from " +
                GameData.SimMngr.ActualSims.Count + " ActualSims prefabs");

            if (!Directory.Exists(personalitiesDir))
            {
                log.LogWarning("[StyleQuirksDumper] Personalities directory not found: " +
                    personalitiesDir);
                yield break;
            }

            int updated = 0;
            int skipped = 0;
            var allQuirks = new Dictionary<string, QuirkData>();

            foreach (GameObject simGO in GameData.SimMngr.ActualSims)
            {
                if (simGO == null) continue;

                SimPlayer sp = simGO.GetComponent<SimPlayer>();
                NPC npc = simGO.GetComponent<NPC>();
                if (sp == null || npc == null) continue;

                string simName = npc.NPCName;
                if (string.IsNullOrEmpty(simName)) continue;

                var quirk = new QuirkData
                {
                    types_in_all_caps = sp.TypesInAllCaps,
                    types_in_all_lowers = sp.TypesInAllLowers,
                    types_in_third_person = sp.TypesInThirdPerson,
                    typo_rate = sp.TypoRate,
                    loves_emojis = sp.LovesEmojis,
                    refers_to_self_as = sp.RefersToSelfAs ?? ""
                };

                allQuirks[simName] = quirk;

                // Try to update the matching personality JSON
                string jsonPath = Path.Combine(personalitiesDir,
                    simName.ToLower() + ".json");

                if (!File.Exists(jsonPath))
                {
                    skipped++;
                    continue;
                }

                if (MergeStyleQuirks(jsonPath, quirk, log))
                    updated++;
                else
                    skipped++;
            }

            log.LogInfo("[StyleQuirksDumper] Done. Updated: " + updated +
                ", Skipped (no matching file or error): " + skipped +
                ", Total prefabs scanned: " + allQuirks.Count);

            // Also write a full dump file for reference
            string dumpPath = Path.Combine(personalitiesDir, "_style_quirks_dump.json");
            WriteDumpFile(dumpPath, allQuirks, log);
        }

        /// <summary>
        /// Reads an existing personality JSON, inserts/replaces the style_quirks
        /// object, and writes it back. Uses simple string manipulation to avoid
        /// requiring a full JSON library dependency.
        /// </summary>
        private static bool MergeStyleQuirks(string jsonPath, QuirkData quirk,
            ManualLogSource log)
        {
            try
            {
                string content = File.ReadAllText(jsonPath);

                // Build the style_quirks JSON block
                string quirksJson = BuildQuirksJson(quirk);

                // Check if style_quirks already exists
                int sqIndex = content.IndexOf("\"style_quirks\"");
                if (sqIndex >= 0)
                {
                    // Find the opening { after "style_quirks":
                    int braceStart = content.IndexOf('{', sqIndex);
                    if (braceStart < 0) return false;

                    // Find the matching closing }
                    int braceEnd = FindMatchingBrace(content, braceStart);
                    if (braceEnd < 0) return false;

                    // Replace the entire style_quirks value
                    string before = content.Substring(0, sqIndex);
                    string after = content.Substring(braceEnd + 1);
                    content = before + "\"style_quirks\": " + quirksJson + after;
                }
                else
                {
                    // Insert before the last closing }
                    int lastBrace = content.LastIndexOf('}');
                    if (lastBrace < 0) return false;

                    // Find the last non-whitespace before the closing brace
                    // to determine if we need a comma
                    string beforeBrace = content.Substring(0, lastBrace).TrimEnd();
                    bool needsComma = beforeBrace.Length > 0 &&
                        beforeBrace[beforeBrace.Length - 1] != '{' &&
                        beforeBrace[beforeBrace.Length - 1] != ',';

                    string insertion = (needsComma ? "," : "") +
                        "\n  \"style_quirks\": " + quirksJson + "\n";
                    content = content.Substring(0, lastBrace) + insertion + "}";
                }

                File.WriteAllText(jsonPath, content);
                return true;
            }
            catch (System.Exception ex)
            {
                log.LogWarning("[StyleQuirksDumper] Failed to update " + jsonPath +
                    ": " + ex.Message);
                return false;
            }
        }

        private static string BuildQuirksJson(QuirkData q)
        {
            return "{\n" +
                "    \"types_in_all_caps\": " + BoolStr(q.types_in_all_caps) + ",\n" +
                "    \"types_in_all_lowers\": " + BoolStr(q.types_in_all_lowers) + ",\n" +
                "    \"types_in_third_person\": " + BoolStr(q.types_in_third_person) + ",\n" +
                "    \"typo_rate\": " + q.typo_rate.ToString("F2") + ",\n" +
                "    \"loves_emojis\": " + BoolStr(q.loves_emojis) + ",\n" +
                "    \"refers_to_self_as\": \"" + EscapeJson(q.refers_to_self_as) + "\"\n" +
                "  }";
        }

        private static string BoolStr(bool v) => v ? "true" : "false";

        private static string EscapeJson(string s)
        {
            if (string.IsNullOrEmpty(s)) return "";
            return s.Replace("\\", "\\\\").Replace("\"", "\\\"");
        }

        private static int FindMatchingBrace(string text, int openPos)
        {
            int depth = 0;
            for (int i = openPos; i < text.Length; i++)
            {
                if (text[i] == '{') depth++;
                else if (text[i] == '}')
                {
                    depth--;
                    if (depth == 0) return i;
                }
            }
            return -1;
        }

        private static void WriteDumpFile(string path, Dictionary<string, QuirkData> allQuirks,
            ManualLogSource log)
        {
            try
            {
                var sb = new System.Text.StringBuilder();
                sb.AppendLine("{");
                int count = 0;
                foreach (var kvp in allQuirks)
                {
                    count++;
                    sb.Append("  \"" + kvp.Key + "\": " + BuildQuirksJson(kvp.Value));
                    if (count < allQuirks.Count) sb.AppendLine(",");
                    else sb.AppendLine();
                }
                sb.AppendLine("}");
                File.WriteAllText(path, sb.ToString());
                log.LogInfo("[StyleQuirksDumper] Full dump written to " + path);
            }
            catch (System.Exception ex)
            {
                log.LogWarning("[StyleQuirksDumper] Failed to write dump: " + ex.Message);
            }
        }

        private struct QuirkData
        {
            public bool types_in_all_caps;
            public bool types_in_all_lowers;
            public bool types_in_third_person;
            public float typo_rate;
            public bool loves_emojis;
            public string refers_to_self_as;
        }
    }
}
