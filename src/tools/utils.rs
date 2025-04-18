use anyhow::Result;
use rmcp::Error as McpError;
use std::process::Command;

pub fn get_pod_name(deployment: &str, namespace: &str) -> Result<String, McpError> {
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
            tracing::error!(error = %e, "Failed to run kubectl");
            McpError::internal_error("Failed to execute kubectl command".to_string(), None)
        })?;

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
        let stderr = String::from_utf8(output.stderr).map_err(|e| {
            tracing::error!(error = %e, "Failed to parse kubectl error");
            McpError::internal_error("Failed to parse kubectl error output".to_string(), None)
        })?;
        tracing::error!(error = "kubectl failed", stderr);
        Err(McpError::internal_error(
            format!("kubectl failed {}", stderr),
            None,
        ))
    }
}

pub fn update_mirrord_config(
    mirrord_config: &str,
    deployment: &str,
    namespace: &str,
) -> Result<String, McpError> {
    // Fetch the pod name for the deployment
    let pod_name = get_pod_name(deployment, namespace).map_err(|e| {
        tracing::error!(error = %e, "Failed to get pod name");
        e
    })?;

    // Update mirrord config with the pod name
    let config: serde_json::Value = serde_json::from_str(mirrord_config).map_err(|e| {
        tracing::error!(error = %e, "Failed to parse mirrord config");
        McpError::internal_error("Failed to parse mirrord config".to_string(), None)
    })?;

    let updated_config = serde_json::json!({
        "target": {
            "namespace": namespace,
            "path": format!("pod/{}", pod_name)
        },
        "feature": config["feature"]
    });
    let config_str = serde_json::to_string(&updated_config).map_err(|e| {
        tracing::error!(error = %e, "Failed to serialize mirrord config");
        McpError::internal_error("Failed to serialize mirrord config".to_string(), None)
    })?;
    Ok(config_str)
}
