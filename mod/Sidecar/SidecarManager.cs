using System;
using System.Collections;
using System.Diagnostics;
using System.IO;
using BepInEx.Logging;
using UnityEngine;

namespace ErenshorLLMDialog.Sidecar
{
    /// <summary>
    /// Status of the sidecar process.
    /// </summary>
    public enum SidecarStatus
    {
        NotStarted,
        Starting,
        Healthy,
        Unhealthy,
        Stopped,
        Disabled
    }

    /// <summary>
    /// Manages the Rust sidecar process lifecycle:
    /// spawn, health polling, restart on crash, and graceful shutdown.
    /// Uses Unity coroutines -- must be driven by a MonoBehaviour host.
    /// </summary>
    public class SidecarManager
    {
        private Process _process;
        private readonly SidecarConfig _config;
        private readonly SidecarClient _client;
        private readonly ManualLogSource _log;
        private readonly MonoBehaviour _coroutineHost;

        private int _restartCount;
        private readonly int _maxRestarts;
        private readonly float _startupTimeout;
        private bool _shutdownRequested;

        public SidecarStatus Status { get; private set; } = SidecarStatus.NotStarted;
        public bool IsHealthy => Status == SidecarStatus.Healthy;

        // Restart backoff: 2s, 4s, 8s, then cap at 8s for any further attempts
        private static readonly float[] RestartBackoffs = { 2f, 4f, 8f };

        public SidecarManager(SidecarConfig config, SidecarClient client,
            ManualLogSource log, MonoBehaviour coroutineHost,
            int maxRestarts = 3, float startupTimeout = 30f)
        {
            _config = config;
            _client = client;
            _log = log;
            _coroutineHost = coroutineHost;
            _maxRestarts = maxRestarts;
            _startupTimeout = startupTimeout;
        }

        /// <summary>
        /// Resolves the sidecar binary path from config or default locations.
        /// Returns null if the binary cannot be found.
        /// </summary>
        private string ResolveBinaryPath()
        {
            string binaryPath = _config.BinaryPath.Value;
            if (string.IsNullOrEmpty(binaryPath))
            {
                string dllDir = Path.GetDirectoryName(
                    System.Reflection.Assembly.GetExecutingAssembly().Location);
                string pluginDir = Path.Combine(dllDir, "ErenshorLLMDialog");
                binaryPath = Path.Combine(pluginDir, "erenshor-llm.exe");

                if (!File.Exists(binaryPath))
                    binaryPath = Path.Combine(dllDir, "erenshor-llm.exe");
            }

            return File.Exists(binaryPath) ? binaryPath : null;
        }

        /// <summary>
        /// Resolves the data directory from config or default (data/ next to binary).
        /// </summary>
        private string ResolveDataDir(string binaryPath)
        {
            string dataDir = _config.DataDir.Value;
            if (string.IsNullOrEmpty(dataDir))
            {
                dataDir = Path.Combine(Path.GetDirectoryName(binaryPath), "data");
            }
            return dataDir;
        }

        /// <summary>
        /// Starts the sidecar process and begins startup health polling.
        /// Call this from Awake() or Start() on the plugin.
        /// </summary>
        public void Start()
        {
            if (Status == SidecarStatus.Disabled)
            {
                _log.LogWarning("[SidecarManager] Sidecar is disabled, not starting.");
                return;
            }

            string binaryPath = ResolveBinaryPath();

            if (binaryPath == null)
            {
                _log.LogError("[SidecarManager] Sidecar binary not found.");
                _log.LogError("[SidecarManager] RuVectorTransform will be disabled. " +
                    "Place erenshor-llm.exe in the ErenshorLLMDialog plugin folder.");
                Status = SidecarStatus.Disabled;
                return;
            }

            _log.LogInfo("[SidecarManager] Starting sidecar: " + binaryPath);
            Status = SidecarStatus.Starting;

            try
            {
                string dataDir = ResolveDataDir(binaryPath);

                // Build CLI arguments, including embedding threads from config.
                // --log-format plain disables ANSI codes and timestamps for clean BepInEx forwarding.
                string args = "--port " + _config.Port.Value +
                    " --data-dir \"" + dataDir + "\"" +
                    " --threads " + _config.EmbeddingThreads.Value +
                    " --log-format plain";

                var psi = new ProcessStartInfo
                {
                    FileName = binaryPath,
                    Arguments = args,
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    RedirectStandardError = true,
                    RedirectStandardOutput = false
                };

                _process = Process.Start(psi);

                if (_process == null || _process.HasExited)
                {
                    _log.LogError("[SidecarManager] Failed to start sidecar process.");
                    Status = SidecarStatus.Disabled;
                    return;
                }

                // Ensure sidecar is killed if the game crashes or is force-closed
                ChildProcessJob.AssignProcess(_process);

                // Begin reading stderr in background and forwarding to BepInEx log
                _process.ErrorDataReceived += OnStderrData;
                _process.BeginErrorReadLine();

                // Start the startup health check coroutine (polls every 500ms for up to configured timeout)
                _coroutineHost.StartCoroutine(WaitForHealthy(_startupTimeout, 0.5f));
            }
            catch (Exception e)
            {
                _log.LogError("[SidecarManager] Exception starting sidecar: " + e.Message);
                Status = SidecarStatus.Disabled;
            }
        }

        /// <summary>
        /// Polls /health until the sidecar reports "ready" or the timeout expires.
        /// </summary>
        private IEnumerator WaitForHealthy(float timeoutSeconds, float pollInterval)
        {
            float elapsed = 0f;
            bool healthy = false;

            while (elapsed < timeoutSeconds)
            {
                if (_process == null || _process.HasExited)
                {
                    _log.LogError("[SidecarManager] Sidecar process exited during startup " +
                        "(exit code: " + (_process?.ExitCode.ToString() ?? "unknown") + ")");
                    Status = SidecarStatus.Stopped;
                    yield break;
                }

                yield return _client.HealthCheck(resp =>
                {
                    if (resp != null && resp.IsReady)
                    {
                        healthy = true;
                    }
                });

                if (healthy)
                {
                    _log.LogInfo("[SidecarManager] Sidecar is healthy (v" +
                        "ersion check via /health). Startup took " +
                        elapsed.ToString("F1") + "s");
                    Status = SidecarStatus.Healthy;
                    _restartCount = 0; // Reset restart counter on successful start
                    yield break;
                }

                yield return new WaitForSeconds(pollInterval);
                elapsed += pollInterval;
            }

            _log.LogError("[SidecarManager] Sidecar failed to become healthy within " +
                timeoutSeconds + " seconds. Killing process.");
            KillProcess();
            Status = SidecarStatus.Unhealthy;
        }

        /// <summary>
        /// Called periodically to check sidecar health.
        /// If the process has exited unexpectedly, attempts a restart.
        /// </summary>
        public void HealthPoll()
        {
            if (Status == SidecarStatus.Disabled || Status == SidecarStatus.NotStarted ||
                _shutdownRequested)
                return;

            if (_process == null)
                return;

            // Check if process has exited unexpectedly
            if (_process.HasExited)
            {
                if (_shutdownRequested)
                    return;

                _log.LogWarning("[SidecarManager] Sidecar process exited unexpectedly " +
                    "(exit code: " + _process.ExitCode + ")");
                Status = SidecarStatus.Stopped;

                if (_restartCount < _maxRestarts)
                {
                    int backoffIdx = _restartCount < RestartBackoffs.Length
                        ? _restartCount
                        : RestartBackoffs.Length - 1;
                    float backoff = RestartBackoffs[backoffIdx];
                    _restartCount++;
                    _log.LogInfo("[SidecarManager] Attempting restart " + _restartCount +
                        "/" + _maxRestarts + " in " + backoff + "s...");
                    _coroutineHost.StartCoroutine(RestartAfterDelay(backoff));
                }
                else
                {
                    _log.LogError("[SidecarManager] Max restarts (" + _maxRestarts +
                        ") exhausted. Disabling sidecar for this session.");
                    Status = SidecarStatus.Disabled;
                }
                return;
            }

            // Process is running -- do an HTTP health check
            _coroutineHost.StartCoroutine(RuntimeHealthCheck());
        }

        /// <summary>
        /// Performs a single runtime health check via HTTP.
        /// </summary>
        private IEnumerator RuntimeHealthCheck()
        {
            yield return _client.HealthCheck(resp =>
            {
                if (resp != null && resp.IsReady)
                {
                    if (Status != SidecarStatus.Healthy)
                    {
                        _log.LogInfo("[SidecarManager] Sidecar recovered, now healthy.");
                    }
                    Status = SidecarStatus.Healthy;
                }
                else
                {
                    if (Status == SidecarStatus.Healthy)
                    {
                        _log.LogWarning("[SidecarManager] Health check failed, marking unhealthy.");
                    }
                    Status = SidecarStatus.Unhealthy;
                }
            });
        }

        /// <summary>
        /// Waits for the backoff delay, then attempts to restart the sidecar.
        /// </summary>
        private IEnumerator RestartAfterDelay(float delaySeconds)
        {
            yield return new WaitForSeconds(delaySeconds);

            if (_shutdownRequested)
                yield break;

            _process = null;
            Start();
        }

        /// <summary>
        /// Runs erenshor-llm.exe build-index and build-responses to rebuild vector indexes.
        /// This is a blocking call intended to run before the daemon starts.
        /// Returns true if both commands succeed.
        /// </summary>
        public bool RebuildIndexes()
        {
            string binaryPath = ResolveBinaryPath();
            if (binaryPath == null)
            {
                _log.LogError("[SidecarManager] Cannot rebuild indexes: sidecar binary not found.");
                return false;
            }

            string dataDir = ResolveDataDir(binaryPath);
            string[] subcommands = { "build-index", "build-responses" };

            foreach (string subcommand in subcommands)
            {
                _log.LogInfo("[SidecarManager] Running: " + subcommand + "...");

                try
                {
                    var psi = new ProcessStartInfo
                    {
                        FileName = binaryPath,
                        Arguments = "--data-dir \"" + dataDir + "\" --log-format plain " + subcommand,
                        UseShellExecute = false,
                        CreateNoWindow = true,
                        RedirectStandardError = true,
                        RedirectStandardOutput = false
                    };

                    var proc = Process.Start(psi);
                    if (proc == null)
                    {
                        _log.LogError("[SidecarManager] Failed to start " + subcommand + " process.");
                        return false;
                    }

                    proc.ErrorDataReceived += OnStderrData;
                    proc.BeginErrorReadLine();

                    bool exited = proc.WaitForExit(60000);
                    if (!exited)
                    {
                        _log.LogError("[SidecarManager] " + subcommand + " timed out after 60s, killing.");
                        try { proc.Kill(); } catch { }
                        return false;
                    }

                    if (proc.ExitCode != 0)
                    {
                        _log.LogError("[SidecarManager] " + subcommand +
                            " failed with exit code " + proc.ExitCode);
                        return false;
                    }

                    _log.LogInfo("[SidecarManager] " + subcommand + " completed successfully.");
                }
                catch (Exception e)
                {
                    _log.LogError("[SidecarManager] Exception running " + subcommand + ": " + e.Message);
                    return false;
                }
            }

            return true;
        }

        /// <summary>
        /// Restarts the sidecar process. Kills the current process and starts a new one.
        /// Resets restart counter and shutdown flag so the new process is fully managed.
        /// </summary>
        public void Restart()
        {
            _log.LogInfo("[SidecarManager] Restart requested.");

            // Kill existing process
            _shutdownRequested = false;
            if (_process != null && !_process.HasExited)
            {
                _log.LogInfo("[SidecarManager] Killing current sidecar process...");
                KillProcess();
            }

            _process = null;
            _restartCount = 0;
            Status = SidecarStatus.NotStarted;

            // Re-start
            Start();
        }

        /// <summary>
        /// Shutdown: kills the sidecar process immediately.
        /// Called from OnApplicationQuit where Unity may terminate us at any moment,
        /// so we kill immediately rather than waiting.
        /// </summary>
        public void Stop()
        {
            if (_shutdownRequested)
                return;
            _shutdownRequested = true;

            if (_process == null || _process.HasExited)
            {
                _log.LogInfo("[SidecarManager] Sidecar already stopped.");
                Status = SidecarStatus.Stopped;
                return;
            }

            _log.LogInfo("[SidecarManager] Killing sidecar...");
            KillProcess();
            _process = null;
            Status = SidecarStatus.Stopped;
        }

        /// <summary>
        /// Force-kills the sidecar process.
        /// </summary>
        private void KillProcess()
        {
            try
            {
                if (_process != null && !_process.HasExited)
                {
                    _process.Kill();
                    _log.LogInfo("[SidecarManager] Sidecar process killed.");
                }
            }
            catch (Exception e)
            {
                _log.LogWarning("[SidecarManager] Error killing sidecar: " + e.Message);
            }
        }

        /// <summary>
        /// Handles stderr output from the sidecar process, forwarding to BepInEx log.
        ///
        /// The sidecar is launched with --log-format plain, which outputs:
        ///   "LEVEL module: message"   (e.g. " INFO erenshor_llm: Server started")
        /// No ANSI codes, no timestamps.
        ///
        /// In debug mode: forwards everything verbatim.
        /// In info mode: parses level and module, maps to BepInEx log levels.
        /// </summary>
        private void OnStderrData(object sender, DataReceivedEventArgs e)
        {
            if (string.IsNullOrEmpty(e.Data))
                return;

            bool debug = ErenshorLLMDialogPlugin.DebugLogging != null &&
                ErenshorLLMDialogPlugin.DebugLogging.Value == Toggle.On;

            if (debug)
            {
                _log.LogInfo("[Sidecar] " + e.Data);
                return;
            }

            // Plain format from sidecar: " INFO erenshor_llm: message"
            // The level is right-padded with spaces (e.g. " INFO", " WARN", "ERROR").
            // StripAnsi is kept as a safety net in case pretty format leaks through.
            string line = StripAnsi(e.Data).TrimStart();

            // Parse "LEVEL module: message" or "LEVEL module::sub: message"
            string level = null;
            string module = null;
            string message = line;

            // Try to extract the level (first whitespace-delimited token)
            int spaceIdx = line.IndexOf(' ');
            if (spaceIdx > 0)
            {
                string candidate = line.Substring(0, spaceIdx).Trim();
                if (candidate == "INFO" || candidate == "WARN" || candidate == "ERROR" ||
                    candidate == "DEBUG" || candidate == "TRACE")
                {
                    level = candidate;
                    string rest = line.Substring(spaceIdx + 1).TrimStart();

                    // Extract module name (before ": ")
                    int colonIdx = rest.IndexOf(": ");
                    if (colonIdx > 0)
                    {
                        module = rest.Substring(0, colonIdx).Trim();
                        message = rest.Substring(colonIdx + 2);
                    }
                    else
                    {
                        message = rest;
                    }
                }
                else
                {
                    // Might be a timestamp-prefixed line from pretty format (fallback).
                    // Try to find level after a "Z " marker.
                    int zIdx = line.IndexOf("Z ");
                    if (zIdx > 0)
                    {
                        string afterTs = line.Substring(zIdx + 2).TrimStart();
                        int sp2 = afterTs.IndexOf(' ');
                        if (sp2 > 0)
                        {
                            string lvl2 = afterTs.Substring(0, sp2).Trim();
                            if (lvl2 == "INFO" || lvl2 == "WARN" || lvl2 == "ERROR" ||
                                lvl2 == "DEBUG" || lvl2 == "TRACE")
                            {
                                level = lvl2;
                                string rest2 = afterTs.Substring(sp2 + 1).TrimStart();
                                int col2 = rest2.IndexOf(": ");
                                if (col2 > 0)
                                {
                                    module = rest2.Substring(0, col2).Trim();
                                    message = rest2.Substring(col2 + 2);
                                }
                                else
                                {
                                    message = rest2;
                                }
                            }
                        }
                    }
                }
            }

            // Route to appropriate BepInEx log level
            string prefix = "[Sidecar]" + (module != null ? "[" + module + "] " : " ");

            if (level == "ERROR")
            {
                _log.LogError(prefix + message);
            }
            else if (level == "WARN")
            {
                _log.LogWarning(prefix + message);
            }
            else if (level == "DEBUG" || level == "TRACE")
            {
                _log.LogDebug(prefix + message);
            }
            else
            {
                _log.LogInfo(prefix + message);
            }
        }

        /// <summary>
        /// Strips ANSI escape sequences from a string.
        /// Safety net for any ANSI codes that might leak through (e.g. if pretty format
        /// is used instead of plain). With --log-format plain this is a no-op pass-through.
        /// </summary>
        private static string StripAnsi(string input)
        {
            // Fast path: if no ESC character, return as-is (common with plain format)
            if (input.IndexOf('\x1b') < 0)
                return input;

            int len = input.Length;
            var sb = new System.Text.StringBuilder(len);

            for (int i = 0; i < len; i++)
            {
                if (input[i] == '\x1b' && i + 1 < len && input[i + 1] == '[')
                {
                    // Skip ESC[ and everything up to the terminating letter
                    i += 2; // skip ESC and [
                    while (i < len && !IsAnsiTerminator(input[i]))
                        i++;
                    // i now points at the terminator letter, loop increment skips it
                    continue;
                }

                sb.Append(input[i]);
            }

            return sb.ToString();
        }

        private static bool IsAnsiTerminator(char c)
        {
            // CSI sequences end with a letter in the range 0x40-0x7E
            return c >= '@' && c <= '~';
        }
    }
}
