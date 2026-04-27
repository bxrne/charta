use rmcp::ErrorData;
use schemars::JsonSchema;
use scxml::ScxmlError;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
pub struct HealthCheck {}

#[derive(Deserialize, JsonSchema)]
pub struct ValidateStateChart {
    pub state_chart: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct VisualiseStateChart {
    pub state_chart: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("failed to parse state chart: {0}")]
    Parse(ScxmlError),
    #[error("state chart validation failed: {0}")]
    Validate(ScxmlError),
}

impl From<ToolError> for ErrorData {
    fn from(err: ToolError) -> Self {
        match err {
            ToolError::Parse(_) => ErrorData::invalid_params(err.to_string(), None),
            ToolError::Validate(_) => ErrorData::invalid_params(err.to_string(), None),
        }
    }
}
