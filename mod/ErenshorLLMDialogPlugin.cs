using System.Collections.Generic;
using BepInEx;
using BepInEx.Configuration;
using BepInEx.Logging;
using ErenshorLLMDialog.Pipeline;
using ErenshorLLMDialog.Pipeline.Input;
using ErenshorLLMDialog.Pipeline.Output;
using ErenshorLLMDialog.Pipeline.Sample;
using ErenshorLLMDialog.Pipeline.Transform;
using ErenshorLLMDialog.Hooks;
using ErenshorLLMDialog.Sidecar;
using HarmonyLib;

namespace ErenshorLLMDialog
{
    public enum Toggle
    {
        Off,
        On
    }

    /// <summary>
    /// LLM mode for Phase 3 response generation.
    /// Currently only Off is functional; other modes are placeholders.
    /// </summary>
    public enum LlmMode
    {
        /// <summary>Template-only responses (Phase 2 behavior).</summary>
        Off,
        /// <summary>Use a local GGUF model for response generation.</summary>
        Local,
        /// <summary>Use a cloud LLM endpoint (e.g. OpenRouter) for response generation.</summary>
        Cloud,
        /// <summary>Try local first, fall back to cloud on failure.</summary>
        Hybrid
    }

    [BepInPlugin("aepod.ErenshorLLMDialog", "Erenshor LLM Dialog", "0.2.0")]
    [BepInProcess("Erenshor.exe")]
    public class ErenshorLLMDialogPlugin : BaseUnityPlugin
    {
        internal static DialogPipeline Pipeline;
        internal static ManualLogSource Log;
        internal static ConfigEntry<Toggle> EnableLLMDialog;
        internal static ConfigEntry<Toggle> DebugLogging;

        private SidecarConfig _sidecarConfig;
        private SidecarClient _sidecarClient;
        private SidecarManager _sidecarManager;

        void Awake()
        {
            Log = Logger;

            // --- General config ---
            EnableLLMDialog = Config.Bind(
                "1 - LLM Dialog", "Enable LLM Dialog", Toggle.On,
                "Master toggle for the LLM Dialog pipeline.");
            DebugLogging = Config.Bind(
                "1 - LLM Dialog", "Debug Logging", Toggle.On,
                "Log full DialogContext to BepInEx log for every chat message.");

            // --- Sidecar config (sections 2-5) ---
            _sidecarConfig = new SidecarConfig(Config);
            _sidecarClient = new SidecarClient(
                _sidecarConfig.Port.Value, Log,
                _sidecarConfig.LlmTimeout.Value);
            _sidecarManager = new SidecarManager(
                _sidecarConfig, _sidecarClient, Log, this,
                _sidecarConfig.MaxRestarts.Value,
                _sidecarConfig.StartupTimeout.Value);

            // --- Multi-sim dispatcher (rate-limited additional responders) ---
            var rateLimiter = new RateLimiter(
                _sidecarConfig.MaxRequestsPerMinute.Value, 60f);
            var dispatcher = new MultiSimDispatcher(
                _sidecarClient, _sidecarManager, rateLimiter, this,
                Log, _sidecarConfig);

            // --- Build transform chain: RuVectorTransform first, HelloWorldTransform as fallback ---
            var ruVectorTransform = new RuVectorTransform(
                _sidecarClient, _sidecarManager, this,
                _sidecarConfig.MinConfidence.Value,
                _sidecarConfig.TemplateCandidates.Value,
                _sidecarConfig.LoreContextCount.Value,
                _sidecarConfig.MemoryContextCount.Value,
                _sidecarConfig.SemanticWeight.Value,
                _sidecarConfig.ChannelWeight.Value,
                _sidecarConfig.ZoneWeight.Value,
                _sidecarConfig.PersonalityWeight.Value,
                _sidecarConfig.RelationshipWeight.Value,
                dispatcher);

            Pipeline = new DialogPipeline(
                input: new PlayerChatInput(),
                sampler: new GameContextSampler(),
                transforms: new List<ITransformModule>
                {
                    ruVectorTransform,        // Phase 2: try sidecar first
                    new HelloWorldTransform()  // Phase 1: fallback
                },
                output: new ChatOutput()
            );

            // --- Initialize memory reuptake system ---
            MemoryReuptakeManager.Initialize(_sidecarClient, this, Log);

            // --- Rebuild indexes if requested ---
            if (_sidecarConfig.RebuildIndexes.Value == Toggle.On)
            {
                Log.LogInfo("Rebuild Indexes is enabled. Rebuilding vector indexes...");
                bool success = _sidecarManager.RebuildIndexes();
                if (success)
                {
                    Log.LogInfo("Index rebuild completed successfully.");
                    _sidecarConfig.RebuildIndexes.Value = Toggle.Off;
                }
                else
                {
                    Log.LogError("Index rebuild failed. Check log for details. " +
                        "Toggle will remain On for next retry.");
                }
            }

            // --- Start sidecar if auto-start is enabled ---
            if (_sidecarConfig.AutoStart.Value == Toggle.On)
            {
                _sidecarManager.Start();
            }
            else
            {
                Log.LogInfo("Sidecar auto-start is disabled. RuVectorTransform will " +
                    "not function until the sidecar is started manually.");
            }

            // --- Start periodic health polling using configured interval ---
            float pollInterval = _sidecarConfig.HealthPollInterval.Value;
            InvokeRepeating(nameof(HealthPoll), pollInterval, pollInterval);

            // --- Dump style quirks if requested ---
            if (_sidecarConfig.DumpStyleQuirks.Value == Toggle.On)
            {
                string personalitiesDir = ResolvePersonalitiesDir();
                if (personalitiesDir != null)
                {
                    Log.LogInfo("Style quirks dump enabled. Target: " + personalitiesDir);
                    StartCoroutine(StyleQuirksDumper.DumpCoroutine(personalitiesDir, Log));
                    _sidecarConfig.DumpStyleQuirks.Value = Toggle.Off;
                }
                else
                {
                    Log.LogWarning("Cannot dump style quirks: unable to resolve data directory.");
                }
            }

            // --- Apply Harmony patches ---
            new Harmony("aepod.ErenshorLLMDialog").PatchAll();
            Log.LogInfo("ErenshorLLMDialog v0.2.0 loaded!");
        }

        /// <summary>
        /// Periodic health poll callback, invoked via InvokeRepeating.
        /// </summary>
        void HealthPoll()
        {
            _sidecarManager?.HealthPoll();
        }

        /// <summary>
        /// Called when the game application is quitting.
        /// Ensures the sidecar process is shut down cleanly.
        /// </summary>
        void OnApplicationQuit()
        {
            ShutdownSidecar();
        }

        /// <summary>
        /// Called when the MonoBehaviour is destroyed.
        /// Belt-and-suspenders shutdown in case OnApplicationQuit is not called.
        /// </summary>
        void OnDestroy()
        {
            ShutdownSidecar();
        }

        private void ShutdownSidecar()
        {
            if (_sidecarManager != null)
            {
                CancelInvoke(nameof(HealthPoll));
                _sidecarManager.Stop();
            }
        }

        /// <summary>
        /// Resolves the path to the sidecar's personalities directory.
        /// Checks DataDir config first, then falls back to "data/personalities"
        /// next to the mod DLL.
        /// </summary>
        private string ResolvePersonalitiesDir()
        {
            string dataDir = _sidecarConfig.DataDir.Value;
            if (!string.IsNullOrEmpty(dataDir))
            {
                string dir = System.IO.Path.Combine(dataDir, "personalities");
                if (System.IO.Directory.Exists(dir)) return dir;
            }

            // Fall back: look next to this DLL
            string dllDir = System.IO.Path.GetDirectoryName(
                System.Reflection.Assembly.GetExecutingAssembly().Location);
            if (!string.IsNullOrEmpty(dllDir))
            {
                // Try "data/personalities" next to the DLL
                string dir = System.IO.Path.Combine(dllDir, "data", "personalities");
                if (System.IO.Directory.Exists(dir)) return dir;

                // Try parent directory (if DLL is in plugins/ subfolder)
                string parentDir = System.IO.Path.GetDirectoryName(dllDir);
                if (!string.IsNullOrEmpty(parentDir))
                {
                    dir = System.IO.Path.Combine(parentDir, "data", "personalities");
                    if (System.IO.Directory.Exists(dir)) return dir;
                }
            }

            return null;
        }
    }
}
