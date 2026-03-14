use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;

#[derive(Debug, Subcommand)]
pub enum UpdateCommands {
    /// Trigger an update for a software item on a host
    Trigger {
        /// Software item UUID
        item_id: Uuid,
        /// Host UUID
        host_id: Uuid,
        /// Target version to update to
        #[arg(long)]
        to_version: String,
        /// Release tag (defaults to to_version)
        #[arg(long)]
        release_tag: Option<String>,
        /// Release URL
        #[arg(long)]
        release_url: Option<String>,
        /// Follow (tail) update output in real-time after triggering
        #[arg(long, short)]
        follow: bool,
        /// Request interactive (PTY) mode for the update session
        #[arg(long, short)]
        interactive: bool,
    },
    /// Trigger a batch update for all outdated items on a host
    BatchHost {
        /// Host UUID
        host_id: Uuid,
        /// Only update items in this category (e.g. security)
        #[arg(long)]
        category: Option<String>,
        /// Exclude these software item UUIDs from the batch
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<Uuid>,
        /// Follow batch progress in real-time after triggering
        #[arg(long, short)]
        follow: bool,
    },
    /// Trigger a batch update to roll out a software item to hosts
    BatchItem {
        /// Software item UUID
        item_id: Uuid,
        /// Target version to update to
        #[arg(long)]
        to_version: String,
        /// Limit to these host UUIDs (default: all assigned hosts)
        #[arg(long, value_delimiter = ',')]
        host: Vec<Uuid>,
        /// Follow batch progress in real-time after triggering
        #[arg(long, short)]
        follow: bool,
    },
}

pub async fn dispatch(command: UpdateCommands, ctx: &CliContext) -> Result<()> {
    match command {
        UpdateCommands::Trigger {
            item_id,
            host_id,
            to_version,
            release_tag,
            release_url,
            follow,
            interactive,
        } => {
            let resp = trigger(TriggerParams {
                item_id: &item_id,
                host_id: &host_id,
                to_version: &to_version,
                release_tag: release_tag.as_deref(),
                release_url: release_url.as_deref(),
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                interactive,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;

            if follow {
                let tail_result = super::tail::tail(super::tail::TailParams {
                    update_history_id: &resp.update_history_id,
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                })
                .await?;
                std::process::exit(tail_result.exit_code());
            }
        }
        UpdateCommands::BatchHost {
            host_id,
            category,
            exclude,
            follow,
        } => {
            let resp =
                super::batch_update::trigger_host_batch(super::batch_update::HostBatchParams {
                    host_id: &host_id,
                    category: category.as_deref(),
                    exclude: &exclude,
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                    request_timeout: ctx.request_timeout,
                })
                .await?;
            crate::output::print_output(ctx.format, &resp)?;

            if follow && let Some(batch_id) = resp.batch_id {
                let result =
                    super::batch_update::follow_batch(super::batch_update::FollowBatchParams {
                        batch_id: &batch_id,
                        server: ctx.server.as_deref(),
                        token: ctx.token.as_deref(),
                        insecure: ctx.insecure,
                    })
                    .await?;
                std::process::exit(result.exit_code());
            }
        }
        UpdateCommands::BatchItem {
            item_id,
            to_version,
            host,
            follow,
        } => {
            let resp =
                super::batch_update::trigger_item_batch(super::batch_update::ItemBatchParams {
                    item_id: &item_id,
                    to_version: &to_version,
                    host_ids: &host,
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                    request_timeout: ctx.request_timeout,
                })
                .await?;
            crate::output::print_output(ctx.format, &resp)?;

            if follow && let Some(batch_id) = resp.batch_id {
                let result =
                    super::batch_update::follow_batch(super::batch_update::FollowBatchParams {
                        batch_id: &batch_id,
                        server: ctx.server.as_deref(),
                        token: ctx.token.as_deref(),
                        insecure: ctx.insecure,
                    })
                    .await?;
                std::process::exit(result.exit_code());
            }
        }
    }
    Ok(())
}
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
    pub interactive: bool,
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
        interactive: params.interactive,
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
