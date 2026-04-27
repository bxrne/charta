use std::io::Write;

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tokio::process::Command;

use scxml::{export, parse_xml, validate};

use crate::tools::{
    CodegenStateChart, ToolError, ValidateStateChart, VisualiseStateChart,
};

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
    #[tool(
        description = "Validate a state chart XML. Returns OK if valid, otherwise returns an error message."
    )]
    pub async fn validate_state_chart(
        &self,
        Parameters(ValidateStateChart { state_chart }): Parameters<ValidateStateChart>,
    ) -> Result<CallToolResult, ErrorData> {
        let chart = parse_xml(&state_chart).map_err(ToolError::Parse)?;
        validate(&chart).map_err(ToolError::Validate)?;
        Ok(CallToolResult::success(vec![Content::text(format!("OK"))]))
    }

    #[tool(
        description = "Visualise a valid state chart XML as a Mermaid diagram. Returns the Mermaid syntax for the diagram."
    )]
    pub async fn visualise_state_chart(
        &self,
        Parameters(VisualiseStateChart { state_chart }): Parameters<VisualiseStateChart>,
    ) -> Result<CallToolResult, ErrorData> {
        let chart = parse_xml(&state_chart).map_err(ToolError::Parse)?;
        validate(&chart).map_err(ToolError::Validate)?;

        let dot = export::mermaid::to_mermaid(&chart);
        Ok(CallToolResult::success(vec![Content::text(dot)]))
    }

    #[tool(
        description = "Generate source code from an SCXML state chart for a target backend. Supported backends: rust, go, cpp, kotlin, c11. Returns the generated code as text. Requires the `sce-codegen` binary on PATH (override with SCE_CODEGEN_BIN)."
    )]
    pub async fn codegen_state_chart(
        &self,
        Parameters(CodegenStateChart {
            state_chart,
            backend,
        }): Parameters<CodegenStateChart>,
    ) -> Result<CallToolResult, ErrorData> {
        // Fail fast on bad SCXML before spawning a subprocess.
        let chart = parse_xml(&state_chart).map_err(ToolError::Parse)?;
        validate(&chart).map_err(ToolError::Validate)?;

        let workdir = tempfile::tempdir().map_err(ToolError::Io)?;
        let stem = "chart";
        let scxml_path = workdir.path().join(format!("{stem}.scxml"));
        {
            let mut f = std::fs::File::create(&scxml_path).map_err(ToolError::Io)?;
            f.write_all(state_chart.as_bytes()).map_err(ToolError::Io)?;
        }

        let bin = std::env::var("SCE_CODEGEN_BIN").unwrap_or_else(|_| "sce-codegen".into());
        let out_dir = workdir.path().join("out");
        std::fs::create_dir_all(&out_dir).map_err(ToolError::Io)?;

        let output = Command::new(&bin)
            .arg("generate")
            .arg(&scxml_path)
            .arg("-l")
            .arg(backend.as_flag())
            .arg("-o")
            .arg(&out_dir)
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ToolError::BinaryNotFound
                } else {
                    ToolError::Io(e)
                }
            })?;

        if !output.status.success() {
            return Err(ToolError::CodegenFailed(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            )
            .into());
        }

        // Collect every generated file (rust/go = 1 file, cpp = 2 files).
        let mut contents = Vec::new();
        let entries = std::fs::read_dir(&out_dir).map_err(ToolError::Io)?;
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        files.sort_by_key(|e| e.path());
        for entry in files {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("<unknown>")
                .to_string();
            let body = std::fs::read_to_string(&path).map_err(ToolError::Io)?;
            contents.push(Content::text(format!("// === {name} ===\n{body}")));
        }

        if contents.is_empty() {
            return Err(ToolError::CodegenFailed(format!(
                "sce-codegen produced no files for backend {:?}",
                backend
            ))
            .into());
        }

        Ok(CallToolResult::success(contents))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Charta {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "charta MCP server. Tools: validate_state_chart, visualise_state_chart, codegen_state_chart.",
            )
    }
}
