use super::utils::update_mirrord_config;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::path::Path;
use std::{io::Write, process::Command};
use tempfile::NamedTempFile;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Request {
    #[schemars(
        description = "Complete Python code using only requests for HTTP requests and json for deserialization. The resulting script is run against the cluster."
    )]
    code: String,
    #[schemars(description = "Kubernetes deployment name.")]
    deployment: String,
    #[schemars(
        description = "Mirrord config in JSON format, e.g., '{\"feature\": {\"network\": {\"incoming\": {\"mode\": \"mirror\", \"ports\": [8888]}}}}'."
    )]
    mirrord_config: String,
}

pub fn run(request: Request) -> Result<String, McpError> {
    // Create temporary project directory
    let temp_dir = tempfile::tempdir().map_err(|e| {
        tracing::error!(error = %e, "Failed to create temporary directory");
        McpError::internal_error("Failed to create temporary directory".to_string(), None)
    })?;
    let project_dir = temp_dir.path();
    tracing::debug!("Created project directory: {}", project_dir.display());

    let config_str =
        update_mirrord_config(&request.mirrord_config, &request.deployment, "default")?;
    // Write main.py
    std::fs::write(Path::new(project_dir).join("main.py"), &request.code).map_err(|e| {
        tracing::error!(error = %e, "Failed to write main.py");
        McpError::internal_error("Failed to write main.py".to_string(), None)
    })?;
    tracing::debug!(
        "Wrote main.py with code length: {} bytes",
        request.code.len()
    );

    // Write mirrord config to temp file
    let mut config_file = NamedTempFile::with_suffix(".json").map_err(|e| {
        tracing::error!(error = %e, "Failed to create temp file");
        McpError::internal_error("Failed to create temp file".to_string(), None)
    })?;
    config_file.write_all(config_str.as_bytes()).map_err(|e| {
        tracing::error!(error = %e, "Failed to write mirrord config");
        McpError::internal_error("Failed to write mirrord config".to_string(), None)
    })?;
    let config_path = config_file.path();
    tracing::debug!("Wrote mirrord config to {}", config_path.display());

    // Run mirrord
    let output = Command::new("mirrord")
        .arg("exec")
        .arg("--config-file")
        .arg(config_path)
        .arg("python3")
        .arg(Path::new(project_dir).join("main.py"))
        .output()
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to execute mirrord");
            McpError::internal_error("Failed to execute mirrord".to_string(), None)
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        tracing::info!("Mirrord execution succeeded");
        tracing::debug!("stdout: '{}', stderr: '{}'", stdout, stderr);
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        tracing::error!(error = stderr, "Mirrord execution failed");
        tracing::debug!("Mirrord config used: {}", config_str);
        Err(McpError::internal_error(
            format!("Mirrord execution failed: {}", stderr),
            None,
        ))
    }
}
