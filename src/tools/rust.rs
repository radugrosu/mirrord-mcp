use super::utils::update_mirrord_config;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::path::PathBuf;
use std::{io::Write, path::Path, process::Command};
use tempfile::NamedTempFile;

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
    // Create temporary project directory
    let temp_dir = tempfile::tempdir().map_err(|e| {
        tracing::error!(error = %e, "Failed to create temporary directory");
        McpError::internal_error("Failed to create temporary directory".to_string(), None)
    })?;
    let project_dir = temp_dir.path();
    tracing::debug!("Created project directory: {}", project_dir.display());

    let config_str =
        update_mirrord_config(&request.mirrord_config, &request.deployment, "default")?;

    let binary_path = compile_rust(&request.code, project_dir)?;

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

fn compile_rust(code: &str, project_dir: &Path) -> Result<PathBuf, McpError> {
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

    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).map_err(|e| {
        tracing::error!(error = %e, "Failed to write Cargo.toml");
        McpError::internal_error("Failed to write Cargo.toml".to_string(), None)
    })?;
    tracing::debug!("Wrote Cargo.toml to {}", project_dir.display());

    // Write main.rs
    std::fs::write(project_dir.join("src").join("main.rs"), code).map_err(|e| {
        tracing::error!(error = %e, "Failed to write main.rs");
        McpError::internal_error("Failed to write main.rs".to_string(), None)
    })?;
    tracing::debug!("Wrote main.rs with code length: {} bytes", code.len());

    // Compile
    tracing::info!("Compiling Rust code in {}", project_dir.display());
    let compile_args = match compile_mode.as_str() {
        "debug" => &["build"][..],
        _ => &["build", "--release"][..],
    };
    let compile_output = Command::new("cargo")
        .current_dir(project_dir)
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

    let binary_path = project_dir
        .join("target")
        .join(compile_mode)
        .join("agent-code");
    if !&binary_path.exists() {
        tracing::error!("Binary not found at: {}", binary_path.display());
        return Err(McpError::internal_error(
            format!("Binary not found at: {}", binary_path.display()),
            None,
        ));
    }
    tracing::debug!("Binary created at {}", binary_path.display());
    Ok(binary_path)
}
