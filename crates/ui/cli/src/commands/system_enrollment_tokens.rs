use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::system_enrollment_tokens::{
    CreateSystemEnrollmentTokenRequest, ListSystemEnrollmentTokensQuery,
    SystemEnrollmentTokenCreatedResponse, SystemEnrollmentTokenResponse,
};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

use crate::commands::settings::DeletedOutput;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<SystemEnrollmentTokenResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No system enrollment tokens found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<12} STATUS\n",
            "ID", "NAME", "USAGE"
        );
        for t in &self.items {
            let usage = match t.max_uses {
                Some(max) => format!("{}/{}", t.current_uses, max),
                None => format!("{}/∞", t.current_uses),
            };
            let status = if t.revoked_at.is_some() {
                "revoked"
            } else {
                "active"
            };
            out.push_str(&format!(
                "{:<38} {:<25} {:<12} {}\n",
                t.id, t.name, usage, status
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for SystemEnrollmentTokenResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:             {}\n", self.id));
        out.push_str(&format!("Name:           {}\n", self.name));
        match self.max_uses {
            Some(max) => out.push_str(&format!("Usage:          {}/{}\n", self.current_uses, max)),
            None => out.push_str(&format!(
                "Usage:          {} (unlimited)\n",
                self.current_uses
            )),
        }
        if let Some(expires) = self.expires_at {
            out.push_str(&format!(
                "Expires:        {}\n",
                expires
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| expires.to_string())
            ));
        } else {
            out.push_str("Expires:        never\n");
        }
        out.push_str(&format!(
            "Created:        {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        if let Some(revoked) = self.revoked_at {
            out.push_str(&format!(
                "Revoked:        {}\n",
                revoked
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| revoked.to_string())
            ));
        }
        if let Some(user_id) = self.created_by_user_id {
            out.push_str(&format!("Created By:     {}\n", user_id));
        }
        out
    }
}

impl HumanOutput for SystemEnrollmentTokenCreatedResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:             {}\n", self.id));
        out.push_str(&format!("Name:           {}\n", self.name));
        out.push_str(&format!("Token:          {}\n", self.token.expose_secret()));
        match self.max_uses {
            Some(max) => out.push_str(&format!("Max Uses:       {}\n", max)),
            None => out.push_str("Max Uses:       unlimited\n"),
        }
        if let Some(expires) = self.expires_at {
            out.push_str(&format!(
                "Expires:        {}\n",
                expires
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| expires.to_string())
            ));
        } else {
            out.push_str("Expires:        never\n");
        }
        out.push_str(&format!(
            "Created:        {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out.push_str("\nIMPORTANT: Save the token value above. It cannot be retrieved later.\n");
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for listing system enrollment tokens.
pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Parameters for creating a system enrollment token.
pub struct CreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub name: &'a str,
    pub max_uses: Option<u32>,
    pub expires_in_seconds: Option<u64>,
}

/// Parameters for showing a system enrollment token.
pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

/// Parameters for revoking a system enrollment token.
pub struct RevokeParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List system enrollment tokens with pagination.
pub async fn list(
    params: ListParams<'_>,
) -> Result<PaginatedResponse<SystemEnrollmentTokenResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let query = ListSystemEnrollmentTokensQuery {
        page: params.page,
        per_page: params.per_page,
    };
    client
        .list_system_enrollment_tokens(&query)
        .await
        .context_to()
}

/// Create a new system enrollment token.
pub async fn create(
    params: CreateParams<'_>,
) -> Result<SystemEnrollmentTokenCreatedResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateSystemEnrollmentTokenRequest {
        name: params.name.to_string(),
        max_uses: params.max_uses,
        expires_in_seconds: params.expires_in_seconds,
    };
    client
        .create_system_enrollment_token(&req)
        .await
        .context_to()
}

/// Show a single system enrollment token by ID.
pub async fn show(params: ShowParams<'_>) -> Result<SystemEnrollmentTokenResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .get_system_enrollment_token(params.id)
        .await
        .context_to()
}

/// Revoke a system enrollment token by ID.
pub async fn revoke(params: RevokeParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .revoke_system_enrollment_token(params.id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: "System enrollment token revoked.".to_string(),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uptrakit_openapi_client::Uuid;
    use uptrakit_shared_types::SecretString;

    fn sample_token_response() -> SystemEnrollmentTokenResponse {
        SystemEnrollmentTokenResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "MQTT Bridge Token".to_string(),
            max_uses: Some(10),
            current_uses: 3,
            expires_at: Some(datetime!(2026-12-31 23:59:59 UTC)),
            created_at: datetime!(2026-01-01 0:00:00 UTC),
            revoked_at: None,
            created_by_user_id: None,
        }
    }

    fn sample_created_response() -> SystemEnrollmentTokenCreatedResponse {
        SystemEnrollmentTokenCreatedResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            token: SecretString::new("upt_sys_abc123def456".to_string()),
            name: "MQTT Bridge Token".to_string(),
            max_uses: Some(10),
            current_uses: 0,
            expires_at: Some(datetime!(2026-12-31 23:59:59 UTC)),
            created_at: datetime!(2026-01-01 0:00:00 UTC),
            created_by_user_id: None,
        }
    }

    #[test]
    fn paginated_empty() {
        let resp = PaginatedResponse::<SystemEnrollmentTokenResponse> {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No system enrollment tokens"));
    }

    #[test]
    fn paginated_has_header_and_row() {
        let resp = PaginatedResponse {
            items: vec![sample_token_response()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("NAME"), "header missing");
        assert!(s.contains("MQTT Bridge Token"), "token name missing");
        assert!(s.contains("3/10"), "usage missing");
        assert!(s.contains("active"), "status missing");
    }

    #[test]
    fn paginated_unlimited_uses() {
        let mut tok = sample_token_response();
        tok.max_uses = None;
        let resp = PaginatedResponse {
            items: vec![tok],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        assert!(resp.to_human_string().contains("∞"));
    }

    #[test]
    fn paginated_revoked_shows_revoked() {
        let mut tok = sample_token_response();
        tok.revoked_at = Some(datetime!(2026-06-01 0:00:00 UTC));
        let resp = PaginatedResponse {
            items: vec![tok],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        assert!(resp.to_human_string().contains("revoked"));
    }

    #[test]
    fn single_token_human_output() {
        let tok = sample_token_response();
        let s = tok.to_human_string();
        assert!(s.contains("MQTT Bridge Token"), "name missing");
        assert!(s.contains("3/10"), "usage missing");
        assert!(!s.contains("never"), "should have expiry");
    }

    #[test]
    fn single_token_unlimited_never_expires() {
        let mut tok = sample_token_response();
        tok.max_uses = None;
        tok.expires_at = None;
        let s = tok.to_human_string();
        assert!(s.contains("unlimited"), "unlimited missing");
        assert!(s.contains("never"), "never expires missing");
    }

    #[test]
    fn created_response_shows_token() {
        let resp = sample_created_response();
        let s = resp.to_human_string();
        assert!(s.contains("upt_sys_abc123def456"), "token value missing");
        assert!(s.contains("IMPORTANT"), "warning missing");
        assert!(s.contains("MQTT Bridge Token"), "name missing");
    }

    #[test]
    fn created_response_unlimited_never_expires() {
        let mut resp = sample_created_response();
        resp.max_uses = None;
        resp.expires_at = None;
        let s = resp.to_human_string();
        assert!(s.contains("unlimited"), "unlimited missing");
        assert!(s.contains("never"), "never missing");
    }
}
