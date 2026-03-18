//! Request/response types for the Dashboard Icons settings API.
//!
//! `GET /api/v1/settings/dashboard-icons` returns [`DashboardIconsSettingsResponse`].
//! `PUT /api/v1/settings/dashboard-icons` accepts [`UpdateDashboardIconsSettingsRequest`].
//!
//! Dashboard Icons is an optional enhancement that automatically assigns icon
//! URLs to software items using the community-curated [Dashboard Icons](https://github.com/homarr-labs/dashboard-icons)
//! collection. The setting is per-tenant and enabled by default.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// Response body for `GET /api/v1/settings/dashboard-icons`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DashboardIconsSettingsResponse {
    /// Whether Dashboard Icons enrichment is enabled for this tenant.
    pub enabled: bool,
}

/// Request body for `PUT /api/v1/settings/dashboard-icons`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateDashboardIconsSettingsRequest {
    /// Whether Dashboard Icons enrichment should be enabled.
    pub enabled: bool,
}

impl Validate for UpdateDashboardIconsSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_round_trip() {
        let resp = DashboardIconsSettingsResponse { enabled: true };
        let json = serde_json::to_string(&resp).expect("serialize");
        let de: DashboardIconsSettingsResponse = serde_json::from_str(&json).expect("deserialize");
        assert!(de.enabled);
    }

    #[test]
    fn response_disabled() {
        let resp = DashboardIconsSettingsResponse { enabled: false };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn request_round_trip() {
        let req = UpdateDashboardIconsSettingsRequest { enabled: true };
        let json = serde_json::to_string(&req).expect("serialize");
        let de: UpdateDashboardIconsSettingsRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert!(de.enabled);
    }

    #[test]
    fn validate_always_succeeds() {
        let req = UpdateDashboardIconsSettingsRequest { enabled: true };
        assert!(req.validate().is_ok());
        let req = UpdateDashboardIconsSettingsRequest { enabled: false };
        assert!(req.validate().is_ok());
    }
}
