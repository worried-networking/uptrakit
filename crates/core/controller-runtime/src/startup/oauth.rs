//! Phase 7c: OAuth settings defaults.
//!
//! Seeds the 18 OAuth-related global settings on every boot if they are not
//! already present. Uses [`insert_global_setting_if_absent`] so that operator
//! customisations written after the initial boot are never overwritten.

use rootcause::prelude::*;
use uptrakit_web_api::SettingKey;

use crate::AppError;

/// Seed all 18 OAuth global settings with their spec defaults if absent.
///
/// Idempotent: each call to [`insert_global_setting_if_absent`] is a no-op
/// when the row already exists, so this function may be called on every boot.
pub(crate) async fn seed_oauth_defaults(db: &sea_orm::DatabaseConnection) -> crate::Result<()> {
    macro_rules! seed {
        ($key:expr, $value:expr) => {
            uptrakit_web_api::settings_store::insert_global_setting_if_absent(db, $key, $value)
                .await
                .context(AppError::Settings)?;
        };
    }

    seed!(SettingKey::OauthDcrEnabled, serde_json::json!(false));
    seed!(SettingKey::OauthCimdEnabled, serde_json::json!(false));
    seed!(SettingKey::OauthCanonicalHost, serde_json::Value::Null);
    seed!(
        SettingKey::OauthAcceptedAudienceHosts,
        serde_json::json!([])
    );
    seed!(
        SettingKey::OauthAllowMultiControllerUnsafe,
        serde_json::json!(false)
    );
    seed!(SettingKey::OauthJwtSigningSecret, serde_json::Value::Null);
    seed!(
        SettingKey::OauthAccessTokenTtlSecs,
        serde_json::json!(900_u64)
    );
    seed!(
        SettingKey::OauthRefreshTokenTtlSecs,
        serde_json::json!(2_592_000_u64)
    );
    seed!(
        SettingKey::OauthRefreshFamilyMaxTtlSecs,
        serde_json::json!(7_776_000_u64)
    );
    seed!(
        SettingKey::OauthAuthorizationCodeTtlSecs,
        serde_json::json!(30_u64)
    );
    seed!(
        SettingKey::OauthAuthorizationRequestTtlSecs,
        serde_json::json!(600_u64)
    );
    seed!(SettingKey::OauthRateDcrPerHour, serde_json::json!(10_u32));
    seed!(SettingKey::OauthRateCimdPerMin, serde_json::json!(5_u32));
    seed!(
        SettingKey::OauthRateAuthorizePerMin,
        serde_json::json!(60_u32)
    );
    seed!(SettingKey::OauthRateTokenPerMin, serde_json::json!(60_u32));
    seed!(
        SettingKey::OauthRateConsentPerMin,
        serde_json::json!(60_u32)
    );
    seed!(
        SettingKey::OauthRateMcpAuthFailPerMin,
        serde_json::json!(30_u32)
    );
    seed!(
        SettingKey::OauthCimdCosmeticFieldAllowlist,
        serde_json::json!([])
    );

    tracing::debug!("OAuth setting defaults seeded");
    Ok(())
}
