use anyhow::Result;
use rmcp::Error as McpError;
use std::ffi::OsString;
use std::path::Path;

/// Trait for language-specific logic needed to run code via mirrord.
pub trait MirrordRunnable {
    /// Sets up the temporary project directory.
    /// This includes writing source files, dependency manifests (like package.json, Cargo.toml, requirements.txt),
    /// and potentially installing dependencies.
    async fn setup_project(&self, project_dir: &Path) -> Result<(), McpError>;

    /// Gets the command and arguments to execute via `mirrord exec`.
    /// This typically includes the language interpreter/binary and the main script/executable path.
    fn get_command_args(&self, project_dir: &Path) -> Result<Vec<OsString>, McpError>;
}
