# Proxmox Task Log Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream Proxmox snapshot/backup task log lines to the update terminal in real time
during pre-update protection, replacing the current static status messages with live Proxmox
output.

**Architecture:** Add `PveTaskLogEntry` to `api_types.rs`, add `task_log` +
`wait_for_task_completion_with_logs` to `ProxmoxClient` in `client.rs`, then update both
`prepare_snapshot_protection` and `prepare_backup_protection` in `update_protection.rs` to use
the new method with header/footer framing when `output_tx` is present. The existing
`wait_for_task_completion` is left untouched.

**Tech Stack:** Rust, tokio async, `reqwest`, `tracing`, `rootcause` (`bail!`), `serde::Deserialize`.

---

## File Map

| File                                                             | Change                                                                                                               |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `crates/plugins/infrastructure/proxmox/src/api_types.rs`         | Add `PveTaskLogEntry` struct + deserialisation test                                                                  |
| `crates/plugins/infrastructure/proxmox/src/client.rs`            | Add `task_log` method after `task_status`; add `wait_for_task_completion_with_logs` after `wait_for_task_completion` |
| `crates/plugins/infrastructure/proxmox/src/update_protection.rs` | Update snapshot wait block (lines 412–447) and backup wait block (lines 619–654)                                     |

---

## Task 1: Add `PveTaskLogEntry` to `api_types.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/api_types.rs:134` (insert after `PveTaskStatus`)

- [ ] **Step 1: Write the failing test**

Open `crates/plugins/infrastructure/proxmox/src/api_types.rs`. In the `#[cfg(test)]`
`mod tests` block (after the existing `deserialize_task_status` test, around line 194), add:

```rust
#[test]
fn deserialize_task_log() {
    let json = r#"{"data":[{"n":0,"t":"INFO: starting"},{"n":1,"t":"INFO: done"}]}"#;
    let resp: PveResponse<Vec<PveTaskLogEntry>> =
        serde_json::from_str(json).expect("deserialize");
    assert_eq!(resp.data.len(), 2);
    assert_eq!(resp.data[0].n, 0);
    assert_eq!(resp.data[0].t, "INFO: starting");
    assert_eq!(resp.data[1].n, 1);
}
```

- [ ] **Step 2: Run the test — expect compile failure**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox deserialize_task_log 2>&1 | head -20
```

Expected: compile error — `PveTaskLogEntry` not found.

- [ ] **Step 3: Add `PveTaskLogEntry` struct**

Insert the following immediately after the `PveTaskStatus` struct (after the closing `}` on line 134):

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

- [ ] **Step 4: Run the test — expect pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox deserialize_task_log
```

Expected: `test api_types::tests::deserialize_task_log ... ok`

- [ ] **Step 5: Confirm full crate tests still pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/api_types.rs
git commit -m "feat(proxmox): add PveTaskLogEntry API type"
```

---

## Task 2: Add `task_log` method to `ProxmoxClient`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/client.rs` (insert after `task_status` at line 441)

The `task_log` method calls `self.get::<Vec<PveTaskLogEntry>>(...)`. The `get` method already
handles the `{"data": T}` envelope — `total` from the Proxmox response is silently ignored by
serde.

- [ ] **Step 1: Add `task_log` to `ProxmoxClient`**

In `client.rs`, locate `pub async fn task_status` (line 438). Immediately after its closing `}` (line 441), insert:

```rust
/// Fetch a page of log lines for a Proxmox task.
///
/// `start` is the 0-based index of the first line to return. Page size is fixed at 500 —
/// sufficient for any single 2-second poll interval. Returns an empty `Vec` when no new
/// lines have been written since `start`.
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

- [ ] **Step 2: Confirm `PveTaskLogEntry` is in scope**

`api_types.rs` re-exports everything via `use crate::api_types::*;` at the top of `client.rs`. Verify:

```bash
grep "use crate::api_types" crates/plugins/infrastructure/proxmox/src/client.rs
```

Expected: `use crate::api_types::*;`

- [ ] **Step 3: Check compile**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: no errors.

- [ ] **Step 4: Run full crate tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/client.rs
git commit -m "feat(proxmox): add task_log client method"
```

---

## Task 3: Add `wait_for_task_completion_with_logs` to `ProxmoxClient`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/client.rs` (insert after `wait_for_task_completion`)

This method is the log-streaming variant of the existing `wait_for_task_completion`. It polls
status first (to avoid a redundant log fetch on the last iteration), then fetches new log lines
on each running iteration. On stop detection it does a single drain, then re-polls once if
`exitstatus` is absent (Proxmox finalization lag).

- [ ] **Step 1: Add the method**

In `client.rs`, locate the closing `}` of `wait_for_task_completion` (line 488). Immediately after it, insert:

```rust
/// Poll a Proxmox task until completion, streaming log lines to `output_tx` as they appear.
///
/// Polls status first each iteration (avoids redundant log call on the final stopped
/// iteration). Log fetch errors are non-fatal and logged at `debug` level. `task_status`
/// errors remain fatal.
pub async fn wait_for_task_completion_with_logs(
    &self,
    node: &str,
    upid: &str,
    timeout: Duration,
    output_tx: &tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) -> Result<PveTaskStatus> {
    tracing::debug!(
        node,
        upid,
        timeout_secs = timeout.as_secs(),
        "waiting for Proxmox task completion with log streaming"
    );
    let deadline = Instant::now() + timeout;
    let mut next_n: u64 = 0;

    loop {
        // Check status first; log is fetched only on running iterations.
        // This avoids a redundant log HTTP call on the final (stopped) iteration.
        let status = self.task_status(node, upid).await?;
        // Re-poll once if exitstatus is absent. Proxmox sets exitstatus atomically
        // with the stopped transition, but a brief finalization lag can occur on
        // busy nodes. One short retry is sufficient.
        // NOTE: the spec uses `let mut status` with reassignment, but this shadow
        // form is used here to satisfy Clippy's `unused_mut` / `needless_late_init`
        // lints. Do NOT revert to the spec's `let mut` form.
        let status = if status.status.eq_ignore_ascii_case("stopped")
            && status.exitstatus.is_none()
        {
            tokio::time::sleep(Duration::from_millis(500)).await;
            self.task_status(node, upid).await?
        } else {
            status
        };
        if status.status.eq_ignore_ascii_case("stopped") {
            // Single drain: fetch all lines since the last poll.
            match self.task_log(node, upid, next_n).await {
                Ok(entries) => {
                    for entry in &entries {
                        let _ = output_tx.send(format!("{}\n", entry.t).into_bytes());
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        node,
                        upid,
                        error = %e,
                        "final task log drain failed; skipping"
                    );
                }
            }

            if status.exitstatus.as_deref() == Some("OK") {
                tracing::debug!(node, upid, "Proxmox task completed successfully");
                return Ok(status);
            }
            let exit = status.exitstatus.as_deref().unwrap_or("unknown");
            bail!(ProxmoxError::Plugin(format!(
                "Proxmox task {upid} on {node} failed with exit status: {exit}"
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
            bail!(ProxmoxError::Plugin(format!(
                "Timed out waiting for Proxmox task {upid} on {node} to complete"
            )));
        }

        tracing::trace!(node, upid, "Proxmox task still running; polling again in 2s");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
```

- [ ] **Step 2: Check compile**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: no errors.

- [ ] **Step 3: Run full crate tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/client.rs
git commit -m "feat(proxmox): add wait_for_task_completion_with_logs"
```

---

## Task 4: Update `prepare_snapshot_protection` in `update_protection.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/update_protection.rs:410–447`

Replace the existing `if let Err(error) = client.wait_for_task_completion(...)` block
(lines 412–447) with the log-streaming pattern. The error branch body (audit upsert, output
send, early return) is identical to before. Note: `--- end ---` fires unconditionally after
the wait regardless of success or failure — the failure status message follows it. This is
intentional per the spec; the operator sees the log section close before the error line.

- [ ] **Step 1: Replace the snapshot wait block**

Locate this block in `update_protection.rs` (around lines 410–447):

```rust
tracing::debug!(node = %mapping.proxmox_node, vmid = mapping.proxmox_vmid, upid = %task, "snapshot task started — waiting for completion");

if let Err(error) = client
    .wait_for_task_completion(&mapping.proxmox_node, &task, snapshot_wait_timeout(policy))
    .await
{
    tracing::warn!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        snapshot_name = %snapshot_name,
        upid = %task,
        error = %error,
        "Proxmox snapshot task did not complete successfully"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(format!("Proxmox snapshot task failed: {error}\n").into_bytes());
    }
    let audit = ProtectionAudit {
        update_history_id: ctx.update_history_id,
        tenant_id: ctx.tenant_id,
        host_id: ctx.host_id,
        software_item_id: ctx.software_item_id,
        plugin_config_id: mapping.plugin_config_id,
        mapping_id: Some(mapping.id),
        mode: ProtectionMode::Snapshot,
        status: "failed".to_string(),
        artifact_kind: Some("snapshot".to_string()),
        artifact_ref: Some(snapshot_name),
        backup_target_key: None,
        detail: Some(SUMMARY_FAILURE.to_string()),
        error_message: Some(error.to_string()),
    };
    store
        .upsert_audit(&to_audit_record(&audit))
        .await
        .map_err(plugin_internal)?;
    return Ok(snapshot_decision_failure());
}
```

Replace with:

```rust
tracing::debug!(node = %mapping.proxmox_node, vmid = mapping.proxmox_vmid, upid = %task, "snapshot task started — waiting for completion");

let wait_result = if let Some(tx) = ctx.output_tx.as_ref() {
    let _ = tx.send(b"\n--- Proxmox snapshot log ---\n".to_vec());
    let result = client
        .wait_for_task_completion_with_logs(
            &mapping.proxmox_node,
            &task,
            snapshot_wait_timeout(policy),
            tx,
        )
        .await;
    let _ = tx.send(b"--- end ---\n\n".to_vec());
    result
} else {
    client
        .wait_for_task_completion(&mapping.proxmox_node, &task, snapshot_wait_timeout(policy))
        .await
};

if let Err(error) = wait_result {
    tracing::warn!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        snapshot_name = %snapshot_name,
        upid = %task,
        error = %error,
        "Proxmox snapshot task did not complete successfully"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(format!("Proxmox snapshot task failed: {error}\n").into_bytes());
    }
    let audit = ProtectionAudit {
        update_history_id: ctx.update_history_id,
        tenant_id: ctx.tenant_id,
        host_id: ctx.host_id,
        software_item_id: ctx.software_item_id,
        plugin_config_id: mapping.plugin_config_id,
        mapping_id: Some(mapping.id),
        mode: ProtectionMode::Snapshot,
        status: "failed".to_string(),
        artifact_kind: Some("snapshot".to_string()),
        artifact_ref: Some(snapshot_name),
        backup_target_key: None,
        detail: Some(SUMMARY_FAILURE.to_string()),
        error_message: Some(error.to_string()),
    };
    store
        .upsert_audit(&to_audit_record(&audit))
        .await
        .map_err(plugin_internal)?;
    return Ok(snapshot_decision_failure());
}
```

- [ ] **Step 2: Check compile**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: no errors.

- [ ] **Step 3: Run tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all pass. The existing mock-database tests construct `ControllerProtectionContext::new`
without `output_tx`, so they exercise the `else` branch and are unaffected.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/update_protection.rs
git commit -m "feat(proxmox): stream snapshot task log to update terminal"
```

---

## Task 5: Update `prepare_backup_protection` in `update_protection.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/update_protection.rs:617–654`

Replace the existing backup wait block with the log-streaming pattern. Note: in the error
audit `artifact_ref: Some(task)` moves `task` — this is fine because the borrow in
`wait_for_task_completion_with_logs` ends before the move.

- [ ] **Step 1: Replace the backup wait block**

Locate this block (around lines 617–654):

```rust
tracing::debug!(node = %mapping.proxmox_node, vmid = mapping.proxmox_vmid, upid = %task, "backup task started — waiting for completion");

if let Err(error) = client
    .wait_for_task_completion(&mapping.proxmox_node, &task, backup_wait_timeout(policy))
    .await
{
    tracing::warn!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        storage = %target_storage_id,
        upid = %task,
        error = %error,
        "Proxmox backup task did not complete successfully"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(format!("Proxmox backup task failed: {error}\n").into_bytes());
    }
    let audit = ProtectionAudit {
        update_history_id: ctx.update_history_id,
        tenant_id: ctx.tenant_id,
        host_id: ctx.host_id,
        software_item_id: ctx.software_item_id,
        plugin_config_id: mapping.plugin_config_id,
        mapping_id: Some(mapping.id),
        mode: ProtectionMode::Backup,
        status: "failed".to_string(),
        artifact_kind: Some("backup".to_string()),
        artifact_ref: Some(task),
        backup_target_key: Some(target_key.to_string()),
        detail: Some(SUMMARY_FAILURE.to_string()),
        error_message: Some(error.to_string()),
    };
    store
        .upsert_audit(&to_audit_record(&audit))
        .await
        .map_err(plugin_internal)?;
    return Ok(snapshot_decision_failure());
}
```

Replace with:

```rust
tracing::debug!(node = %mapping.proxmox_node, vmid = mapping.proxmox_vmid, upid = %task, "backup task started — waiting for completion");

let wait_result = if let Some(tx) = ctx.output_tx.as_ref() {
    let _ = tx.send(b"\n--- Proxmox backup log ---\n".to_vec());
    let result = client
        .wait_for_task_completion_with_logs(
            &mapping.proxmox_node,
            &task,
            backup_wait_timeout(policy),
            tx,
        )
        .await;
    let _ = tx.send(b"--- end ---\n\n".to_vec());
    result
} else {
    client
        .wait_for_task_completion(&mapping.proxmox_node, &task, backup_wait_timeout(policy))
        .await
};

if let Err(error) = wait_result {
    tracing::warn!(
        node = %mapping.proxmox_node,
        vmid = mapping.proxmox_vmid,
        storage = %target_storage_id,
        upid = %task,
        error = %error,
        "Proxmox backup task did not complete successfully"
    );
    if let Some(tx) = ctx.output_tx.as_ref() {
        let _ = tx.send(format!("Proxmox backup task failed: {error}\n").into_bytes());
    }
    let audit = ProtectionAudit {
        update_history_id: ctx.update_history_id,
        tenant_id: ctx.tenant_id,
        host_id: ctx.host_id,
        software_item_id: ctx.software_item_id,
        plugin_config_id: mapping.plugin_config_id,
        mapping_id: Some(mapping.id),
        mode: ProtectionMode::Backup,
        status: "failed".to_string(),
        artifact_kind: Some("backup".to_string()),
        artifact_ref: Some(task),
        backup_target_key: Some(target_key.to_string()),
        detail: Some(SUMMARY_FAILURE.to_string()),
        error_message: Some(error.to_string()),
    };
    store
        .upsert_audit(&to_audit_record(&audit))
        .await
        .map_err(plugin_internal)?;
    return Ok(snapshot_decision_failure());
}
```

- [ ] **Step 2: Check compile**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: no errors.

- [ ] **Step 3: Run the full quality gate**

```bash
cargo fmt --all && \
cargo check --no-default-features --features db-sqlite && \
cargo check --all-features && \
cargo clippy --all-targets --no-default-features --features db-sqlite && \
cargo clippy --all-targets --all-features && \
cargo test -p uptrakit-plugin-infrastructure-proxmox --all-features
```

Expected: all clean.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/update_protection.rs
git commit -m "feat(proxmox): stream backup task log to update terminal"
```

---

## Verification

After all five tasks, the terminal output during a Proxmox pre-update protection step with an `output_tx` receiver should look like:

**Snapshot:**

```text
Creating Proxmox snapshot for pve1 (VMID 101)…

--- Proxmox snapshot log ---
INFO: task started by user 'root@pam!uptrakit'
INFO: Snapshot created
--- end ---

Proxmox snapshot 'utk-<id>' created successfully.
```

**Backup:**

```text
Starting Proxmox backup for pve1 (VMID 101) to storage 'local'…

--- Proxmox backup log ---
INFO: Starting Backup of VM 101 (qemu)
INFO: status = 512.00 MB / 2048.00 MB -- (25%)
INFO: archive file size: 1.87 GB
INFO: Finished Backup of VM 101 (0:42)
--- end ---

Proxmox backup completed successfully.
```

When `output_tx` is `None` (non-interactive dispatch, existing tests), no header/footer is sent
and `wait_for_task_completion` is called as before — no behaviour change.
