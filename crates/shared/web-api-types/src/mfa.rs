use serde::{Deserialize, Serialize};
use std::fmt;

use crate::validation::{Validate, ValidationError};

/// Identifies the MFA method used in a challenge verification request.
///
/// Deserialized from HTTP bodies — uses infallible custom `Deserialize` with
/// `Other(String)` so unknown methods never cause a 400 parse error.
/// Loses `Copy` due to `String`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MfaMethod {
    Totp,
    Email,
    RecoveryCode,
    /// Unknown method from a future client; verified as false.
    Other(String),
}

impl MfaMethod {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Totp => "totp",
            Self::Email => "email",
            Self::RecoveryCode => "recovery_code",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for MfaMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for MfaMethod {
    fn from(s: String) -> Self {
        match s.as_str() {
            "totp" => Self::Totp,
            "email" => Self::Email,
            "recovery_code" => Self::RecoveryCode,
            _ => Self::Other(s),
        }
    }
}

impl<'de> Deserialize<'de> for MfaMethod {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Self::from(s))
    }
}

/// Returned by `POST /api/v1/auth/login` when the user has 2FA enrolled.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct MfaChallengeResponse {
    pub mfa_token: String,
    pub mfa_methods: Vec<MfaMethod>,
}

impl MfaChallengeResponse {
    pub fn new(mfa_token: String, mfa_methods: Vec<MfaMethod>) -> Self {
        Self {
            mfa_token,
            mfa_methods,
        }
    }
}

/// Body for `POST /api/v1/auth/mfa/verify`.
#[derive(Debug, Serialize, Deserialize)]
pub struct MfaVerifyRequest {
    pub mfa_token: String,
    pub code: String,
    pub method: MfaMethod,
}

impl Validate for MfaVerifyRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.mfa_token.is_empty() {
            return Err(ValidationError {
                field: "mfa_token",
                message: "mfa_token must not be empty".to_string(),
            });
        }
        if self.code.is_empty() {
            return Err(ValidationError {
                field: "code",
                message: "code must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

/// Body for `POST /api/v1/auth/mfa/email`.
#[derive(Debug, Serialize, Deserialize)]
pub struct MfaEmailRequest {
    pub mfa_token: String,
}

impl Validate for MfaEmailRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.mfa_token.is_empty() {
            return Err(ValidationError {
                field: "mfa_token",
                message: "mfa_token must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

/// Returned by `GET /api/v1/auth/me/2fa`.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct MfaStatusResponse {
    pub totp_enrolled: bool,
    pub recovery_codes_count: u32,
    pub methods_available: Vec<MfaMethod>,
}

impl MfaStatusResponse {
    /// Construct a new [`MfaStatusResponse`].
    #[must_use]
    pub fn new(
        totp_enrolled: bool,
        recovery_codes_count: u32,
        methods_available: Vec<MfaMethod>,
    ) -> Self {
        Self {
            totp_enrolled,
            recovery_codes_count,
            methods_available,
        }
    }
}

/// Returned by `POST /api/v1/auth/me/2fa/totp/enroll`.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct TotpEnrollResponse {
    /// `otpauth://totp/` URI for QR generation in the browser.
    pub otpauth_uri: String,
    /// Human-readable base32 secret (for manual entry).
    pub secret: String,
}

impl TotpEnrollResponse {
    /// Construct a new [`TotpEnrollResponse`].
    #[must_use]
    pub fn new(otpauth_uri: String, secret: String) -> Self {
        Self {
            otpauth_uri,
            secret,
        }
    }
}

/// Body for `POST /api/v1/auth/me/2fa/totp/confirm`.
#[derive(Serialize, Deserialize)]
pub struct TotpConfirmRequest {
    pub code: String,
}

impl Validate for TotpConfirmRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.code.len() != 6 || !self.code.chars().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError {
                field: "code",
                message: "code must be exactly 6 digits".to_string(),
            });
        }
        Ok(())
    }
}

/// Returned by `POST /api/v1/auth/me/2fa/totp/confirm`.
#[non_exhaustive]
#[derive(Serialize, Deserialize)]
pub struct TotpConfirmResponse {
    /// Plaintext recovery codes shown once.
    pub recovery_codes: Vec<String>,
    /// New full-session tokens (replaces the restricted session, if any).
    pub session: Option<crate::auth::AuthResponse>,
}

impl TotpConfirmResponse {
    /// Construct a new [`TotpConfirmResponse`].
    #[must_use]
    pub fn new(recovery_codes: Vec<String>, session: Option<crate::auth::AuthResponse>) -> Self {
        Self {
            recovery_codes,
            session,
        }
    }
}

/// Body for `POST /api/v1/auth/me/2fa/totp/disable`.
#[derive(Debug, Serialize, Deserialize)]
pub struct DisableTotpRequest {
    pub password: Option<uptrakit_shared_types::SecretString>,
    pub totp_code: Option<String>,
}

impl Validate for DisableTotpRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        match (&self.password, &self.totp_code) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            _ => Err(ValidationError {
                field: "password",
                message: "exactly one of password or totp_code must be provided".to_string(),
            }),
        }
    }
}

/// Body for `POST /api/v1/auth/me/2fa/recovery-codes/regenerate`.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegenerateRecoveryCodesRequest {
    pub password: Option<uptrakit_shared_types::SecretString>,
    pub totp_code: Option<String>,
}

impl Validate for RegenerateRecoveryCodesRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        match (&self.password, &self.totp_code) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            _ => Err(ValidationError {
                field: "password",
                message: "exactly one of password or totp_code must be provided".to_string(),
            }),
        }
    }
}

/// Returned by `POST /api/v1/auth/me/2fa/recovery-codes/regenerate`.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct RegenerateRecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}

impl RegenerateRecoveryCodesResponse {
    /// Construct a new [`RegenerateRecoveryCodesResponse`].
    #[must_use]
    pub fn new(recovery_codes: Vec<String>) -> Self {
        Self { recovery_codes }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    // ── MfaMethod enum ───────────────────────────────────────────────────────

    #[test]
    fn mfa_method_totp_as_str() {
        assert_eq!(MfaMethod::Totp.as_str(), "totp");
    }

    #[test]
    fn mfa_method_email_as_str() {
        assert_eq!(MfaMethod::Email.as_str(), "email");
    }

    #[test]
    fn mfa_method_recovery_code_as_str() {
        assert_eq!(MfaMethod::RecoveryCode.as_str(), "recovery_code");
    }

    #[test]
    fn mfa_method_other_as_str() {
        let other = MfaMethod::Other("future_method".to_string());
        assert_eq!(other.as_str(), "future_method");
    }

    #[test]
    fn mfa_method_display_matches_as_str() {
        assert_eq!(format!("{}", MfaMethod::Totp), "totp");
        assert_eq!(format!("{}", MfaMethod::Email), "email");
        assert_eq!(format!("{}", MfaMethod::RecoveryCode), "recovery_code");
    }

    #[test]
    fn mfa_method_from_string_totp() {
        assert_eq!(MfaMethod::from("totp".to_string()), MfaMethod::Totp);
    }

    #[test]
    fn mfa_method_from_string_email() {
        assert_eq!(MfaMethod::from("email".to_string()), MfaMethod::Email);
    }

    #[test]
    fn mfa_method_from_string_recovery_code() {
        assert_eq!(
            MfaMethod::from("recovery_code".to_string()),
            MfaMethod::RecoveryCode
        );
    }

    #[test]
    fn mfa_method_from_string_unknown() {
        let unknown = MfaMethod::from("future_method".to_string());
        assert!(matches!(
            unknown,
            MfaMethod::Other(ref s) if s == "future_method"
        ));
    }

    #[test]
    fn mfa_method_serde_round_trip_totp() {
        let method = MfaMethod::Totp;
        let json = serde_json::to_string(&method).unwrap();
        let deserialized: MfaMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, method);
    }

    #[test]
    fn mfa_method_serde_round_trip_recovery_code() {
        let method = MfaMethod::RecoveryCode;
        let json = serde_json::to_string(&method).unwrap();
        let deserialized: MfaMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, method);
    }

    #[test]
    fn mfa_method_deserialize_unknown() {
        let json = r#""future_method""#;
        let method: MfaMethod = serde_json::from_str(json).unwrap();
        assert!(matches!(
            method,
            MfaMethod::Other(ref s) if s == "future_method"
        ));
    }

    // ── MfaVerifyRequest ─────────────────────────────────────────────────────

    fn valid_mfa_verify() -> MfaVerifyRequest {
        MfaVerifyRequest {
            mfa_token: "token_123".to_string(),
            code: "123456".to_string(),
            method: MfaMethod::Totp,
        }
    }

    #[test]
    fn mfa_verify_request_valid() {
        assert!(valid_mfa_verify().validate().is_ok());
    }

    #[test]
    fn mfa_verify_request_empty_token() {
        let mut req = valid_mfa_verify();
        req.mfa_token = String::new();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "mfa_token");
    }

    #[test]
    fn mfa_verify_request_empty_code() {
        let mut req = valid_mfa_verify();
        req.code = String::new();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "code");
    }

    // ── MfaEmailRequest ──────────────────────────────────────────────────────

    fn valid_mfa_email() -> MfaEmailRequest {
        MfaEmailRequest {
            mfa_token: "token_123".to_string(),
        }
    }

    #[test]
    fn mfa_email_request_valid() {
        assert!(valid_mfa_email().validate().is_ok());
    }

    #[test]
    fn mfa_email_request_empty_token() {
        let mut req = valid_mfa_email();
        req.mfa_token = String::new();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "mfa_token");
    }

    // ── TotpConfirmRequest ───────────────────────────────────────────────────

    #[test]
    fn totp_confirm_request_valid() {
        let req = TotpConfirmRequest {
            code: "123456".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn totp_confirm_request_not_digits() {
        let req = TotpConfirmRequest {
            code: "12345a".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "code");
    }

    #[test]
    fn totp_confirm_request_too_short() {
        let req = TotpConfirmRequest {
            code: "12345".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "code");
    }

    #[test]
    fn totp_confirm_request_too_long() {
        let req = TotpConfirmRequest {
            code: "1234567".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "code");
    }

    // ── DisableTotpRequest ───────────────────────────────────────────────────

    #[test]
    fn disable_totp_request_with_password() {
        let req = DisableTotpRequest {
            password: Some(uptrakit_shared_types::SecretString::new("pass123")),
            totp_code: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn disable_totp_request_with_totp_code() {
        let req = DisableTotpRequest {
            password: None,
            totp_code: Some("123456".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn disable_totp_request_with_both() {
        let req = DisableTotpRequest {
            password: Some(uptrakit_shared_types::SecretString::new("pass123")),
            totp_code: Some("123456".to_string()),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    #[test]
    fn disable_totp_request_with_neither() {
        let req = DisableTotpRequest {
            password: None,
            totp_code: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    // ── RegenerateRecoveryCodesRequest ───────────────────────────────────────

    #[test]
    fn regenerate_recovery_codes_request_with_password() {
        let req = RegenerateRecoveryCodesRequest {
            password: Some(uptrakit_shared_types::SecretString::new("pass123")),
            totp_code: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn regenerate_recovery_codes_request_with_totp_code() {
        let req = RegenerateRecoveryCodesRequest {
            password: None,
            totp_code: Some("123456".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn regenerate_recovery_codes_request_with_both() {
        let req = RegenerateRecoveryCodesRequest {
            password: Some(uptrakit_shared_types::SecretString::new("pass123")),
            totp_code: Some("123456".to_string()),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    #[test]
    fn regenerate_recovery_codes_request_with_neither() {
        let req = RegenerateRecoveryCodesRequest {
            password: None,
            totp_code: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    // ── Struct serialization round-trips ─────────────────────────────────────

    #[test]
    fn mfa_challenge_response_round_trip() {
        let resp = MfaChallengeResponse {
            mfa_token: "token_abc".to_string(),
            mfa_methods: vec![MfaMethod::Totp, MfaMethod::Email],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: MfaChallengeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.mfa_token, "token_abc");
        assert_eq!(deserialized.mfa_methods.len(), 2);
        assert_eq!(deserialized.mfa_methods[0], MfaMethod::Totp);
        assert_eq!(deserialized.mfa_methods[1], MfaMethod::Email);
    }

    #[test]
    fn mfa_status_response_round_trip() {
        let resp = MfaStatusResponse {
            totp_enrolled: true,
            recovery_codes_count: 5,
            methods_available: vec![MfaMethod::Totp, MfaMethod::Email, MfaMethod::RecoveryCode],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: MfaStatusResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.totp_enrolled);
        assert_eq!(deserialized.recovery_codes_count, 5);
        assert_eq!(deserialized.methods_available.len(), 3);
    }

    #[test]
    fn totp_enroll_response_round_trip() {
        let resp = TotpEnrollResponse {
            otpauth_uri: "otpauth://totp/test".to_string(),
            secret: "JBSWY3DPEBLW64TMMQ======".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: TotpEnrollResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.otpauth_uri, "otpauth://totp/test");
        assert_eq!(deserialized.secret, "JBSWY3DPEBLW64TMMQ======");
    }

    #[test]
    fn totp_confirm_response_round_trip() {
        let resp = TotpConfirmResponse {
            recovery_codes: vec!["code1".to_string(), "code2".to_string()],
            session: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: TotpConfirmResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.recovery_codes.len(), 2);
        assert!(deserialized.session.is_none());
    }

    #[test]
    fn regenerate_recovery_codes_response_round_trip() {
        let resp = RegenerateRecoveryCodesResponse {
            recovery_codes: vec![
                "code1".to_string(),
                "code2".to_string(),
                "code3".to_string(),
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: RegenerateRecoveryCodesResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.recovery_codes.len(), 3);
    }
}
