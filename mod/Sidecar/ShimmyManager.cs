using System;
using System.Diagnostics;
using System.IO;
using System.Net;
using BepInEx.Logging;

namespace ErenshorLLMDialog.Sidecar
{
    /// <summary>
    /// Manages the shimmy local inference server process lifecycle.
    /// Shimmy provides an OpenAI-compatible endpoint for local GGUF model inference.
    /// Only started when LLM mode is Local or Hybrid.
    /// </summary>
    public class ShimmyManager
    {
        private Process _process;
        private readonly SidecarConfig _config;
        private readonly ManualLogSource _log;
        private bool _shutdownRequested;

        public bool IsRunning => _process != null && !_process.HasExited;

        public ShimmyManager(SidecarConfig config, ManualLogSource log)
        {
            _config = config;
            _log = log;
        }

        /// <summary>
        /// Resolves the shimmy binary path (same directory as erenshor-llm.exe).
        /// </summary>
        private string ResolveBinaryPath()
        {
            string dllDir = Path.GetDirectoryName(
                System.Reflection.Assembly.GetExecutingAssembly().Location);
            string pluginDir = Path.Combine(dllDir, "ErenshorLLMDialog");
            string path = Path.Combine(pluginDir, "shimmy.exe");

            if (!File.Exists(path))
                path = Path.Combine(dllDir, "shimmy.exe");

            return File.Exists(path) ? path : null;
        }

        /// <summary>
        /// Resolves the data directory (where models/ lives) using the same
        /// logic as SidecarManager.
        /// </summary>
        private string ResolveDataDir()
        {
            string dataDir = _config.DataDir.Value;
            if (!string.IsNullOrEmpty(dataDir))
                return dataDir;

            string dllDir = Path.GetDirectoryName(
                System.Reflection.Assembly.GetExecutingAssembly().Location);
            string pluginDir = Path.Combine(dllDir, "ErenshorLLMDialog");
            string dir = Path.Combine(pluginDir, "data");

            if (Directory.Exists(dir))
                return dir;

            return Path.Combine(dllDir, "data");
        }

        /// <summary>
        /// Restarts shimmy: kills the current process and starts a new one.
        /// </summary>
        public void Restart()
        {
            _log.LogInfo("[ShimmyManager] Restart requested.");
            _shutdownRequested = false;

            if (_process != null && !_process.HasExited)
            {
                _log.LogInfo("[ShimmyManager] Killing current shimmy process...");
                try { _process.Kill(); } catch { }
            }

            _process = null;
            Start();
        }

        /// <summary>
        /// Starts the shimmy inference server.
        /// Shimmy auto-discovers GGUF models from ./models/ relative to its CWD.
        /// </summary>
        public void Start()
        {
            _shutdownRequested = false;

            if (IsRunning)
            {
                _log.LogInfo("[ShimmyManager] Shimmy is already running.");
                return;
            }

            string binaryPath = ResolveBinaryPath();
            if (binaryPath == null)
            {
                _log.LogError("[ShimmyManager] shimmy.exe not found. " +
                    "Place shimmy.exe in the ErenshorLLMDialog plugin folder. " +
                    "Local LLM inference will not be available.");
                return;
            }

            string dataDir = ResolveDataDir();
            int port = _config.ShimmyPort.Value;
            string gpuBackend = _config.ShimmyGpuBackend.Value;

            string args = "serve --bind 127.0.0.1:" + port;
            if (!string.IsNullOrEmpty(gpuBackend) && gpuBackend != "auto")
            {
                args += " --gpu-backend " + gpuBackend;
            }

            _log.LogInfo("[ShimmyManager] Starting shimmy: " + binaryPath);
            _log.LogInfo("[ShimmyManager] Args: " + args);
            _log.LogInfo("[ShimmyManager] CWD (models dir): " + dataDir);

            try
            {
                var psi = new ProcessStartInfo
                {
                    FileName = binaryPath,
                    Arguments = args,
                    WorkingDirectory = dataDir,
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    RedirectStandardError = true,
                    RedirectStandardOutput = false
                };

                _process = Process.Start(psi);

                if (_process == null || _process.HasExited)
                {
                    _log.LogError("[ShimmyManager] Failed to start shimmy process.");
                    return;
                }

                _process.ErrorDataReceived += OnStderrData;
                _process.BeginErrorReadLine();

                _log.LogInfo("[ShimmyManager] Shimmy started (PID: " + _process.Id + ")");
            }
            catch (Exception e)
            {
                _log.LogError("[ShimmyManager] Exception starting shimmy: " + e.Message);
            }
        }

        /// <summary>
        /// Blocks until shimmy's /v1/models endpoint responds or the timeout expires.
        /// Returns true if shimmy is ready.
        /// </summary>
        public bool WaitForReady(float timeoutSeconds = 30f)
        {
            if (!IsRunning)
                return false;

            int port = _config.ShimmyPort.Value;
            string url = "http://127.0.0.1:" + port + "/v1/models";

            float elapsed = 0f;
            float pollInterval = 0.5f;

            _log.LogInfo("[ShimmyManager] Waiting for shimmy to become ready on port " + port + "...");

            while (elapsed < timeoutSeconds)
            {
                if (_process == null || _process.HasExited)
                {
                    _log.LogError("[ShimmyManager] Shimmy process exited during startup " +
                        "(exit code: " + (_process?.ExitCode.ToString() ?? "unknown") + ")");
                    return false;
                }

                try
                {
                    var request = (HttpWebRequest)WebRequest.Create(url);
                    request.Method = "GET";
                    request.Timeout = 2000;

                    using (var response = (HttpWebResponse)request.GetResponse())
                    {
                        if (response.StatusCode == HttpStatusCode.OK)
                        {
                            _log.LogInfo("[ShimmyManager] Shimmy is ready. " +
                                "Startup took " + elapsed.ToString("F1") + "s");
                            return true;
                        }
                    }
                }
                catch
                {
                    // Not ready yet, keep polling
                }

                System.Threading.Thread.Sleep((int)(pollInterval * 1000));
                elapsed += pollInterval;
            }

            _log.LogError("[ShimmyManager] Shimmy failed to become ready within " +
                timeoutSeconds + " seconds.");
            return false;
        }

        /// <summary>
        /// Graceful shutdown: kills the shimmy process immediately.
        /// Called from OnApplicationQuit where Unity may terminate us at any moment,
        /// so we don't wait -- just kill.
        /// </summary>
        public void Stop()
        {
            if (_shutdownRequested)
                return;
            _shutdownRequested = true;

            if (_process == null || _process.HasExited)
            {
                _log.LogInfo("[ShimmyManager] Shimmy already stopped.");
                return;
            }

            _log.LogInfo("[ShimmyManager] Killing shimmy...");

            try
            {
                if (!_process.HasExited)
                {
                    _process.Kill();
                    _process.WaitForExit(1000);
                    _log.LogInfo("[ShimmyManager] Shimmy process killed.");
                }
            }
            catch (Exception e)
            {
                _log.LogWarning("[ShimmyManager] Error stopping shimmy: " + e.Message);
            }

            _process = null;
        }

        private void OnStderrData(object sender, DataReceivedEventArgs e)
        {
            if (!string.IsNullOrEmpty(e.Data))
            {
                _log.LogInfo("[Shimmy] " + e.Data);
            }
        }
    }
}
