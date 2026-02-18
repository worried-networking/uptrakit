use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::software_items::{ReleaseInfoRequest, TriggerUpdateRequest};

/// Parameters for triggering an update.
pub struct TriggerParams<'a> {
    pub item_id: &'a Uuid,
    pub host_id: &'a Uuid,
    pub to_version: &'a str,
    pub release_tag: Option<&'a str>,
    pub release_url: Option<&'a str>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
}

/// Trigger an update for a software item on a specific host.
pub async fn trigger(params: TriggerParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure)?;

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

    let resp = client
        .trigger_update(params.item_id, params.host_id, &req)
        .await
        .context_to()?;

    let human = format!(
        "Update triggered.\n  History ID: {}\n  Status:     {}\n",
        resp.update_history_id, resp.status
    );

    print_output(params.format, &human, &resp)
}
