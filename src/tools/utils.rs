use anyhow::Result;
use rmcp::Error as McpError;
use std::process::{Command, Output};
use std::time::Duration;
use tokio::task;
use tokio::time::timeout;

const KUBECTL_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn get_pod_name(deployment: &str, namespace: &str) -> Result<String, McpError> {
    let deployment_name = deployment.to_string();
    let namespace = namespace.to_string();

    let blocking_task = task::spawn_blocking(move || {
        Command::new("kubectl")
            .arg("get")
            .arg("pods")
            .arg("-n")
            .arg(namespace)
            .arg("-l")
            .arg(format!("app={}", deployment_name))
            .arg("-o")
            .arg("jsonpath={.items[0].metadata.name}")
            .output()
    });

    match timeout(KUBECTL_TIMEOUT, blocking_task).await {
        Ok(Ok(Ok(output))) => {
            // Timeout succeeded, spawn_blocking succeeded, Command::output succeeded
            handle_kubectl_output(output, deployment) // Pass deployment for error message
        }
        Ok(Ok(Err(e))) => {
            // Timeout succeeded, spawn_blocking succeeded, Command::output failed (e.g., command not found)
            tracing::error!(error = %e, "Failed to run kubectl command");
            if e.kind() == std::io::ErrorKind::NotFound {
                Err(McpError::internal_error(
                    "Failed to execute kubectl: 'kubectl' command not found in PATH.".to_string(),
                    None,
                ))
            } else {
                Err(McpError::internal_error(
                    format!("Failed to start kubectl process: {}", e),
                    None,
                ))
            }
        }
        Ok(Err(e)) => {
            // Timeout succeeded, but spawn_blocking failed (rare, might indicate panic)
            tracing::error!(error = %e, "kubectl blocking task failed");
            Err(McpError::internal_error(
                format!("kubectl task failed: {}", e),
                None,
            ))
        }
        Err(_) => {
            // Timeout elapsed
            tracing::error!("kubectl command timed out after {:?}", KUBECTL_TIMEOUT);
            Err(McpError::internal_error(
                format!("kubectl command timed out after {:?}", KUBECTL_TIMEOUT),
                None,
            ))
        }
    }
}

fn handle_kubectl_output(output: Output, deployment: &str) -> Result<String, McpError> {
    if output.status.success() {
        let pod_name = String::from_utf8(output.stdout).map_err(|e| {
            tracing::error!(error = %e, "Invalid pod name");
            McpError::internal_error(
                "Failed to parse pod name from kubectl output".to_string(),
                None,
            )
        })?;
        if pod_name.is_empty() {
            tracing::error!("No pod found for deployment");
            Err(McpError::internal_error(
                format!("No pod found for deployment: {}", deployment),
                None,
            ))
        } else {
            tracing::info!("Found pod: {}", pod_name);
            Ok(pod_name)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string(); // Use lossy for robustness
        tracing::error!(error = "kubectl failed", stderr = %stderr);
        Err(McpError::internal_error(
            format!("kubectl command failed: {}", stderr),
            None,
        ))
    }
}

pub async fn update_mirrord_config(
    mirrord_config: &str,
    deployment: &str,
    namespace: &str,
) -> Result<String, McpError> {
    let pod_name = get_pod_name(deployment, namespace).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to get pod name");
        e
    })?;

    let mut config_value: serde_json::Value =
        serde_json::from_str(mirrord_config).map_err(|e| {
            tracing::error!(error = %e, "Failed to parse mirrord config");
            McpError::internal_error("Failed to parse mirrord config".to_string(), None)
        })?;

    // Ensure the top level is an object
    let config_obj = config_value.as_object_mut().ok_or_else(|| {
        tracing::error!("Mirrord config is not a JSON object");
        McpError::internal_error("Mirrord config must be a JSON object".to_string(), None)
    })?;

    // Create or update the "target" field
    let target_value = serde_json::json!({
        "namespace": namespace,
        "path": format!("pod/{}", pod_name)
    });
    config_obj.insert("target".to_string(), target_value);

    // Serialize the modified config
    serde_json::to_string(&config_value).map_err(|e| {
        tracing::error!(error = %e, "Failed to serialize updated mirrord config");
        McpError::internal_error("Failed to serialize mirrord config".to_string(), None)
    })
}
