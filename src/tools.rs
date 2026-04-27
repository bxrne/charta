use schemars::JsonSchema;
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
