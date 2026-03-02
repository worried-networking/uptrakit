use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use futures_util::StreamExt;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::batch_progress_stream::BatchProgressEvent;
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::update_batches::{
    BatchUpdateResponse, HostBatchUpdateRequest, ItemBatchUpdateRequest,
    UpdateBatchDetailResponse, UpdateBatchListQuery, UpdateBatchSummaryResponse,
};

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for BatchUpdateResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        match self.batch_id {
            Some(id) => {
                out.push_str(&format!(
                    "Batch update created.\n  Batch ID:      {id}\n  Total created: {}\n",
                    self.total_created
                ));
            }
            None => {
                out.push_str("No eligible items found for batch update.\n");
            }
        }
        if !self.skipped.is_empty() {
            out.push_str(&format!("  Skipped:       {}\n", self.skipped.len()));
            for s in &self.skipped {
                out.push_str(&format!(
                    "    - {} on {}: {}\n",
                    s.software_item_name, s.host_name, s.reason
                ));
            }
        }
        out
    }
}

impl HumanOutput for PaginatedResponse<UpdateBatchSummaryResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No update batches found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<14} {:<14} {:<6} {:<6} {:<6} {}\n",
            "ID", "TYPE", "STATUS", "DONE", "FAIL", "PEND", "CREATED"
        );
        for batch in &self.items {
            let created = batch
                .created_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| batch.created_at.to_string());
            out.push_str(&format!(
                "{:<38} {:<14} {:<14} {:<6} {:<6} {:<6} {}\n",
                batch.id,
                batch.batch_type,
                batch.status,
                batch.completed_count,
                batch.failed_count,
                batch.pending_count,
                created,
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for UpdateBatchDetailResponse {
    fn to_human_string(&self) -> String {
        let created = self
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| self.created_at.to_string());
        let completed = self
            .completed_at
            .map(|t| {
                t.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| t.to_string())
            })
            .unwrap_or_else(|| "-".to_string());

        let mut out = format!(
            "Batch ID:    {}\n\
             Type:        {}\n\
             Status:      {}\n\
             Total:       {}\n\
             Completed:   {}\n\
             Failed:      {}\n\
             Pending:     {}\n\
             Actor:       {} ({})\n\
             Created:     {}\n\
             Completed:   {}\n",
            self.id,
            self.batch_type,
            self.status,
            self.total_count,
            self.completed_count,
            self.failed_count,
            self.pending_count,
            self.actor_type,
            self.actor_id,
            created,
            completed,
        );

        if !self.updates.is_empty() {
            out.push_str(&format!(
                "\n{:<38} {:<20} {:<20} {:<12} {}\n",
                "UPDATE ID", "HOST", "SOFTWARE", "STATUS", "VERSION"
            ));
            for item in &self.updates {
                out.push_str(&format!(
                    "{:<38} {:<20} {:<20} {:<12} {}\n",
                    item.update_history_id,
                    truncate(&item.host_name, 18),
                    truncate(&item.software_item_name, 18),
                    item.status,
                    item.to_version,
                ));
            }
        }
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

pub struct HostBatchParams<'a> {
    pub host_id: &'a Uuid,
    pub category: Option<&'a str>,
    pub exclude: &'a [Uuid],
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ItemBatchParams<'a> {
    pub item_id: &'a Uuid,
    pub to_version: &'a str,
    pub host_ids: &'a [Uuid],
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ListBatchParams<'a> {
    pub status: Option<&'a str>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ShowBatchParams<'a> {
    pub batch_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct FollowBatchParams<'a> {
    pub batch_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
}

/// Result of a batch follow operation.
pub struct FollowResult {
    /// Final batch status.
    pub status: String,
}

impl FollowResult {
    /// Map the final batch status to a CLI exit code.
    ///
    /// - `completed` → 0
    /// - `partially_completed` → 1
    /// - anything else → 2
    pub fn exit_code(&self) -> i32 {
        match self.status.as_str() {
            "completed" => 0,
            "partially_completed" => 1,
            _ => 2,
        }
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Trigger a host-wide batch update.
pub async fn trigger_host_batch(params: HostBatchParams<'_>) -> Result<BatchUpdateResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let exclude = if params.exclude.is_empty() {
        None
    } else {
        Some(params.exclude.to_vec())
    };

    let req = HostBatchUpdateRequest {
        category_filter: params.category.map(|s| s.to_string()),
        exclude_item_ids: exclude,
    };

    client
        .trigger_host_batch_update(params.host_id, &req)
        .await
        .context_to()
}

/// Trigger an item-wide batch update.
pub async fn trigger_item_batch(params: ItemBatchParams<'_>) -> Result<BatchUpdateResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let host_ids = if params.host_ids.is_empty() {
        None
    } else {
        Some(params.host_ids.to_vec())
    };

    let req = ItemBatchUpdateRequest {
        to_version: params.to_version.to_string(),
        host_ids,
    };

    client
        .trigger_item_batch_update(params.item_id, &req)
        .await
        .context_to()
}

/// List update batches.
pub async fn list_batches(
    params: ListBatchParams<'_>,
) -> Result<PaginatedResponse<UpdateBatchSummaryResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    let query = UpdateBatchListQuery {
        status: params.status.map(str::to_string),
        page: params.page,
        per_page: params.per_page,
    };

    client.list_update_batches(&query).await.context_to()
}

/// Show a single update batch with details.
pub async fn show_batch(params: ShowBatchParams<'_>) -> Result<UpdateBatchDetailResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;

    client.get_update_batch(params.batch_id).await.context_to()
}

/// Follow batch progress via SSE stream.
///
/// Prints progress updates to stderr and returns when the batch completes.
/// Ctrl+C detaches without affecting the batch.
pub async fn follow_batch(params: FollowBatchParams<'_>) -> Result<FollowResult> {
    // SSE connections are long-lived — no request timeout.
    let client = authenticated_client(params.server, params.token, params.insecure, None)?;

    eprintln!("Following batch progress for {} ...", params.batch_id);

    let stream = client
        .stream_batch_progress(params.batch_id)
        .await
        .context_to()?;
    tokio::pin!(stream);

    let result = loop {
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nDetached (batch continues in the background).");
                break FollowResult {
                    status: "detached".to_string(),
                };
            }
            event = stream.next() => {
                match event {
                    Some(Ok(BatchProgressEvent::Update(update))) => {
                        let status_label = match update.event.as_str() {
                            "update_dispatched" => "DISPATCHED",
                            "update_started" => "STARTED",
                            "update_completed" => "COMPLETED",
                            "update_failed" => "FAILED",
                            other => other,
                        };
                        eprint!(
                            "  [{status_label}] {} on {}",
                            update.software_item_name, update.host_name,
                        );
                        if let Some(ref err) = update.error {
                            eprint!(" — {err}");
                        }
                        eprintln!();
                    }
                    Some(Ok(BatchProgressEvent::Progress(progress))) => {
                        eprintln!(
                            "  Progress: {}/{} completed, {} failed, {} pending",
                            progress.completed, progress.total,
                            progress.failed, progress.pending,
                        );
                    }
                    Some(Ok(BatchProgressEvent::BatchCompleted(completed))) => {
                        eprintln!("Batch {}", completed.status);
                        break FollowResult {
                            status: completed.status,
                        };
                    }
                    Some(Err(e)) => {
                        eprintln!("Stream error: {e}");
                        break FollowResult {
                            status: "error".to_string(),
                        };
                    }
                    None => {
                        eprintln!("Stream ended without completion event.");
                        break FollowResult {
                            status: "disconnected".to_string(),
                        };
                    }
                }
            }
        }
    };

    Ok(result)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_response_human_output_with_batch() {
        let resp = BatchUpdateResponse {
            batch_id: Some("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6".parse().unwrap()),
            total_created: 3,
            updates: vec![],
            skipped: vec![],
        };
        let s = resp.to_human_string();
        assert!(s.contains("Batch update created"));
        assert!(s.contains("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"));
        assert!(s.contains("Total created: 3"));
    }

    #[test]
    fn batch_response_human_output_no_eligible() {
        let resp = BatchUpdateResponse {
            batch_id: None,
            total_created: 0,
            updates: vec![],
            skipped: vec![],
        };
        let s = resp.to_human_string();
        assert!(s.contains("No eligible items"));
    }

    #[test]
    fn follow_result_exit_codes() {
        assert_eq!(
            FollowResult {
                status: "completed".to_string()
            }
            .exit_code(),
            0
        );
        assert_eq!(
            FollowResult {
                status: "partially_completed".to_string()
            }
            .exit_code(),
            1
        );
        assert_eq!(
            FollowResult {
                status: "detached".to_string()
            }
            .exit_code(),
            2
        );
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("abc", 5), "abc");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("abcdefghij", 5);
        assert_eq!(result, "abcd…");
    }

    #[test]
    fn list_batches_human_output_empty() {
        let resp: PaginatedResponse<UpdateBatchSummaryResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        let s = resp.to_human_string();
        assert!(s.contains("No update batches found"));
    }
}
