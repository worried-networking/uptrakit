//! Re-exports of [`CanonicalUrlConfig`] and related types from the shared crate,
//! plus a disabled-mode placeholder constructor for use in [`super::OAuthState::disabled`].

pub use uptrakit_web_api_types::oauth::{
    CanonicalUrlConfig, CanonicalUrlConfigError, CanonicalUrlError, MAX_ACCEPTED_AUDIENCE_HOSTS,
};

/// Returns a [`CanonicalUrlConfig`] that is only valid as a placeholder when OAuth is
/// disabled.
///
/// Uses `disabled.invalid` — a reserved TLD that can never resolve — so that
/// callers that accidentally reach OAuth logic while `enabled = false` fail loudly
/// rather than silently accepting every request.
///
/// # Panics
///
/// This constructor is called only when `oauth.mcp_enabled = false`.  The host
/// `disabled.invalid` satisfies all canonicalisation rules, so the inner
/// `CanonicalUrlConfig::new` call will always succeed.  A panic here indicates
/// a bug in the canonicalisation rules, not a configuration error.
#[must_use]
pub fn disabled_placeholder() -> CanonicalUrlConfig {
    CanonicalUrlConfig::new("disabled.invalid".to_string(), vec![])
        .expect("disabled placeholder is always valid")
}
