use super::utils::get_pod_name;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::{io::Write, path::Path, process::Command};
use tempfile::NamedTempFile;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Request {
    #[schemars(
        description = "Complete rust code using only reqwest::blocking::get, serde::Deserialize, serde_json, and anyhow::Result. The resulting binary is run against the cluster."
    )]
    code: String,
    #[schemars(description = "Kubernetes deployment name.")]
    deployment: String,
    #[schemars(
        description = "Mirrord config in JSON format.e.g., '{\"feature\": {\"network\": {\"incoming\": {\"mode\": \"mirror\", \"ports\": [ 8888 ] } } }'."
    )]
    mirrord_config: String,
}

pub fn run(request: Request) -> Result<String, McpError> {
    // Fetch the pod name for the deployment
    let pod_name = get_pod_name(&request.deployment, "default").map_err(|e| {
        tracing::error!(error = %e, "Failed to get pod name");
        e
    })?;

    // Update mirrord config with the pod name
    let config: serde_json::Value = serde_json::from_str(&request.mirrord_config).map_err(|e| {
        tracing::error!(error = %e, "Failed to parse mirrord config");
        McpError::internal_error("Failed to parse mirrord config".to_string(), None)
    })?;

    let updated_config = serde_json::json!({
        "target": {
            "namespace": "default",
            "path": format!("pod/{}", pod_name)
        },
        "feature": config["feature"]
    });
    let config_str = serde_json::to_string(&updated_config).map_err(|e| {
        tracing::error!(error = %e, "Failed to serialize mirrord config");
        McpError::internal_error("Failed to serialize mirrord config".to_string(), None)
    })?;

    // Create temporary project directory
    let project_dir = format!("/tmp/mirrord_agent_code_{}", Uuid::new_v4());
    tracing::debug!("Creating project directory: {}", project_dir);
    std::fs::create_dir_all(format!("{}/src", &project_dir)).map_err(|e| {
        tracing::error!(error=%e, "Failed to create project directory");
        McpError::internal_error("Failed to create project directory".to_string(), None)
    })?;

    let compile_mode = std::env::var("MCP_SERVICE_COMPILE_MODE").unwrap_or("release".to_string());
    tracing::debug!("Compile mode: {}", compile_mode);

    // Write Cargo.toml
    let mut cargo_toml = r#"
[package]
name = "agent-code"
version = "0.1.0"
edition = "2024"

[dependencies]
reqwest = { version = "0.12", features = ["json", "blocking"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
"#
    .to_string();

    if compile_mode == "debug" {
        cargo_toml.push_str(
            r#"
[profile.dev]
opt-level = 1
debug = false
split-debuginfo = "unpacked"
debug-assertions = false
overflow-checks = false
lto = false
panic = "abort"
incremental = true
codegen-units = 16
"#,
        );
    }

    std::fs::write(format!("{}/Cargo.toml", &project_dir), cargo_toml).map_err(|e| {
        tracing::error!(error = %e, "Failed to write Cargo.toml");
        McpError::internal_error("Failed to write Cargo.toml".to_string(), None)
    })?;
    tracing::debug!("Wrote Cargo.toml to {}", project_dir);

    // Write main.rs
    std::fs::write(format!("{}/src/main.rs", &project_dir), &request.code).map_err(|e| {
        tracing::error!(error = %e, "Failed to write main.rs");
        McpError::internal_error("Failed to write main.rs".to_string(), None)
    })?;
    tracing::debug!(
        "Wrote main.rs with code length: {} bytes",
        request.code.len()
    );

    // Compile
    tracing::info!("Compiling Rust code in {}", project_dir);
    let compile_args = match compile_mode.as_str() {
        "debug" => &["build"][..],
        _ => &["build", "--release"][..],
    };
    let compile_output = Command::new("cargo")
        .current_dir(&project_dir)
        .args(compile_args)
        .output()
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to execute cargo build");
            McpError::internal_error("Failed to execute cargo build".to_string(), None)
        })?;

    if !compile_output.status.success() {
        let err = String::from_utf8_lossy(&compile_output.stderr);
        tracing::error!(error = %err, "Build failed");
        return Err(McpError::internal_error(
            format!("Build failed: {}", err),
            None,
        ));
    }
    tracing::info!("Compilation succeeded");

    let binary_path = format!("{}/target/{}/agent-code", &project_dir, &compile_mode);
    if !Path::new(&binary_path).exists() {
        tracing::error!("Binary not found at: {}", binary_path);
        return Err(McpError::internal_error(
            format!("Binary not found at: {}", binary_path),
            None,
        ));
    }
    tracing::debug!("Binary created at {}", binary_path);

    // Write mirrord config to temp file
    let mut config_file = NamedTempFile::with_suffix(".json").map_err(|e| {
        tracing::error!(error = %e, "Failed to create temp file");
        McpError::internal_error("Failed to create temp file".to_string(), None)
    })?;
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
    tracing::info!("Executing mirrord for pod: {}", pod_name);
    let output = Command::new("mirrord")
        .arg("exec")
        .arg("--config-file")
        .arg(&config_path)
        .arg(&binary_path)
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
