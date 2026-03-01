//! Shimmy process manager.
//!
//! Starts shimmy as a child process when LLM mode is Local or Hybrid.
//! Shimmy provides OpenAI-compatible inference for GGUF models.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tracing::{info, warn, error};

/// Manages the shimmy inference server child process.
pub struct ShimmyProcess {
    child: Child,
}

impl ShimmyProcess {
    /// Start shimmy as a child process.
    ///
    /// Looks for shimmy.exe (Windows) or shimmy (Unix) next to the sidecar binary.
    /// Falls back to PATH lookup.
    pub fn start(
        data_dir: &Path,
        bind_addr: &str,
        gpu_backend: &str,
        model_dir: &str,
    ) -> Option<Self> {
        let shimmy_name = if cfg!(windows) { "shimmy.exe" } else { "shimmy" };

        // Look for shimmy next to our own binary
        let shimmy_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(shimmy_name)))
            .filter(|p| p.exists())
            .unwrap_or_else(|| shimmy_name.into());

        let model_dir_resolved = data_dir.join(model_dir);

        let mut cmd = Command::new(&shimmy_path);
        cmd.arg("serve")
            .arg("--bind").arg(bind_addr)
            .arg("--model-dirs").arg(&model_dir_resolved)
            .arg("--gpu-backend").arg(gpu_backend)
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        info!(
            "Starting shimmy: {} serve --bind {} --model-dirs {} --gpu-backend {}",
            shimmy_path.display(), bind_addr, model_dir_resolved.display(), gpu_backend
        );

        match cmd.spawn() {
            Ok(child) => {
                info!("Shimmy started (PID: {})", child.id());
                Some(Self { child })
            }
            Err(e) => {
                error!(
                    "Failed to start shimmy at {:?}: {}. Local LLM will not be available.",
                    shimmy_path, e
                );
                None
            }
        }
    }

    /// Wait for shimmy to become ready by polling /v1/models.
    pub async fn wait_ready(endpoint: &str, timeout: Duration) -> bool {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();

        let url = format!("{}/v1/models", endpoint);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!("Shimmy ready ({:.1}s)", start.elapsed().as_secs_f32());
                    return true;
                }
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        warn!("Shimmy did not become ready within {}s", timeout.as_secs());
        false
    }

    /// Check if the shimmy process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ShimmyProcess {
    fn drop(&mut self) {
        info!("Stopping shimmy...");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
