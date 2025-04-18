use super::executor::execute_mirrord_run; // Import the shared executor
use super::runnable::MirrordRunnable; // Import the trait definition
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::ffi::OsString; // Needed for command args
use std::path::Path;
use std::process::Command; // Needed for npm install

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

// Struct to hold Node.js-specific data and implement the trait
struct NodeRunner<'a> {
    code: &'a str,
}

impl MirrordRunnable for NodeRunner<'_> {
    fn setup_project(&self, project_dir: &Path) -> Result<(), McpError> {
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
        let npm_install_output = Command::new("npm")
            .current_dir(project_dir)
            .arg("install")
            .output()
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to execute npm install");
                // Check if the error is 'NotFound'
                if e.kind() == std::io::ErrorKind::NotFound {
                    McpError::internal_error(
                        "Failed to run npm: 'npm' command not found in PATH.".to_string(),
                        None,
                    )
                } else {
                    McpError::internal_error(format!("Failed to start npm process: {}", e), None)
                }
            })?;

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

pub fn run(request: Request) -> Result<String, McpError> {
    let runner = NodeRunner {
        code: &request.code,
    };

    execute_mirrord_run(
        &runner,
        &request.deployment,
        &request.mirrord_config,
        "default",
    )
}
