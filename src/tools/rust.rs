use super::executor::execute_mirrord_run;
use super::runnable::MirrordRunnable;
use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::schemars;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::task;
use tokio::time::timeout;

const CARGO_BUILD_TIMEOUT: Duration = Duration::from_secs(180);

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

// Struct to hold Rust-specific data and implement the trait
struct RustRunner<'a> {
    code: &'a str,
    compile_mode: String,
}

impl MirrordRunnable for RustRunner<'_> {
    async fn setup_project(&self, project_dir: &Path) -> Result<(), McpError> {
        // Ensure src directory exists
        let src_dir = project_dir.join("src");
        std::fs::create_dir_all(&src_dir).map_err(|e| {
            tracing::error!(error = %e, path = %src_dir.display(), "Failed to create src directory");
            McpError::internal_error("Failed to create src directory".to_string(), None)
        })?;

        // Write Cargo.toml
        let mut cargo_toml = r#"
[package]
name = "mirrord-agent-code"
version = "0.1.0"
edition = "2021" # Consider updating to 2024 if appropriate, but 2021 is safer for broader compiler support

[dependencies]
reqwest = { version = "0.12", features = ["json", "blocking"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
"#
        .to_string();
        // Add debug profile settings if needed
        if self.compile_mode == "debug" {
            // Note: These settings are very aggressive in order to minimize compile time
            cargo_toml.push_str(
                r#"
[profile.dev]
opt-level = 0
# debug = false # Usually want debug symbols in debug mode
# split-debuginfo = "unpacked" # Useful for large projects, maybe overkill here
# debug-assertions = false # Usually want assertions in debug mode
# overflow-checks = false # Usually want overflow checks in debug mode
lto = false
panic = "unwind" # 'abort' can make debugging harder
incremental = true
codegen-units = 256 # Default is usually fine
"#,
            );
        }

        let cargo_toml_path = project_dir.join("Cargo.toml");
        std::fs::write(&cargo_toml_path, cargo_toml).map_err(|e| {
            tracing::error!(error = %e, path = %cargo_toml_path.display(), "Failed to write Cargo.toml");
            McpError::internal_error("Failed to write Cargo.toml".to_string(), None)
        })?;
        tracing::debug!("Wrote Cargo.toml to {}", cargo_toml_path.display());

        // Write main.rs
        let main_rs_path = src_dir.join("main.rs");
        std::fs::write(&main_rs_path, self.code).map_err(|e| {
            tracing::error!(error = %e, path = %main_rs_path.display(), "Failed to write main.rs");
            McpError::internal_error("Failed to write main.rs".to_string(), None)
        })?;
        tracing::debug!(
            "Wrote main.rs to {} with code length: {} bytes",
            main_rs_path.display(),
            self.code.len()
        );

        // Compile
        tracing::info!(
            "Compiling rust cod in {} mode in {}",
            self.compile_mode,
            project_dir.display()
        );
        let compile_args = match self.compile_mode.as_str() {
            "debug" => &["build"][..],
            _ => &["build", "--release"][..], // Default to release
        };

        let project_dir_owned = project_dir.to_path_buf(); // Clone PathBuf to move into task
        let blocking_task = task::spawn_blocking(move || {
            let mut command = Command::new("cargo");
            command
                .current_dir(project_dir_owned)
                .args(compile_args)
                .output()
        });

        let compile_output = match timeout(CARGO_BUILD_TIMEOUT, blocking_task).await {
            Ok(Ok(Ok(output))) => Ok(output), // All succeeded
            Ok(Ok(Err(e))) => {
                // Command::output failed
                tracing::error!(error = %e, "Failed to run cargo build");
                if e.kind() == std::io::ErrorKind::NotFound {
                    Err(McpError::internal_error(
                        "Failed to execute cargo: 'cargo' command not found in PATH.".to_string(),
                        None,
                    ))
                } else {
                    Err(McpError::internal_error(
                        format!("Failed to start cargo process: {}", e),
                        None,
                    ))
                }
            }
            Ok(Err(e)) => {
                // spawn_blocking failed
                tracing::error!(error = %e, "cargo blocking task failed");
                Err(McpError::internal_error(
                    format!("cargo task failed: {}", e),
                    None,
                ))
            }
            Err(_) => {
                // Timeout elapsed
                tracing::error!("Cargo build timed out after {:?}", CARGO_BUILD_TIMEOUT);
                Err(McpError::internal_error(
                    format!("Cargo build timed out after {:?}", CARGO_BUILD_TIMEOUT),
                    None,
                ))
            }
        }?;

        if !compile_output.status.success() {
            let stderr = String::from_utf8_lossy(&compile_output.stderr);
            let stdout = String::from_utf8_lossy(&compile_output.stdout); // Include stdout for more context
            tracing::error!(stderr = %stderr, stdout = %stdout, "cargo build failed");
            return Err(McpError::internal_error(
                format!("Rust build failed: {}", stderr), // Primarily report stderr
                None,
            ));
        }
        tracing::info!("Rust compilation succeeded");

        // Check if binary exists (optional sanity check, executor handles command failure)
        let binary_path = self.get_binary_path(project_dir);
        if !binary_path.exists() {
            tracing::error!(
                "Compiled binary not found at expected path: {}",
                binary_path.display()
            );
            // This might indicate an issue with cargo build output or path calculation
            return Err(McpError::internal_error(
                format!(
                    "Compiled binary not found after successful build at: {}",
                    binary_path.display()
                ),
                None,
            ));
        }
        tracing::debug!("Binary confirmed at {}", binary_path.display());

        Ok(())
    }

    fn get_command_args(&self, project_dir: &Path) -> Result<Vec<OsString>, McpError> {
        // Calculate the path to the compiled binary
        let binary_path = self.get_binary_path(project_dir);
        Ok(vec![
            binary_path.into(), // The executable path as an OsString
        ])
    }
}

impl RustRunner<'_> {
    fn get_binary_path(&self, project_dir: &Path) -> PathBuf {
        project_dir
            .join("target")
            .join(&self.compile_mode) // Use the stored compile mode
            .join("mirrord-agent-code") // Match the package name in Cargo.toml
    }
}

pub async fn run(request: Request) -> Result<String, McpError> {
    // Determine compile mode
    let compile_mode =
        std::env::var("MCP_SERVICE_COMPILE_MODE").unwrap_or_else(|_| "release".to_string());

    // Create the runner instance
    let runner = RustRunner {
        code: &request.code,
        compile_mode,
    };

    // Call the shared executor function
    execute_mirrord_run(
        &runner,
        &request.deployment,
        &request.mirrord_config,
        "default",
    )
    .await
}
