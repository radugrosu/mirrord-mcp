use super::executor::execute_mirrord_run;
use super::runnable::MirrordRunnable;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::ffi::OsString; // Use OsString for command args
use std::path::Path;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Request {
    #[schemars(
        description = "Complete Python code using only requests for HTTP requests and json for deserialization. The resulting script is run against the cluster."
    )]
    code: String,
    #[schemars(description = "Kubernetes deployment name.")]
    deployment: String,
    #[schemars(
        description = "Mirrord config in JSON format, e.g., '{\"feature\": {\"network\": {\"incoming\": {\"mode\": \"mirror\", \"ports\": [8888]}}}}'."
    )]
    mirrord_config: String,
}
struct PythonRunner<'a> {
    code: &'a str,
}
impl MirrordRunnable for PythonRunner<'_> {
    fn setup_project(&self, project_dir: &Path) -> Result<(), McpError> {
        let main_py_path = project_dir.join("main.py");
        std::fs::write(&main_py_path, self.code).map_err(|e| {
            tracing::error!(error = %e, path = %main_py_path.display(), "Failed to write main.py");
            McpError::internal_error("Failed to write main.py".to_string(), None)
        })?;
        tracing::debug!(
            "Wrote main.py to {} with code length: {} bytes",
            main_py_path.display(),
            self.code.len()
        );

        // -- Handle Python dependencies --
        let requirements = "requests\n"; // Assuming 'json' is standard lib, but 'requests' needs install
        let req_path = project_dir.join("requirements.txt");
        std::fs::write(&req_path, requirements).map_err(|e| {
             tracing::error!(error = %e, path = %req_path.display(), "Failed to write requirements.txt");
            McpError::internal_error("Failed to write requirements.txt".to_string(), None)
        })?;

        let venv_path = project_dir.join(".venv");
        let venv_output = std::process::Command::new("python3")
            .arg("-m")
            .arg("venv")
            .arg(&venv_path)
            .output()
            .map_err(|e| {
                McpError::internal_error(format!("Failed to create python venv: {}", e), None)
            })?;
        if !venv_output.status.success() {
            let stderr = String::from_utf8_lossy(&venv_output.stderr);
            tracing::error!(error=%stderr, "Failed to create python venv");
            return Err(McpError::internal_error(
                format!("Failed to create python venv: {}", stderr),
                None,
            ));
        }

        // Install dependencies into venv
        let pip_path = venv_path.join("bin").join("pip");
        let install_output = std::process::Command::new(pip_path)
            .arg("install")
            .arg("-r")
            .arg(&req_path)
            .output()
            .map_err(|e| {
                McpError::internal_error(format!("Failed to run pip install: {}", e), None)
            })?;

        if !install_output.status.success() {
            let stderr = String::from_utf8_lossy(&install_output.stderr);
            tracing::error!(error=%stderr, "pip install failed");
            return Err(McpError::internal_error(
                format!("pip install failed: {}", stderr),
                None,
            ));
        }
        tracing::info!("Python dependencies installed successfully.");
        Ok(())
    }

    fn get_command_args(&self, project_dir: &Path) -> Result<Vec<OsString>, McpError> {
        let python_executable = project_dir.join(".venv").join("bin").join("python");
        let script_path = project_dir.join("main.py");
        Ok(vec![
            python_executable.into(), // Convert PathBuf to OsString
            script_path.into(),
        ])
    }
}

pub fn run(request: Request) -> Result<String, McpError> {
    // Create the runner instance, passing necessary data from the request
    let runner = PythonRunner {
        code: &request.code,
    };

    // Call the shared executor function with the runner instance
    execute_mirrord_run(
        &runner,
        &request.deployment,
        &request.mirrord_config,
        "default", // Namespace - make configurable later if needed
    )
}
