use anyhow::Result;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::io::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct HealthCheck {}

#[derive(Clone)]
struct Charta {
    tool_router: ToolRouter<Self>,
}

impl Default for Charta {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl Charta {
    #[tool(description = "Simple health check tool for verifying MCP connectivity.")]
    async fn health_check(
        &self,
        Parameters(HealthCheck {}): Parameters<HealthCheck>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![Content::text(format!("OK"))]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Charta {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("charta MCP server. Call health_check to verify connectivity.")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    Charta::default().serve(stdio()).await?.waiting().await?;
    Ok(())
}
