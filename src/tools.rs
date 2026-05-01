//! Typed request payloads and the shared error type used by every tool
//! exposed by the [`crate::router::Charta`] MCP handler.
//!
//! Each `*StateChart` struct here doubles as:
//! * a `serde::Deserialize` target for the JSON-RPC tool arguments, and
//! * a `schemars::JsonSchema` source so the MCP framework can advertise the
//!   tool's input schema to clients.

use rmcp::ErrorData;
use schemars::JsonSchema;
use scxml::ScxmlError;
use serde::Deserialize;



/// Arguments for the `validate_state_chart` tool.
///
/// `state_chart` is the raw SCXML XML document the caller wants validated.
#[derive(Deserialize, JsonSchema)]
pub struct ValidateStateChart {
    /// Raw SCXML XML to parse and structurally validate.
    pub state_chart: String,
}

/// Arguments for the `verify_state_chart` tool.
///
/// `state_chart` is the raw SCXML XML document the caller wants verified if it contains any
/// verification.
#[derive(Deserialize, JsonSchema)]
pub struct VerifyStateChart {
    /// Raw SCXML XML to parse and formally verify 
    pub state_chart: String,
    // Verifaction tool to use, e.g. "smt" or "k-induction"
    pub tool: VerificationTool,
}

// Supported verification tools. 
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy)]
pub enum VerificationTool {
    Smt,
    KInduction
}



/// Arguments for the `visualise_state_chart` tool.
#[derive(Deserialize, JsonSchema)]
pub struct VisualiseStateChart {
    /// Raw SCXML XML to render as a Mermaid diagram.
    pub state_chart: String,
}

/// Target language for `codegen_state_chart`. Mirrors the languages
/// supported by `sce-codegen` from scxml-core-engine.
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Rust backend — emits `chart_sm.rs` with a `StatePolicy` trait impl.
    Rust,
    /// Go backend — emits `chart_sm.go` (requires Go 1.22+ for generics).
    Go,
    /// C++ backend — emits `chart_sm.h` + `chart_sm.inl` (CRTP).
    Cpp,
    /// Kotlin backend — emits `chartSm.kt` with sealed interfaces / coroutines.
    Kotlin,
    /// C11 backend — emits `chart_sm.h` + `chart_sm.c` for MCU / embedded use.
    C11,
}

impl Backend {
    /// CLI flag value passed to `sce-codegen --language`.
    ///
    /// The mapping is intentionally explicit (rather than `Debug`-derived) so
    /// that renaming a variant in Rust does not silently change the wire flag.
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

/// Arguments for the `codegen_state_chart` tool.
#[derive(Deserialize, JsonSchema)]
pub struct CodegenStateChart {
    /// Raw SCXML XML to feed into `sce-codegen`.
    pub state_chart: String,
    /// Target language; controls which files `sce-codegen` produces.
    pub backend: Backend,
}

/// Unified error type for all tool handlers.
///
/// The `From<ToolError> for ErrorData` impl below maps each variant onto an
/// appropriate JSON-RPC error category (invalid params vs. internal error)
/// so callers receive structured failures instead of free-form text shoved
/// into a success payload.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// SCXML XML failed to parse (malformed XML or unknown element).
    #[error("failed to parse state chart: {0}")]
    Parse(ScxmlError),
    /// SCXML parsed but failed semantic / structural validation.
    #[error("state chart validation failed: {0}")]
    Validate(ScxmlError),
    /// Filesystem or process I/O failure (writing temp file, reading output, …).
    #[error("io error during codegen: {0}")]
    Io(#[from] std::io::Error),
    /// `sce-codegen` (or `$SCE_CODEGEN_BIN`) was not found on the search path.
    #[error(
        "sce-codegen binary not found. Install with `cargo install --git https://github.com/newmassrael/scxml-core-engine sce-build --features cli` or set SCE_CODEGEN_BIN"
    )]
    BinaryNotFound,
    /// `sce-codegen` ran but exited non-zero; payload is its captured stderr.
    #[error("sce-codegen failed: {0}")]
    CodegenFailed(String),

    // `verify` ran but failed. artefacts returned.
    #[error("verification failed")]
    VerifyFailed
}

impl From<ToolError> for ErrorData {
    /// Project the rich [`ToolError`] onto the limited MCP error vocabulary.
    ///
    /// * Bad SCXML input is the caller's fault → `invalid_params`.
    /// * Anything else (missing binary, I/O, codegen crash) is server-side →
    ///   `internal_error`.
    fn from(err: ToolError) -> Self {
        let msg = err.to_string();
        match err {
            ToolError::Parse(_) | ToolError::Validate(_) => ErrorData::invalid_params(msg, None),
            ToolError::VerifyFailed |
            ToolError::BinaryNotFound | ToolError::Io(_) | ToolError::CodegenFailed(_) => {
                ErrorData::internal_error(msg, None)
            }
        }
    }
}
