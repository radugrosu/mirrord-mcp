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
