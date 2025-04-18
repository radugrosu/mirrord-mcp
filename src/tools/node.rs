use super::utils::update_mirrord_config;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::{io::Write, process::Command};
use tempfile::NamedTempFile;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Request {
    #[schemars(
        description = "Complete JavaScript code using only axios for HTTP requests and JSON.parse for deserialization. The resulting script is run against the cluster."
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
    let project_dir = format!("/tmp/mirrord_node_code_{}", Uuid::new_v4());
    tracing::debug!("Creating project directory: {}", project_dir);
    std::fs::create_dir_all(&project_dir).map_err(|e| {
        tracing::error!(error=%e, "Failed to create project directory");
        McpError::internal_error("Failed to create project directory".to_string(), None)
    })?;

    compile_node(&request.code, &project_dir)?;

    // Write mirrord config to temp file
    let mut config_file = NamedTempFile::with_suffix(".json").map_err(|e| {
        tracing::error!(error = %e, "Failed to create temp file");
        McpError::internal_error("Failed to create temp file".to_string(), None)
    })?;

    let config_str =
        update_mirrord_config(&request.mirrord_config, &request.deployment, "default")?;
    config_file.write_all(config_str.as_bytes()).map_err(|e| {
        tracing::error!(error = %e, "Failed to write mirrord config");
        McpError::internal_error("Failed to write mirrord config".to_string(), None)
    })?;
    let config_path = config_file
        .path()
        .to_str()
        .ok_or_else(|| {
            tracing::error!("Failed to convert path to string");
            McpError::internal_error("Failed to convert path to string".to_string(), None)
        })?
        .to_string();
    tracing::debug!("Wrote mirrord config to {}", config_path);

    // Run mirrord
    let output = Command::new("mirrord")
        .arg("exec")
        .arg("--config-file")
        .arg(&config_path)
        .arg("node")
        .arg(format!("{}/index.js", project_dir))
        .output()
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to execute mirrord");
            McpError::internal_error("Failed to execute mirrord".to_string(), None)
        })?;

    // Clean up
    let _ = std::fs::remove_dir_all(&project_dir);
    let _ = config_file.close();
    tracing::debug!("Cleaned up project directory and config file");

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

pub fn compile_node(code: &str, project_dir: &str) -> Result<(), McpError> {
    // Write package.json
    let package_json = r#"
{
  "name": "mirrord-node-code",
  "version": "0.1.0",
  "dependencies": {
    "axios": "^1.7.0"
  }
}
"#;
    std::fs::write(format!("{}/package.json", &project_dir), package_json).map_err(|e| {
        tracing::error!(error = %e, "Failed to write package.json");
        McpError::internal_error("Failed to write package.json".to_string(), None)
    })?;
    tracing::debug!("Wrote package.json to {}", project_dir);

    // Write index.js
    std::fs::write(format!("{}/index.js", project_dir), code).map_err(|e| {
        tracing::error!(error = %e, "Failed to write index.js");
        McpError::internal_error("Failed to write index.js".to_string(), None)
    })?;
    tracing::debug!("Wrote index.js with code length: {} bytes", code.len());

    // Install dependencies
    tracing::info!("Installing Node.js dependencies in {}", project_dir);
    let npm_install_output = Command::new("npm")
        .current_dir(project_dir)
        .arg("install")
        .output()
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to execute npm install");
            McpError::internal_error("Failed to execute npm install".to_string(), None)
        })?;

    if !npm_install_output.status.success() {
        let err = String::from_utf8_lossy(&npm_install_output.stderr);
        tracing::error!(error = %err, "npm install failed");
        return Err(McpError::internal_error(
            format!("npm install failed: {}", err),
            None,
        ));
    }
    tracing::info!("npm install succeeded");
    Ok(())
}
