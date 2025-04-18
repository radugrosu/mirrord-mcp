use super::{runnable::MirrordRunnable, utils::update_mirrord_config};
use anyhow::Result;
use rmcp::Error as McpError;
use std::io::Write;
use std::process::Command;
use tempfile::{NamedTempFile, TempPath}; // Use TempPath for config file persistence

/// Executes a language-specific script/binary using mirrord.
///
/// Handles common tasks like temp directory creation, config generation,
/// mirrord execution, and output processing. Language-specific logic
/// is provided via a type implementing `MirrordRunnable`.
///
/// # Arguments
/// * `deployment` - The target Kubernetes deployment name.
/// * `mirrord_config` - The base mirrord configuration (JSON string).
/// * `namespace` - The target Kubernetes namespace (currently hardcoded, consider making configurable).
/// * `setup_project` - A closure that takes the temporary project directory `Path`
///   and performs language-specific setup (e.g., writing source files, installing deps).
/// * `get_command_args` - A closure that takes the temporary project directory `Path`
///   and returns the executable and arguments to be passed to `mirrord exec`.
///
/// # Returns
/// The stdout of the successful execution, or an McpError.
pub fn execute_mirrord_run<R: MirrordRunnable>(
    runner: &R,
    deployment: &str,
    mirrord_config: &str,
    namespace: &str,
) -> Result<String, McpError> {
    // --- 1. Create Temp Directory ---
    let temp_dir = tempfile::tempdir().map_err(|e| {
        tracing::error!(error = %e, "Failed to create temporary directory");
        McpError::internal_error("Failed to create temporary directory".to_string(), None)
    })?;
    let project_dir = temp_dir.path();
    tracing::debug!("Created project directory: {}", project_dir.display());

    // --- 2. Run Language-Specific Setup ---
    runner.setup_project(project_dir).inspect_err(|_| {
        tracing::error!("Project setup failed in {}", project_dir.display());
    })?;
    tracing::debug!("Project setup completed for {}", project_dir.display());

    // --- 3. Update and Write Mirrord Config ---
    let config_str =
        update_mirrord_config(mirrord_config, deployment, namespace).inspect_err(|e| {
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

    // --- 4. Get Language-Specific Command Args ---
    let command_args = runner.get_command_args(project_dir).inspect_err(|_| {
        tracing::error!("Failed to determine command arguments");
    })?;
    tracing::debug!("Command args to execute: {:?}", command_args);

    // --- 5. Execute Mirrord ---
    let mut command = Command::new("mirrord");
    command.arg("exec").arg("--config-file").arg(&config_path);
    // language-specific executable and args
    for arg in command_args {
        command.arg(arg);
    }

    tracing::info!(command = ?command, "Executing mirrord command...");
    let output = command.output().map_err(|e| {
        tracing::error!(error = %e, command = ?command, "Failed to execute mirrord command");
        // Check if the error is 'NotFound'
        if e.kind() == std::io::ErrorKind::NotFound {
            McpError::internal_error(
                "Failed to execute mirrord: 'mirrord' command not found in PATH.".to_string(),
                None,
            )
        } else {
            McpError::internal_error(format!("Failed to start mirrord process: {}", e), None)
        }
    })?;

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
