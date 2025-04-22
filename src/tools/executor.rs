use anyhow::Result;
use rmcp::Error as McpError;
use std::io::Write;
use std::process::Command;
use std::time::Duration;
use tempfile::{NamedTempFile, TempPath}; // Use TempPath for config file persistence
use tokio::task;
use tokio::time::timeout;
use crate::tools::utils::update_mirrord_config;

const MIRRORD_EXEC_TIMEOUT: Duration = Duration::from_secs(120); // 2 minutes

/// Executes a language-specific script/binary using mirrord.
///
/// Handles common tasks like temp directory creation, config generation,
/// mirrord execution, and output processing. Language-specific logic
/// is provided via a type implementing `MirrordRunnable`.
///
/// # Arguments
/// * `cmd_str` - The command-line statement to run.
/// * `deployment` - The target Kubernetes deployment name.
/// * `mirrord_config` - The base mirrord configuration (JSON string).
/// * `namespace` - The target Kubernetes namespace (currently hardcoded, consider making configurable).
///
/// # Returns
/// The stdout of the successful execution, or an McpError.
pub async fn execute_mirrord_run(
    cmd_str: &str,
    deployment: &str,
    mirrord_config: &str,
    namespace: &str,
) -> Result<String, McpError> {
    let args = shell_words::split(cmd_str).map_err(|e| {
        tracing::error!(error = %e, "Failed to parse command line arguments");
        McpError::internal_error("Failed to parse command line arguments".to_string(), None)
    })?;
    // --- 1. Update and Write Mirrord Config ---
    let config_str = update_mirrord_config(mirrord_config, deployment, namespace)
        .await
        .inspect_err(|e| {
            tracing::error!(error = ?e, "Failed to update mirrord config");
        })?;

    let mut config_file = NamedTempFile::with_suffix(".json").map_err(|e| {
        tracing::error!(error = %e, "Failed to create temp config file");
        McpError::internal_error("Failed to create temp config file".to_string(), None)
    })?;

    config_file.write_all(config_str.as_bytes()).map_err(|e| {
        tracing::error!(error = %e, path = %config_file.path().display(), "Failed to write mirrord config");
        McpError::internal_error("Failed to write mirrord config".to_string(), None)
    })?;
    // Keep the TempPath to ensure the file isn't deleted before the command runs
    let config_path: TempPath = config_file.into_temp_path();
    tracing::debug!("Wrote mirrord config to {}", config_path.display());

    // --- 5. Execute Mirrord ---
    let config_path_owned = config_path.to_path_buf(); // Clone PathBuf to move into task
    let blocking_task = task::spawn_blocking(move || {
        let mut command = Command::new("mirrord");
        command
            .arg("exec")
            .arg("--config-file")
            .arg(&config_path_owned); // Use owned path
        for arg in args {
            command.arg(arg);
        }
        tracing::info!(command = ?command, "Executing mirrord command in blocking task...");
        command.output() // Execute the command
    });

    let output = match timeout(MIRRORD_EXEC_TIMEOUT, blocking_task).await {
        Ok(Ok(Ok(output))) => Ok(output), // All succeeded
        Ok(Ok(Err(e))) => {
            // Command::output failed
            tracing::error!(error = %e, "Failed to run mirrord command");
            if e.kind() == std::io::ErrorKind::NotFound {
                Err(McpError::internal_error(
                    "Failed to execute mirrord: 'mirrord' command not found in PATH.".to_string(),
                    None,
                ))
            } else {
                Err(McpError::internal_error(
                    format!("Failed to start mirrord process: {}", e),
                    None,
                ))
            }
        }
        Ok(Err(e)) => {
            // spawn_blocking failed
            tracing::error!(error = %e, "mirrord blocking task failed");
            Err(McpError::internal_error(
                format!("mirrord task failed: {}", e),
                None,
            ))
        }
        Err(_) => {
            // Timeout elapsed
            tracing::error!(
                "Mirrord execution timed out after {:?}",
                MIRRORD_EXEC_TIMEOUT
            );
            Err(McpError::internal_error(
                format!(
                    "Mirrord execution timed out after {:?}",
                    MIRRORD_EXEC_TIMEOUT
                ),
                None,
            ))
        }
    }?;

    // --- 6. Handle Output ---
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        tracing::info!("Mirrord execution succeeded");
        tracing::debug!(
            "stdout num bytes: {}, stderr num bytes: {}",
            stdout.len(),
            stderr.len()
        );
        tracing::trace!("stdout: '{}', stderr: '{}'", stdout, stderr);
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code_info = output
            .status
            .code()
            .map_or_else(|| "None".to_string(), |c| c.to_string());
        tracing::error!(exit_code = %exit_code_info, error = stderr, "Mirrord execution failed");
        tracing::debug!("Mirrord config used: {}", config_str);
        Err(McpError::internal_error(
            format!(
                "Mirrord execution failed (Exit Code: {}): {}",
                exit_code_info, stderr
            ),
            None,
        ))
    }
    // --- 7. Cleanup --- (Automatic via Drop)
}
