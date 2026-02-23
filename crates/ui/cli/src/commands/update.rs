use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::software_items::{
    ReleaseInfoRequest, TriggerUpdateRequest, TriggerUpdateResponse,
};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for TriggerUpdateResponse {
    fn to_human_string(&self) -> String {
        format!(
            "Update triggered.\n  History ID: {}\n  Status:     {}\n",
            self.update_history_id, self.status
        )
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for triggering an update.
pub struct TriggerParams<'a> {
    pub item_id: &'a Uuid,
    pub host_id: &'a Uuid,
    pub to_version: &'a str,
    pub release_tag: Option<&'a str>,
    pub release_url: Option<&'a str>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Trigger an update for a software item on a specific host.
pub async fn trigger(params: TriggerParams<'_>) -> Result<TriggerUpdateResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let release_info = if params.release_tag.is_some() || params.release_url.is_some() {
        Some(ReleaseInfoRequest {
            tag: params.release_tag.unwrap_or(params.to_version).to_string(),
            release_url: params.release_url.unwrap_or("").to_string(),
            assets: vec![],
        })
    } else {
        None
    };

    let req = TriggerUpdateRequest {
        to_version: params.to_version.to_string(),
        release_info,
    };

    client
        .trigger_update(params.item_id, params.host_id, &req)
        .await
        .context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_openapi_client::types::software_items::TriggerUpdateStatus;

    #[test]
    fn trigger_update_human_output() {
        let resp = TriggerUpdateResponse {
            update_history_id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            status: TriggerUpdateStatus::Pending,
        };
        let s = resp.to_human_string();
        assert!(s.contains("Update triggered"), "message missing");
        assert!(
            s.contains("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"),
            "history id missing"
        );
        assert!(s.contains("pending"), "status missing");
    }
}
