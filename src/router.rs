use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::tools::{HealthCheck, ValidateStateChart};
#[derive(Clone)]
pub struct Charta {
    pub tool_router: ToolRouter<Self>,
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
    pub async fn health_check(
        &self,
        Parameters(HealthCheck {}): Parameters<HealthCheck>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![Content::text(format!("OK"))]))
    }

    #[tool(
        description = "Validate a state chart. Returns OK if valid, otherwise returns an error message."
    )]
    pub async fn validate_state_chart(
        &self,
        Parameters(ValidateStateChart { state_chart }): Parameters<ValidateStateChart>,
    ) -> Result<CallToolResult, ErrorData> {
        // TODO: validate state chart against a bunch of properties
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
