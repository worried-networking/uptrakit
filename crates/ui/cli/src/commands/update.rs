use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use uptrakit_web_api_types::software_items::TriggerUpdateResponse;

/// Parameters for triggering an update.
pub struct TriggerParams<'a> {
    pub item_id: &'a str,
    pub host_id: &'a str,
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

    let path = format!(
        "/api/v1/software-items/{}/hosts/{}/update",
        params.item_id, params.host_id
    );

    let mut body = serde_json::json!({
        "to_version": params.to_version,
    });

    if params.release_tag.is_some() || params.release_url.is_some() {
        let release_info = serde_json::json!({
            "tag": params.release_tag.unwrap_or(params.to_version),
            "release_url": params.release_url.unwrap_or(""),
        });
        body["release_info"] = release_info;
    }

    let resp: TriggerUpdateResponse = client.post_json(&path, body).await?;

    let human = format!(
        "Update triggered.\n  History ID: {}\n  Status:     {:?}\n",
        resp.update_history_id, resp.status
    );

    print_output(params.format, &human, &resp)
}
