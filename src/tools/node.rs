use super::executor::execute_mirrord_run;
use super::runnable::MirrordRunnable;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tokio::task;
use tokio::time::timeout;

const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(180);

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

struct NodeRunner<'a> {
    code: &'a str,
}

impl MirrordRunnable for NodeRunner<'_> {
    async fn setup_project(&self, project_dir: &Path) -> Result<(), McpError> {
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
        let package_json_path = project_dir.join("package.json");
        std::fs::write(&package_json_path, package_json).map_err(|e| {
            tracing::error!(error = %e, path = %package_json_path.display(), "Failed to write package.json");
            McpError::internal_error("Failed to write package.json".to_string(), None)
        })?;
        tracing::debug!("Wrote package.json to {}", package_json_path.display());

        // Write index.js
        let index_js_path = project_dir.join("index.js");
        std::fs::write(&index_js_path, self.code).map_err(|e| {
            tracing::error!(error = %e, path = %index_js_path.display(), "Failed to write index.js");
            McpError::internal_error("Failed to write index.js".to_string(), None)
        })?;
        tracing::debug!(
            "Wrote index.js to {} with code length: {} bytes",
            index_js_path.display(),
            self.code.len()
        );

        // Install dependencies
        tracing::info!(
            "Installing Node.js dependencies in {}",
            project_dir.display()
        );
        let project_dir_owned = project_dir.to_path_buf(); // Clone for task
        let blocking_task = task::spawn_blocking(move || {
            Command::new("npm")
                .current_dir(&project_dir_owned) // Use owned path
                .arg("install")
                .output()
        });
        let npm_install_output = match timeout(NPM_INSTALL_TIMEOUT, blocking_task).await {
            Ok(Ok(Ok(output))) => output,
            Ok(Ok(Err(e))) => {
                tracing::error!(error = %e, "Failed to execute npm install");
                return if e.kind() == std::io::ErrorKind::NotFound {
                    Err(McpError::internal_error(
                        "Failed to run npm: 'npm' command not found in PATH.".to_string(),
                        None,
                    ))
                } else {
                    Err(McpError::internal_error(
                        format!("Failed to start npm process: {}", e),
                        None,
                    ))
                };
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, "npm install blocking task failed");
                return Err(McpError::internal_error(
                    format!("npm install task failed: {}", e),
                    None,
                ));
            }
            Err(_) => {
                tracing::error!("npm install timed out after {:?}", NPM_INSTALL_TIMEOUT);
                return Err(McpError::internal_error(
                    format!("npm install timed out after {:?}", NPM_INSTALL_TIMEOUT),
                    None,
                ));
            }
        };
        if !npm_install_output.status.success() {
            let stderr = String::from_utf8_lossy(&npm_install_output.stderr);
            let stdout = String::from_utf8_lossy(&npm_install_output.stdout);
            tracing::error!(stderr = %stderr, stdout = %stdout, "npm install failed");
            return Err(McpError::internal_error(
                format!("npm install failed: {}", stderr), // Primarily report stderr
                None,
            ));
        }
        tracing::info!("npm install succeeded");
        Ok(())
    }

    fn get_command_args(&self, project_dir: &Path) -> Result<Vec<OsString>, McpError> {
        let script_path = project_dir.join("index.js");
        Ok(vec![
            OsString::from("node"), // The executable
            script_path.into(),     // The script path as an OsString
        ])
    }
}

pub async fn run(request: Request) -> Result<String, McpError> {
    let runner = NodeRunner {
        code: &request.code,
    };

    execute_mirrord_run(
        &runner,
        &request.deployment,
        &request.mirrord_config,
        "default",
    )
    .await
}
