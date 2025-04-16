use anyhow::Result;
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Write, path::Path, process::Command};
use tempfile::NamedTempFile;
use tracing::{debug, error, info, warn};
use tracing_subscriber;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct RunServiceRequest {
    code: String,
    deployment: String,
    mirrord_config: String,
}

#[derive(Serialize, Deserialize)]
struct ToolParameters {
    #[serde(rename = "type")]
    param_type: String,
    properties: serde_json::Map<String, Value>,
    required: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct ToolFunction {
    name: String,
    description: String,
    parameters: ToolParameters,
}

#[derive(Serialize, Deserialize)]
struct ToolDefinition {
    #[serde(rename = "type")]
    tool_type: String,
    function: ToolFunction,
}

async fn tools() -> Result<Json<Vec<ToolDefinition>>, StatusCode> {
    let tool_json = include_str!("../tools/run_service.json");
    let tool_function: ToolFunction =
        serde_json::from_str(tool_json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tools = vec![ToolDefinition {
        tool_type: "function".to_string(),
        function: tool_function,
    }];
    info!("Serving /tools endpoint with run_service tool");
    Ok(Json(tools))
}

fn get_pod_name(deployment: &str, namespace: &str) -> Result<String, StatusCode> {
    let output = Command::new("kubectl")
        .arg("get")
        .arg("pods")
        .arg("-n")
        .arg(namespace)
        .arg("-l")
        .arg(format!("app={}", deployment))
        .arg("-o")
        .arg("jsonpath={.items[0].metadata.name}")
        .output()
        .map_err(|e| {
            error!("Failed to run kubectl: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if output.status.success() {
        let pod_name = String::from_utf8(output.stdout).map_err(|e| {
            error!("Invalid pod name: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        if pod_name.is_empty() {
            error!("No pod found for deployment: {}", deployment);
            Err(StatusCode::NOT_FOUND)
        } else {
            info!("Found pod: {}", pod_name);
            Ok(pod_name)
        }
    } else {
        let stderr = String::from_utf8(output.stderr).map_err(|e| {
            error!("Invalid kubectl error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        error!("kubectl failed: {}", stderr);
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

async fn run_service(Json(req): Json<RunServiceRequest>) -> Result<String, StatusCode> {
    // Fetch the pod name for the deployment
    let pod_name = get_pod_name(&req.deployment, "default").map_err(|e| {
        error!("Failed to get pod name: {}", e);
        StatusCode::NOT_FOUND
    })?;

    // Update mirrord config with the pod name
    let config: serde_json::Value = serde_json::from_str(&req.mirrord_config).map_err(|e| {
        error!("Failed to parse mirrord config: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let updated_config = serde_json::json!({
        "target": {
            "namespace": "default",
            "path": format!("pod/{}", pod_name)
        },
        "feature": config["feature"]
    });
    let config_str = serde_json::to_string(&updated_config).map_err(|e| {
        error!("Failed to serialize mirrord config: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    // Create temporary project directory
    let project_dir = format!("/tmp/mirrord_agent_code_{}", Uuid::new_v4());
    debug!("Creating project directory: {}", project_dir);
    std::fs::create_dir_all(format!("{}/src", &project_dir)).map_err(|e| {
        error!("Failed to create project directory: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Write Cargo.toml
    let cargo_toml = r#"
[package]
name = "agent-code"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = { version = "0.12", features = ["json", "blocking"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
"#;
    std::fs::write(format!("{}/Cargo.toml", &project_dir), cargo_toml).map_err(|e| {
        error!("Failed to write Cargo.toml: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    debug!("Wrote Cargo.toml to {}", project_dir);

    // Write main.rs
    std::fs::write(format!("{}/src/main.rs", &project_dir), &req.code).map_err(|e| {
        error!("Failed to write main.rs: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    debug!("Wrote main.rs with code length: {} bytes", req.code.len());

    // Compile
    info!("Compiling Rust code in {}", project_dir);
    let compile_output = Command::new("cargo")
        .current_dir(&project_dir)
        .args(["build", "--release"])
        .output()
        .map_err(|e| {
            error!("Failed to execute cargo build: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if !compile_output.status.success() {
        let err = String::from_utf8_lossy(&compile_output.stderr);
        error!("Build failed: {}", err);
        return Err(StatusCode::BAD_REQUEST);
    }
    info!("Compilation succeeded");

    let binary_path = format!("{}/target/release/agent-code", &project_dir);
    if !Path::new(&binary_path).exists() {
        error!("Binary not found at: {}", binary_path);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    debug!("Binary created at {}", binary_path);

    // Write mirrord config to temp file
    let mut config_file = NamedTempFile::with_suffix(".json").map_err(|e| {
        error!("Failed to create temp file: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    config_file.write_all(config_str.as_bytes()).map_err(|e| {
        error!("Failed to write mirrord config: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let config_path = config_file
        .path()
        .to_str()
        .ok_or_else(|| {
            error!("Failed to convert path to string");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .to_string();
    debug!("Wrote mirrord config to {}", config_path);

    // Run mirrord
    info!("Executing mirrord for pod: {}", pod_name);
    let output = Command::new("mirrord")
        .arg("exec")
        .arg("--config-file")
        .arg(&config_path)
        .arg(&binary_path)
        .output()
        .map_err(|e| {
            error!("Failed to execute mirrord: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Clean up
    let _ = std::fs::remove_dir_all(&project_dir);
    let _ = config_file.close();
    debug!("Cleaned up project directory and config file");

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        info!("Mirrord execution succeeded");
        debug!("stdout: '{}', stderr: '{}'", stdout, stderr);
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        warn!("Mirrord execution failed: {}", stderr);
        warn!("Mirrord config used: {}", config_str);
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    info!("Starting MCP server on port 3000");
    let app = Router::new()
        .route("/tools", get(tools))
        .route("/run-service", post(run_service));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
