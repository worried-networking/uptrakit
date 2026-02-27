mod cli;
mod client;
mod commands;
pub(crate) mod db;
mod error;
mod host_info;
mod host_ops;
mod ssh_config;
mod ssh_executor;
mod ssh_key;
mod ssh_pool;
mod ssh_target;
mod ssh_transport;

use clap::Parser;
use rootcause::prelude::*;
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};

use cli::{Args, Commands};

/// How often the daemon polls the local `ssh_hosts` table for changes.
const HOST_RELOAD_INTERVAL: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Typed error for initialization helpers
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum InitError {
    #[error("{0}")]
    Directory(String),
    #[error("{0}")]
    MasterKey(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Hex(String),
}

type InitResult<T> = std::result::Result<T, rootcause::Report<InitError>>;

// ---------------------------------------------------------------------------
// Service event enum
// ---------------------------------------------------------------------------

/// Events produced by the SSH agent's service loop.
///
/// Extends the original `client::UpdateEvent` with a host-config-changed
/// trigger so both sources of internal events flow through the same
/// `poll_service_event` / `on_service_event` contract.
enum SshAgentEvent {
    /// Progress from an in-flight update task (output line or completion).
    Update(client::UpdateEvent),
    /// The host-config reload ticker fired; the handler will diff the DB
    /// snapshot and send `ReportHosts` if anything changed.
    HostConfigChanged,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

struct SshAgentHandler {
    state_dir: std::path::PathBuf,
    local_db: Option<sea_orm::DatabaseConnection>,
    in_flight_update: Option<client::InFlightUpdate>,
    pool: ssh_pool::SshConnectionPool,
    /// Periodic ticker for host-config change detection.
    ///
    /// `None` until the first successful `on_connected`; reset on every
    /// reconnect so the first tick fires `HOST_RELOAD_INTERVAL` after connect,
    /// not sooner.
    reload_ticker: Option<tokio::time::Interval>,
    /// Last-known snapshot of `(id, updated_at)` pairs from `ssh_hosts`.
    ///
    /// Used to detect additions, removals, and updates without a full model
    /// load on every tick.  Populated in `on_connected` after
    /// `report_enrolled_hosts` completes.
    host_snapshot: Vec<host_ops::HostSnapshot>,
}

impl SshAgentHandler {
    /// Drive an in-flight update to completion or the next output line.
    ///
    /// Returns `pending()` when no update is in flight, so the `select!` in
    /// `poll_service_event` can safely poll this arm alongside the reload
    /// ticker without a double-borrow of `self`.
    async fn poll_update(in_flight: &mut Option<client::InFlightUpdate>) -> client::UpdateEvent {
        if let Some(update) = in_flight {
            tokio::select! {
                biased;
                Some(output_msg) = update.output_rx.recv() => {
                    client::UpdateEvent::Output(output_msg)
                }
                result = &mut update.handle => {
                    client::UpdateEvent::Completed(result)
                }
            }
        } else {
            std::future::pending::<client::UpdateEvent>().await
        }
    }

    /// Wait for the next reload ticker tick.
    ///
    /// Returns `pending()` when the ticker has not yet been initialized (i.e.
    /// before the first `on_connected`).
    async fn poll_reload_tick(ticker: &mut Option<tokio::time::Interval>) -> tokio::time::Instant {
        if let Some(t) = ticker {
            t.tick().await
        } else {
            std::future::pending::<tokio::time::Instant>().await
        }
    }
}

#[async_trait::async_trait]
impl ServiceHandler for SshAgentHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";

    type ServiceEvent = SshAgentEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        // Open (or create) the local SSH host database.
        let local_db = crate::db::init_db(&self.state_dir).await.map_err(|e| {
            report!(LoopError::Other(format!(
                "failed to initialize local database: {e}"
            )))
        })?;
        tracing::debug!("local SSH host database initialized");

        // Report enrolled hosts to controller (full SSH-based report).
        client::report_enrolled_hosts(&local_db, conn, &self.pool).await;

        // Initialize the host snapshot AFTER reporting so any machine_id
        // updates written by report_enrolled_hosts are captured.
        self.host_snapshot = match host_ops::list_host_snapshots(&local_db).await {
            Ok(snapshot) => snapshot,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to initialize host snapshot; dynamic reload will trigger \
                     a full re-report on the first tick"
                );
                Vec::new()
            }
        };

        // Start (or restart) the reload ticker.  First tick fires
        // HOST_RELOAD_INTERVAL after connect so we do not double-report
        // immediately after the initial report_enrolled_hosts.
        let start = tokio::time::Instant::now() + HOST_RELOAD_INTERVAL;
        let mut ticker = tokio::time::interval_at(start, HOST_RELOAD_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        self.reload_ticker = Some(ticker);

        self.local_db = Some(local_db);
        Ok(())
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        let db = self.local_db.as_ref().ok_or_else(|| {
            report!(LoopError::Other(
                "local_db not initialized: on_connected must be called before on_message"
                    .to_string()
            ))
        })?;
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                Ok(client::handle_check_versions_ssh(payload, db, conn, &self.pool).await)
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                client::handle_execute_update_ssh(
                    *payload,
                    db,
                    &mut self.in_flight_update,
                    conn,
                    &self.pool,
                )
                .await;
                Ok(None)
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                Ok(client::handle_discover_software_ssh(payload, db, conn, &self.pool).await)
            }
            _ => {
                tracing::debug!("ignoring unrecognized message in authenticated loop");
                Ok(None)
            }
        }
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        // Borrow separate fields by name — Rust's field-projection rules allow
        // both borrows simultaneously, sidestepping a double-borrow of `self`.
        tokio::select! {
            biased;
            event = Self::poll_update(&mut self.in_flight_update) => {
                SshAgentEvent::Update(event)
            }
            _ = Self::poll_reload_tick(&mut self.reload_ticker) => {
                SshAgentEvent::HostConfigChanged
            }
        }
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {
            SshAgentEvent::Update(update_event) => {
                let Some(ref update) = self.in_flight_update else {
                    tracing::error!("received update event but no in-flight update exists");
                    return Ok(None);
                };
                let update_history_id = update.update_history_id;

                match update_event {
                    client::UpdateEvent::Output(output_msg) => {
                        client::send_update_output(conn, update_history_id, output_msg).await;
                    }
                    client::UpdateEvent::Completed(result) => {
                        client::send_update_result(conn, update_history_id, result).await;
                        self.in_flight_update = None;
                    }
                }
                Ok(None)
            }

            SshAgentEvent::HostConfigChanged => {
                self.handle_host_config_changed(conn).await;
                Ok(None)
            }
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        client::ssh_agent_capabilities()
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        let outcome = client::handle_graceful_shutdown(
            conn,
            self.in_flight_update.take(),
            shutdown_timeout_seconds,
            disconnect_reason,
            outcome,
        )
        .await;

        // Gracefully close all pooled SSH connections so remote hosts receive
        // a clean disconnect rather than a silent socket drop.
        self.pool.disconnect_all().await;

        outcome
    }
}

impl SshAgentHandler {
    /// React to a host-config reload tick.
    ///
    /// Queries the current `ssh_hosts` snapshot, diffs it against the stored
    /// snapshot, evicts stale pool entries, and sends an updated `ReportHosts`
    /// message if anything changed.  Returns without sending if the snapshot
    /// is unchanged.
    async fn handle_host_config_changed(&mut self, conn: &mut ControllerConnection) {
        let db = match self.local_db.as_ref() {
            Some(db) => db,
            None => {
                // Defensive: reload_ticker is None until on_connected, so this
                // branch should never be reached in practice.
                tracing::warn!("host config reload tick fired before DB was initialized; skipping");
                return;
            }
        };

        let current_snapshot = match host_ops::list_host_snapshots(db).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to list host snapshots during reload check; skipping"
                );
                return;
            }
        };

        if current_snapshot == self.host_snapshot {
            // Nothing changed — no-op.
            return;
        }

        // Compute what changed.  Collect into owned Strings immediately so the
        // borrows of `self.host_snapshot` and `current_snapshot` are released
        // before we update the snapshot and call async methods.
        let (deleted_ids, changed_ids) = {
            let (d, c) = diff_host_snapshots(&self.host_snapshot, &current_snapshot);
            let deleted: HashSet<String> = d.into_iter().map(str::to_string).collect();
            let changed: HashSet<String> = c.into_iter().map(str::to_string).collect();
            (deleted, changed)
        };

        // Evict pool entries for deleted and updated/new hosts so the next
        // acquire establishes a fresh connection rather than reusing a stale
        // or wrong-host session.
        for id in &deleted_ids {
            self.pool.evict(id).await;
        }
        for id in &changed_ids {
            self.pool.evict(id).await;
        }

        // Commit the new snapshot before the async send so that a send failure
        // does not cause us to re-send on the very next tick.
        self.host_snapshot = current_snapshot;

        // Load the full host list for building HostInfo.
        let hosts = match host_ops::list_hosts(db).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to list hosts for dynamic reload; skipping ReportHosts send"
                );
                return;
            }
        };

        tracing::info!(
            total_hosts = hosts.len(),
            changed = changed_ids.len(),
            deleted = deleted_ids.len(),
            "host configuration changed — sending updated ReportHosts"
        );

        // Convert to &str for the client call.
        let changed_ref: HashSet<&str> = changed_ids.iter().map(String::as_str).collect();
        client::report_hosts_after_config_change(db, conn, &hosts, &changed_ref, &self.pool).await;
    }
}

// ---------------------------------------------------------------------------
// Snapshot diff helper
// ---------------------------------------------------------------------------

/// Compute the difference between two host snapshots.
///
/// Returns `(deleted_ids, changed_ids)`:
/// - `deleted_ids`: host IDs present in `prev` but absent from `curr`
/// - `changed_ids`: host IDs that are new in `curr`, or present in both but
///   with a different `updated_at`
fn diff_host_snapshots<'a>(
    prev: &'a [host_ops::HostSnapshot],
    curr: &'a [host_ops::HostSnapshot],
) -> (Vec<&'a str>, HashSet<&'a str>) {
    let prev_map: std::collections::HashMap<&str, i64> =
        prev.iter().map(|s| (s.id.as_str(), s.updated_at)).collect();
    let curr_ids: HashSet<&str> = curr.iter().map(|s| s.id.as_str()).collect();

    let deleted: Vec<&str> = prev
        .iter()
        .filter(|s| !curr_ids.contains(s.id.as_str()))
        .map(|s| s.id.as_str())
        .collect();

    let mut changed: HashSet<&str> = HashSet::new();
    for snap in curr {
        match prev_map.get(snap.id.as_str()) {
            Some(&prev_ts) if prev_ts != snap.updated_at => {
                changed.insert(snap.id.as_str());
            }
            None => {
                // New host — needs SSH to discover machine_id.
                changed.insert(snap.id.as_str());
            }
            _ => {}
        }
    }

    (deleted, changed)
}

// ---------------------------------------------------------------------------
// Shutdown resolution
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-agent-ssh",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    // Host subcommands run with minimal tracing and no rustls provider.
    if let Some(Commands::Host { command }) = args.command {
        // Verbosity-aware tracing for CLI subcommands.
        uptrakit_service_sdk::init_tracing("uptrakit_agent_ssh", args.common.verbose);

        if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }

        let state_dir = match resolve_state_dir_from_common(&args.common).await {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };

        if let Err(e) = commands::host::run(&state_dir, command).await {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // ── Daemon mode ─────────────────────────────────────────────────
    // Validate that --url is provided for daemon mode.
    if args.common.url.is_none() {
        eprintln!("error: --url is required for daemon mode");
        std::process::exit(1);
    }

    uptrakit_service_sdk::init_tracing("uptrakit_agent_ssh", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    // Initialize master encryption key for local SSH credential storage.
    if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
        tracing::error!("{e}");
        std::process::exit(1);
    }

    // Resolve state directory early so we can pass it to the handler.
    let state_dir = match resolve_state_dir_from_common(&args.common).await {
        Ok(dir) => dir,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };

    let mut handler = SshAgentHandler {
        state_dir,
        local_db: None,
        in_flight_update: None,
        pool: ssh_pool::SshConnectionPool::new(),
        reload_ticker: None,
        host_snapshot: Vec::new(),
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent-ssh",
        &args.common,
        &mut handler,
    )
    .await;
}

/// Resolve the state directory for this service.
async fn resolve_state_dir_from_common(
    common: &uptrakit_service_sdk::cli::CommonServiceArgs,
) -> InitResult<std::path::PathBuf> {
    let dirs = common.resolve_dirs("agent-ssh").map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to resolve directories: {e}"
        )))
    })?;
    dirs.ensure_state_dir().await.map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to ensure state directory: {e}"
        )))
    })?;
    Ok(dirs.state_dir().to_path_buf())
}

/// Initialize the master encryption key from CLI args or environment.
fn init_master_key(
    master_key_file: &Option<std::path::PathBuf>,
    allow_plaintext_secrets: bool,
) -> InitResult<()> {
    let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
    let key_hex = read_master_key_hex(master_key_file.as_deref(), env_val.as_deref())?;

    match key_hex {
        Some(key_hex) => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "--allow-plaintext-secrets is enabled. This flag is for development only; \
                    encryption remains enabled because a master key was provided."
                );
            }
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).map_err(|e| {
                report!(InitError::MasterKey(format!(
                    "failed to initialize master key: {e}"
                )))
            })?;
            tracing::info!("master encryption key initialized");
        }
        None => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "master encryption key not set; encryption at rest is disabled. \
                    This is for development only and is NOT safe for production."
                );
            } else {
                bail!(InitError::MasterKey(
                    "master encryption key is required: set UPTRAKIT_MASTER_KEY env var \
                     (64-char hex string) or pass --master-key-file <path>. \
                     For development only, pass --allow-plaintext-secrets to run without \
                     encryption at rest."
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
    env_val: Option<&str>,
) -> InitResult<Option<String>> {
    if let Some(key_file) = master_key_file {
        let contents = std::fs::read_to_string(key_file).map_err(|e| report!(InitError::Io(e)))?;
        return Ok(Some(contents.trim().to_string()));
    }

    if let Some(env_val) = env_val {
        return Ok(Some(env_val.trim().to_string()));
    }

    Ok(None)
}

fn parse_master_key_hex(key_hex: &str) -> InitResult<[u8; 32]> {
    let bytes = uptrakit_shared_types::hex::decode(key_hex).map_err(|e| {
        report!(InitError::Hex(format!(
            "master key must be a 64-character hex string: {e}"
        )))
    })?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        report!(InitError::Hex(format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            v.len()
        )))
    })?;
    Ok(key_bytes)
}

#[cfg(test)]
mod tests {
    use super::{diff_host_snapshots, host_ops, parse_master_key_hex, read_master_key_hex};
    use std::io::Write;

    #[test]
    fn missing_key_returns_none() {
        let result = read_master_key_hex(None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn env_key_is_trimmed() {
        let result = read_master_key_hex(None, Some("  deadbeef  "));
        assert!(matches!(result, Ok(Some(ref value)) if value == "deadbeef"));
    }

    #[test]
    fn file_key_is_trimmed() {
        let mut file = match tempfile::NamedTempFile::new() {
            Ok(f) => f,
            Err(_) => return,
        };
        assert!(file.write_all(b"  0123  ").is_ok());
        let result = read_master_key_hex(Some(file.path()), None);
        assert!(matches!(result, Ok(Some(ref value)) if value == "0123"));
    }

    #[test]
    fn parse_master_key_rejects_invalid_hex() {
        let result = parse_master_key_hex("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_rejects_invalid_length() {
        let result = parse_master_key_hex("aa");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_accepts_valid_length() {
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_master_key_hex(key_hex);
        assert!(matches!(result, Ok(bytes) if bytes.len() == 32));
    }

    // ── snapshot diff tests ──────────────────────────────────────────────────

    fn snap(id: &str, updated_at: i64) -> host_ops::HostSnapshot {
        host_ops::HostSnapshot {
            id: id.to_string(),
            updated_at,
        }
    }

    #[test]
    fn snapshot_diff_no_change_is_noop() {
        let prev = vec![snap("A", 100), snap("B", 200)];
        let curr = prev.clone();
        let (deleted, changed) = diff_host_snapshots(&prev, &curr);
        assert!(
            deleted.is_empty(),
            "expected no deletions, got: {deleted:?}"
        );
        assert!(changed.is_empty(), "expected no changes, got: {changed:?}");
    }

    #[test]
    fn snapshot_diff_detects_added_host() {
        let prev = vec![snap("A", 100)];
        let curr = vec![snap("A", 100), snap("B", 200)];
        let (deleted, changed) = diff_host_snapshots(&prev, &curr);
        assert!(deleted.is_empty(), "expected no deletions");
        assert_eq!(changed.len(), 1, "expected one addition");
        assert!(changed.contains("B"), "expected B in changed set");
    }

    #[test]
    fn snapshot_diff_detects_removed_host() {
        let prev = vec![snap("A", 100), snap("B", 200)];
        let curr = vec![snap("A", 100)];
        let (deleted, changed) = diff_host_snapshots(&prev, &curr);
        assert_eq!(deleted.len(), 1, "expected one deletion");
        assert!(deleted.contains(&"B"), "expected B in deleted set");
        assert!(changed.is_empty(), "expected no additions or updates");
    }

    #[test]
    fn snapshot_diff_detects_updated_host() {
        let prev = vec![snap("A", 100), snap("B", 200)];
        let curr = vec![snap("A", 100), snap("B", 999)];
        let (deleted, changed) = diff_host_snapshots(&prev, &curr);
        assert!(deleted.is_empty(), "expected no deletions");
        assert_eq!(changed.len(), 1, "expected one update");
        assert!(changed.contains("B"), "expected B in changed set");
    }

    #[test]
    fn snapshot_diff_add_and_remove_simultaneously() {
        let prev = vec![snap("A", 100), snap("B", 200)];
        let curr = vec![snap("A", 100), snap("C", 300)];
        let (deleted, changed) = diff_host_snapshots(&prev, &curr);
        assert_eq!(deleted.len(), 1, "expected B deleted");
        assert!(deleted.contains(&"B"));
        assert_eq!(changed.len(), 1, "expected C added");
        assert!(changed.contains("C"));
    }
}
