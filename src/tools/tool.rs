use anyhow::Result;
use rmcp::Error as McpError;
use rmcp::model::Content;
use rmcp::{
    ServerHandler,
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool,
};

use super::node;
use super::rust;

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
        description = "Run a rust binary against a Kubernetes service using mirrord to mirror traffic"
    )]
    fn run_rust(&self, #[tool(aggr)] request: rust::Request) -> Result<CallToolResult, McpError> {
        let result = rust::run(request)?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
    #[tool(
        description = "Run a JavaScript script against a Kubernetes service using mirrord to mirror traffic"
    )]
    fn run_node(&self, #[tool(aggr)] request: node::Request) -> Result<CallToolResult, McpError> {
        let result = node::run(request)?;
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
