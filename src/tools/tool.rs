use anyhow::Result;
use rmcp::model::Content;
use rmcp::{Error as McpError, schemars};
use rmcp::{
    ServerHandler,
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool,
};

use super::executor::execute_mirrord_run;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Request {
    #[schemars(
        description = "Complete command-line statement to run, using absolute paths for binaries, and all necessary flags."
    )]
    cmd_str: String,
    #[schemars(description = "Kubernetes deployment name.")]
    deployment: String,
    #[schemars(
        description = "Mirrord config in JSON format.e.g., '{\"feature\": {\"network\": {\"incoming\": {\"mode\": \"mirror\", \"ports\": [ 8888 ] } } }'."
    )]
    mirrord_config: String,
}
#[derive(Debug, Clone)]
pub struct MirrordService;

impl MirrordService {
    pub fn new() -> Self {
        MirrordService {}
    }
}

#[tool(tool_box)]
impl MirrordService {
    #[tool(
        description = "Run a command-line statement against a Kubernetes service using mirrord to mirror traffic. Use absolute paths for binaries and all necessary flags."
    )]
    async fn run(&self, #[tool(aggr)] request: Request) -> Result<CallToolResult, McpError> {
        let result = execute_mirrord_run(
            &request.cmd_str,
            &request.deployment,
            &request.mirrord_config,
            "default",
        )
        .await?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
}

#[tool(tool_box)]
impl ServerHandler for MirrordService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Mirrord execution service".to_string()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
