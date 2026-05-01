# Proxmox Task Log Streaming

## Goal

Stream Proxmox task log output (snapshot and backup) to the update terminal in real time during
pre-update protection, so operators see meaningful progress instead of a static status message for
the duration of a long-running backup.

## Scope

### In scope

- Live streaming of Proxmox task log lines to `ctx.output_tx` during snapshot and backup waits
- Section header/footer framing (`--- Proxmox snapshot log ---` / `--- end ---`)
- New `PveTaskLogEntry` API type and `task_log` client method
- New `wait_for_task_completion_with_logs` client method (log-aware variant of the existing wait)
- Both `ProtectionMode::Snapshot` and `ProtectionMode::Backup` paths

### Out of scope

- Log filtering or verbosity controls
- Per-line prefixes (`[PVE]`)
- UPID URL encoding (consistent with existing `task_status` behaviour)
- Changes to non-Proxmox protection paths
- Frontend changes

## Current Codebase Baseline

`update_protection.rs` already sends simple status strings to `ctx.output_tx`
(`Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>`) at three points per operation: start,
success, and failure. The actual wait is delegated to
`ProxmoxClient::wait_for_task_completion(node, upid, timeout)`, which polls
`GET /nodes/{node}/tasks/{upid}/status` every 2 s but never fetches log lines.

The Proxmox API exposes `GET /nodes/{node}/tasks/{upid}/log?start={n}&limit={m}` returning
`{"data": [{"n": 0, "t": "..."}, ...], ...}` — a paginated, 0-based-indexed list of log lines.

## Design

### New API type — `api_types.rs`

```rust
/// A single log line from `GET /nodes/{node}/tasks/{upid}/log`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveTaskLogEntry {
    /// 0-based line number; monotonically increasing across pages.
    pub n: u64,
    /// Log line text (no trailing newline).
    pub t: String,
}
```

Deserialized via `PveResponse<Vec<PveTaskLogEntry>>`. The `total` field Proxmox includes
alongside `data` is ignored by serde; no changes to `PveResponse` needed.

### New `ProxmoxClient` methods — `client.rs`

#### `task_log`

```rust
pub async fn task_log(
    &self,
    node: &str,
    upid: &str,
    start: u64,
) -> Result<Vec<PveTaskLogEntry>> {
    self.get::<Vec<PveTaskLogEntry>>(
        &format!("/nodes/{node}/tasks/{upid}/log?start={start}&limit=500"),
    )
    .await
}
```

Page size 500 is fixed and internal; no caller-visible `limit` parameter. At 2 s poll intervals,
even a very chatty backup task will not produce 500 lines per cycle.

#### `wait_for_task_completion_with_logs`

```rust
pub async fn wait_for_task_completion_with_logs(
    &self,
    node: &str,
    upid: &str,
    timeout: Duration,
    output_tx: &tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) -> Result<PveTaskStatus> {
    let deadline = Instant::now() + timeout;
    let mut next_n: u64 = 0;

    loop {
        // Check status first; log is fetched only on running iterations.
        // This avoids a redundant log HTTP call on the final (stopped) iteration.
        let mut status = self.task_status(node, upid).await?;
        if status.status.eq_ignore_ascii_case("stopped") {
            // Re-poll once if exitstatus is absent. Proxmox sets exitstatus atomically
            // with the stopped transition, but a brief finalization lag can occur on
            // busy nodes. One short retry is sufficient.
            if status.exitstatus.is_none() {
                tokio::time::sleep(Duration::from_millis(500)).await;
                status = self.task_status(node, upid).await?;
            }

            // Single drain: fetch all lines since the last poll.
            match self.task_log(node, upid, next_n).await {
                Ok(entries) => {
                    for entry in &entries {
                        let _ = output_tx.send(format!("{}\n", entry.t).into_bytes());
                    }
                }
                Err(e) => {
                    tracing::debug!(node, upid, error = %e, "final task log drain failed; skipping");
                }
            }
            if status.exitstatus.as_deref() == Some("OK") {
                tracing::debug!(node, upid, "Proxmox task completed successfully");
                return Ok(status);
            }
            let exit = status.exitstatus.as_deref().unwrap_or("unknown");
            bail!(ProxmoxError::Plugin(format!(
                "Proxmox task failed with exit status: {exit}"
            )));
        }

        // Task still running — fetch new log lines and advance cursor.
        // `n` is a 0-based sequential index guaranteed by the Proxmox API;
        // `last.n + 1` is the correct next page start. If no lines are returned
        // (task started but has not written output yet), `next_n` is unchanged
        // and the next poll re-fetches from the same offset.
        match self.task_log(node, upid, next_n).await {
            Ok(entries) => {
                for entry in &entries {
                    let _ = output_tx.send(format!("{}\n", entry.t).into_bytes());
                }
                if let Some(last) = entries.last() {
                    next_n = last.n + 1;
                }
            }
            Err(e) => {
                tracing::debug!(node, upid, error = %e, "task log fetch failed; skipping");
            }
        }

        if Instant::now() >= deadline {
            bail!(ProxmoxError::Plugin(
                "Timed out waiting for Proxmox task completion".to_string()
            ));
        }

        tracing::trace!(node, upid, "Proxmox task still running; polling again in 2s");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
```

**Error handling:** `task_log` errors are non-fatal — the `match` error arm logs at `debug` level
and continues. `task_status` errors remain fatal (existing behaviour). The debug log makes
silent log-streaming degradation (e.g. 403, malformed response) diagnosable without polluting
operator-visible output.

**Accepted risks:**

- *`UnboundedSender` backpressure* — `output_tx` is unbounded; a stalled consumer (blocked
  WebSocket write) can accumulate buffered messages during a long backup. This is an inherited
  property of the existing `output_tx` pattern used throughout the protection code and is
  accepted as-is.
- *UPID uniqueness* — the loop assumes a UPID is not reassigned to a different task during the
  polling window. Proxmox UPIDs encode a timestamp and PID and are not reused within a task's
  lifetime. This assumption holds in all practical deployments.
- *Single-page final drain* — the drain on task completion fetches one page (max 500 lines)
  starting at `next_n`. If a burst of completion output exceeds 500 lines in the last polling
  window, trailing lines are silently dropped. This is not a concern for typical snapshot or
  backup workloads; the full log is always available in the Proxmox UI.

`wait_for_task_completion` (the existing no-output variant) is untouched. All non-protection
callers (if any) are unaffected.

### Changes to `update_protection.rs`

Both `prepare_snapshot_protection` and `prepare_backup_protection` replace their
`wait_for_task_completion` call with the following pattern:

```rust
let kind = "snapshot"; // or "backup"
let wait_result = if let Some(tx) = ctx.output_tx.as_ref() {
    let _ = tx.send(format!("\n--- Proxmox {kind} log ---\n").into_bytes());
    let result = client
        .wait_for_task_completion_with_logs(node, upid, timeout, tx)
        .await;
    let _ = tx.send(b"--- end ---\n\n".to_vec());
    result
} else {
    client
        .wait_for_task_completion(node, upid, timeout)
        .await
};
```

The footer (`--- end ---\n\n`) fires unconditionally after the wait, whether it succeeded or
failed. The `wait_result` variable then replaces the existing `if let Err(error) =
client.wait_for_task_completion(...).await` check — the error branch body (audit persistence,
`output_tx` failure message, early return) is unchanged:

```rust
if let Err(error) = wait_result {
    // same audit upsert and output_tx failure send as before
    return Ok(snapshot_decision_failure()); // or backup equivalent
}
```

**Local variables:** `node` and `upid` are already in scope at the call sites as
`&mapping.proxmox_node` and `&task` respectively. `timeout` comes from
`snapshot_wait_timeout(policy)` / `backup_wait_timeout(policy)` as before.

### Terminal output example — backup success

```text
Starting Proxmox backup for pve1 (VMID 101) to storage 'local'…

--- Proxmox backup log ---
INFO: Starting Backup of VM 101 (qemu)
INFO: status = 512.00 MB / 2048.00 MB -- (25%)
INFO: status = 1024.00 MB / 2048.00 MB -- (50%)
INFO: archive file size: 1.87 GB
INFO: Finished Backup of VM 101 (0:42)
--- end ---

Proxmox backup completed successfully.
```

### Terminal output example — snapshot failure

```text
Creating Proxmox snapshot for pve1 (VMID 101)…

--- Proxmox snapshot log ---
INFO: task started by user 'root@pam!uptrakit'
ERROR: snapshot failed: VM is locked
--- end ---

Proxmox snapshot task failed: Proxmox task failed with exit status: ERROR
```

## Implementation Sequence

1. Add `PveTaskLogEntry` to `api_types.rs` (with deserialise test).
2. Add `task_log` to `ProxmoxClient` in `client.rs`.
3. Add `wait_for_task_completion_with_logs` to `ProxmoxClient` in `client.rs`.
4. Update `prepare_snapshot_protection` in `update_protection.rs`.
5. Update `prepare_backup_protection` in `update_protection.rs`.

## Tests

- **`PveTaskLogEntry` deserialisation** — unit test in `api_types.rs` verifying `n` and `t`
  fields parse correctly from a sample JSON response.
- **`task_log` URL construction** — not directly unit-testable without HTTP mocking; covered by
  integration testing against a real or stubbed Proxmox API.
- **`wait_for_task_completion_with_logs`** — covered by integration tests (requires a live task
  UPID against a real or stubbed Proxmox API). Unit testing is not practical without HTTP
  mocking infrastructure.
- Existing `update_protection.rs` tests are unaffected; all mock-database tests construct
  `ControllerProtectionContext` without `output_tx` (`output_tx = None`), so they exercise the
  `else` branch unchanged.
