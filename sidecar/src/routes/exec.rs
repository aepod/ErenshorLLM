//! POST /v1/exec endpoint.
//!
//! Spawns `busybox.exe <command> [args]` as a child process and returns
//! captured stdout/stderr. Used by ErenshorOS to provide unix commands
//! (ls, cat, grep, find, mkdir, etc.) natively on Windows.

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::Command;
use tracing::{info, warn};

/// Maximum output size in bytes before truncation (64 KB).
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Default timeout in seconds if not specified in the request.
const DEFAULT_TIMEOUT_SECS: u32 = 10;

/// Commands that are never allowed to execute (basic safety).
const BLOCKED_COMMANDS: &[&str] = &["format", "shutdown", "reboot", "poweroff"];

#[derive(Deserialize)]
struct ExecRequest {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    timeout_secs: Option<u32>,
}

#[derive(Serialize)]
struct ExecResponse {
    exit_code: i32,
    stdout: String,
    stderr: String,
    truncated: bool,
}

#[derive(Serialize)]
struct ExecError {
    error: String,
}

async fn handle_exec(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, (StatusCode, Json<ExecError>)> {
    let cmd_lower = req.command.to_lowercase();

    // Block dangerous commands
    if BLOCKED_COMMANDS.contains(&cmd_lower.as_str()) {
        warn!("Blocked exec of dangerous command: {}", req.command);
        return Err((
            StatusCode::FORBIDDEN,
            Json(ExecError {
                error: format!("Command '{}' is blocked for safety", req.command),
            }),
        ));
    }

    // Block `rm -rf /` specifically (check args for the pattern)
    if cmd_lower == "rm" {
        let args_joined = req.args.join(" ");
        if args_joined.contains("-rf /") || args_joined.contains("-rf \\") {
            warn!("Blocked dangerous rm invocation: rm {}", args_joined);
            return Err((
                StatusCode::FORBIDDEN,
                Json(ExecError {
                    error: "Dangerous rm invocation blocked".to_string(),
                }),
            ));
        }
    }

    // Resolve busybox.exe path relative to sidecar binary location
    let busybox_path = match std::env::current_exe() {
        Ok(exe) => exe
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("busybox.exe"),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecError {
                    error: format!("Cannot resolve exe path: {}", e),
                }),
            ));
        }
    };

    if !busybox_path.exists() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ExecError {
                error: format!(
                    "busybox.exe not found at {}. Place it alongside the sidecar binary.",
                    busybox_path.display()
                ),
            }),
        ));
    }

    let timeout = std::time::Duration::from_secs(
        req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS) as u64,
    );

    let working_dir = &state.config.data_dir;

    info!(
        command = %req.command,
        args = ?req.args,
        cwd = %working_dir.display(),
        "Executing busybox command"
    );

    // Spawn busybox with the command as first arg, then user args
    let mut child = match Command::new(&busybox_path)
        .arg(&req.command)
        .args(&req.args)
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecError {
                    error: format!("Failed to spawn busybox: {}", e),
                }),
            ));
        }
    };

    // Take stdout/stderr handles before waiting so we can read them after wait.
    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    // Wait with timeout. child.wait() borrows &mut self so we can kill on timeout.
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecError {
                    error: format!("Process error: {}", e),
                }),
            ));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err((
                StatusCode::REQUEST_TIMEOUT,
                Json(ExecError {
                    error: format!(
                        "Command timed out after {} seconds",
                        timeout.as_secs()
                    ),
                }),
            ));
        }
    };

    // Read captured output from the pipes
    use tokio::io::AsyncReadExt;

    let mut stdout_raw = Vec::new();
    if let Some(mut out) = child_stdout {
        let _ = out.read_to_end(&mut stdout_raw).await;
    }

    let mut stderr_raw = Vec::new();
    if let Some(mut err) = child_stderr {
        let _ = err.read_to_end(&mut stderr_raw).await;
    }

    let exit_code = status.code().unwrap_or(-1);

    // Truncate output if needed
    let mut truncated = false;

    let stdout = if stdout_raw.len() > MAX_OUTPUT_BYTES {
        truncated = true;
        String::from_utf8_lossy(&stdout_raw[..MAX_OUTPUT_BYTES]).into_owned()
    } else {
        String::from_utf8_lossy(&stdout_raw).into_owned()
    };

    let stderr = if stderr_raw.len() > MAX_OUTPUT_BYTES {
        truncated = true;
        String::from_utf8_lossy(&stderr_raw[..MAX_OUTPUT_BYTES]).into_owned()
    } else {
        String::from_utf8_lossy(&stderr_raw).into_owned()
    };

    Ok(Json(ExecResponse {
        exit_code,
        stdout,
        stderr,
        truncated,
    }))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/v1/exec", post(handle_exec))
}
