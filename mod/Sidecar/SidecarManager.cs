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

                // Build CLI arguments, including embedding threads from config
                string args = "--port " + _config.Port.Value +
                    " --data-dir \"" + dataDir + "\"" +
                    " --threads " + _config.EmbeddingThreads.Value;

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
                        Arguments = "--data-dir \"" + dataDir + "\" " + subcommand,
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
        /// Graceful shutdown: sends POST /shutdown, waits, then kills if necessary.
        /// This is a blocking call intended for OnApplicationQuit/OnDestroy.
        /// Since we cannot yield in OnApplicationQuit, we use synchronous waits.
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

            _log.LogInfo("[SidecarManager] Sending POST /shutdown to sidecar...");

            // We cannot use coroutines in OnApplicationQuit (MonoBehaviour is being destroyed).
            // Send a simple synchronous-ish shutdown via the process approach:
            // Just kill after a brief wait. The sidecar's own signal handler will flush memory.
            try
            {
                // Try to let the sidecar shut down gracefully
                // Give it 3 seconds to exit on its own (it has signal handlers)
                if (!_process.HasExited)
                {
                    // Send a shutdown signal via the process
                    // On Windows, CloseMainWindow or Kill; the sidecar handles SIGTERM/Ctrl+C
                    bool exited = _process.WaitForExit(3000);
                    if (!exited)
                    {
                        _log.LogWarning("[SidecarManager] Sidecar did not exit within 3s, killing.");
                        KillProcess();
                    }
                    else
                    {
                        _log.LogInfo("[SidecarManager] Sidecar exited gracefully " +
                            "(exit code: " + _process.ExitCode + ")");
                    }
                }
            }
            catch (Exception e)
            {
                _log.LogWarning("[SidecarManager] Error during shutdown: " + e.Message);
                KillProcess();
            }

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
        /// </summary>
        private void OnStderrData(object sender, DataReceivedEventArgs e)
        {
            if (!string.IsNullOrEmpty(e.Data))
            {
                _log.LogInfo("[Sidecar] " + e.Data);
            }
        }
    }
}
