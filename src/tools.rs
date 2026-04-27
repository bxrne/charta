use rmcp::ErrorData;
use schemars::JsonSchema;
use scxml::ScxmlError;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
pub struct ValidateStateChart {
    pub state_chart: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct VisualiseStateChart {
    pub state_chart: String,
}

/// Target language for `codegen_state_chart`. Mirrors the languages
/// supported by `sce-codegen` from scxml-core-engine.
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Rust,
    Go,
    Cpp,
    Kotlin,
    C11,
}

impl Backend {
    /// CLI flag value passed to `sce-codegen --language`.
    pub fn as_flag(self) -> &'static str {
        match self {
            Backend::Rust => "rust",
            Backend::Go => "go",
            Backend::Cpp => "cpp",
            Backend::Kotlin => "kotlin",
            Backend::C11 => "c11",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct CodegenStateChart {
    /// SCXML XML source.
    pub state_chart: String,
    /// Target language: `rust`, `go`, or `cpp`.
    pub backend: Backend,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("failed to parse state chart: {0}")]
    Parse(ScxmlError),
    #[error("state chart validation failed: {0}")]
    Validate(ScxmlError),
    #[error("io error during codegen: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "sce-codegen binary not found. Install with `cargo install --git https://github.com/newmassrael/scxml-core-engine sce-build --features cli` or set SCE_CODEGEN_BIN"
    )]
    BinaryNotFound,
    #[error("sce-codegen failed: {0}")]
    CodegenFailed(String),
}

impl From<ToolError> for ErrorData {
    fn from(err: ToolError) -> Self {
        let msg = err.to_string();
        match err {
            ToolError::Parse(_) | ToolError::Validate(_) => ErrorData::invalid_params(msg, None),
            ToolError::BinaryNotFound
            | ToolError::Io(_)
            | ToolError::CodegenFailed(_) => ErrorData::internal_error(msg, None),
        }
    }
}
