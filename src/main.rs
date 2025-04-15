use anyhow::Result;
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Command;

#[derive(Serialize, Deserialize)]
struct RunServiceRequest {
    command: String,
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
    Ok(Json(tools))
}

fn get_pod_name(deployment: &str, namespace: &str) -> Result<String, String> {
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
        .map_err(|e| format!("Failed to run kubectl: {}", e))?;

    if output.status.success() {
        let pod_name =
            String::from_utf8(output.stdout).map_err(|e| format!("Invalid pod name: {}", e))?;
        if pod_name.is_empty() {
            Err("No pod found for deployment".to_string())
        } else {
            Ok(pod_name)
        }
    } else {
        let stderr = String::from_utf8(output.stderr)
            .map_err(|e| format!("Invalid kubectl error: {}", e))?;
        Err(format!("kubectl failed: {}", stderr))
    }
}

async fn run_service(Json(req): Json<RunServiceRequest>) -> Result<String, String> {
    // Fetch the pod name for the deployment
    let pod_name = get_pod_name(&req.deployment, "default")
        .map_err(|e| format!("Failed to get pod name: {}", e))?;

    // Update mirrord config with the pod name
    let config: serde_json::Value = serde_json::from_str(&req.mirrord_config)
        .map_err(|e| format!("Invalid mirrord config: {}", e))?;
    let updated_config = serde_json::json!({
        "target": {
            "namespace": "default",
            "path": format!("pod/{}", pod_name)
        },
        "feature": config["feature"]
    });
    let config_str = serde_json::to_string(&updated_config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    // Write config to temp file
    let config_path = "/tmp/mirrord-config.json";
    std::fs::write(config_path, &config_str)
        .map_err(|e| format!("Failed to write config: {}", e))?;

    // Split the command into binary and arguments
    let parts = req.command.splitn(2, ' ').collect::<Vec<&str>>();
    if parts.is_empty() {
        return Err("Empty command provided".to_string());
    }
    let binary = parts[0];
    let args = parts.get(1).unwrap_or(&"");

    // Run mirrord
    let output = Command::new("mirrord")
        .arg("exec")
        .arg("--config-file")
        .arg(config_path)
        .arg("--")
        .arg(binary)
        .arg(args)
        .output()
        .map_err(|e| format!("Failed to run mirrord: {}", e))?;

    if output.status.success() {
        let stdout =
            String::from_utf8(output.stdout).map_err(|e| format!("Invalid output: {}", e))?;
        Ok(stdout)
    } else {
        let stderr =
            String::from_utf8(output.stderr).map_err(|e| format!("Invalid error output: {}", e))?;
        Err(format!("Mirrord failed: {}", stderr))
    }
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/tools", get(tools))
        .route("/run-service", post(run_service));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
