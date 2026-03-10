use serde::{Deserialize, Serialize};

/// An access preset definition with its role composition.
#[derive(Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessPresetResponse {
    pub name: String,
    pub description: String,
    pub roles: Vec<String>,
}
