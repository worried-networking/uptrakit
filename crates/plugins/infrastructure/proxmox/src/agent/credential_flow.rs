//! Cluster-scoped PVE credential flow.
//!
//! [`run_credential_flow`] is the single entry point both `on_host_bootstrapped`
//! and `on_host_synced` delegate to: it creates or reuses the shared
//! `uptrakit@pve` user's per-tenant API token, and drives the legacy-user
//! migration state machine (phase 1: record the legacy user once detected;
//! phase 2: prove-then-delete it and promote the cluster to the ack-confirmed
//! plugin config id; a recovery arm reconciles state if the legacy user is
//! found already gone).
//!
//! A process-global per-cluster [`tokio::sync::Mutex`] registry serializes
//! concurrent flows against the same cluster (bootstrap and sync racing, or
//! two cluster nodes syncing at once) so the migration bookkeeping — which
//! spans several DB writes and is not itself transactional — never races
//! itself. The lock key is the detected cluster name, or the fixed string
//! `"standalone"` for a lone node.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use sea_orm::DatabaseConnection;
use uptrakit_command::RemoteExecutor;
use uptrakit_plugin_infrastructure_core::agent_infra::{InfraPluginContext, PluginConfigReport};

use crate::pve_setup::{self, PveCredentialState};

use super::db_ops;
use super::entity::proxmox_host_state;
use super::plugin::build_pve_config_report;

/// Maximum phase-2 (legacy-user removal) retry attempts before a migration is
/// reported as stuck rather than merely pending. Retries continue past this
/// cap — it only changes the summary-line wording.
const MAX_MIGRATION_ATTEMPTS: i32 = 5;

// ── Per-cluster lock registry ────────────────────────────────────────────────

static CLUSTER_LOCKS: LazyLock<parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

fn cluster_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    Arc::clone(
        CLUSTER_LOCKS
            .lock()
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

// ── Outcome + output ─────────────────────────────────────────────────────────

/// Outcome of the credential/migration flow, mapped to a bootstrap/sync
/// summary line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PveCredentialOutcome {
    Provisioned,
    Reused,
    Regenerated,
    SkippedNoTenant,
    Failed,
    MigrationPending,
}

/// Everything [`run_credential_flow`] resolved, for callers to fold into
/// `BootstrapInfraResult`/`SyncInfraResult`.
pub(crate) struct CredentialFlowOutput {
    pub report: Option<PluginConfigReport>,
    pub existing_config_id: Option<String>,
    pub outcome: PveCredentialOutcome,
    pub summary_lines: Vec<String>,
    /// Whether this run's `check_pve_state` read failed, forcing migration
    /// bookkeeping (phase 1/2/recovery) to sit out and creation to fall back
    /// to the guarded add-user-then-regenerate shape. Exposed so the sync
    /// caller can skip its own `ensure_pve_privileges` repair (which would
    /// otherwise run a further doomed round of `pveum` commands against the
    /// same flaky transport) — not itself part of the numbered decision
    /// branches.
    pub degraded: bool,
}

/// Run the cluster-scoped credential/migration flow for `host_id`.
///
/// Infallible by contract: every internal failure maps to
/// [`PveCredentialOutcome::Failed`] (or a narrower outcome) plus a
/// human-readable summary line, never a propagated error.
pub(crate) async fn run_credential_flow(
    ctx: &InfraPluginContext<'_>,
    executor: &dyn RemoteExecutor,
    host_id: uuid::Uuid,
    node_name: Option<&str>,
) -> CredentialFlowOutput {
    // Branch 1: no tenant context yet.
    let Some(tenant_id) = ctx.tenant_id.and_then(|s| uuid::Uuid::parse_str(s).ok()) else {
        return skipped_no_tenant();
    };

    // Read phase: cluster detection is read-only and runs before the
    // per-cluster lock so an unrelated cluster's slow lock hold never blocks
    // it.
    let cluster_nodes = pve_setup::detect_pve_cluster_nodes(executor).await;
    let cluster_name = pve_setup::detect_pve_cluster_name(executor).await;

    // Branch 2 (empty half): cluster detection failed outright — return
    // before any pveum mutation, including `check_pve_state` itself.
    if cluster_nodes.is_empty() {
        return CredentialFlowOutput {
            report: None,
            existing_config_id: None,
            outcome: PveCredentialOutcome::Failed,
            summary_lines: vec![
                "PVE cluster detection failed; credential setup deferred this run".to_string(),
            ],
            degraded: false,
        };
    }

    let lock_key = cluster_name
        .clone()
        .unwrap_or_else(|| "standalone".to_string());
    let lock = cluster_lock(&lock_key);
    let _guard = lock.lock().await;

    run_locked(
        ctx,
        executor,
        host_id,
        node_name,
        &tenant_id,
        cluster_name.as_deref(),
        &cluster_nodes,
    )
    .await
}

fn skipped_no_tenant() -> CredentialFlowOutput {
    CredentialFlowOutput {
        report: None,
        existing_config_id: None,
        outcome: PveCredentialOutcome::SkippedNoTenant,
        summary_lines: vec![
            "PVE detected; API credential setup skipped: no tenant context".to_string(),
        ],
        degraded: false,
    }
}

/// The credential/migration section proper — everything from
/// `check_pve_state` on, run under the per-cluster lock.
async fn run_locked(
    ctx: &InfraPluginContext<'_>,
    executor: &dyn RemoteExecutor,
    host_id: uuid::Uuid,
    node_name: Option<&str>,
    tenant_id: &uuid::Uuid,
    cluster_name: Option<&str>,
    cluster_nodes: &[String],
) -> CredentialFlowOutput {
    let mut lines: Vec<String> = Vec::new();

    let state_result = pve_setup::check_pve_state(executor, tenant_id).await;
    let degraded = state_result.is_err();
    let state: PveCredentialState = match &state_result {
        Ok(s) => s.clone(),
        Err(_) => PveCredentialState::default(),
    };

    // Branch 3: config name.
    let cfg_name = match cluster_name {
        Some(cn) => format!("pve-{cn}"),
        None => {
            let node_part = node_name.unwrap_or("node");
            let short_id: String = host_id.to_string().chars().take(8).collect();
            format!("pve-{node_part}-{short_id}")
        }
    };

    // Cluster row set — own row always; peer rows (matching pve_node_name)
    // only in a multi-node cluster. Defined once, reused for reads AND
    // writes (phase 1/2/recovery).
    let host_id_str = host_id.to_string();
    let multi_node = cluster_nodes.len() > 1;
    let all_pve_hosts = match db_ops::find_pve_hosts(ctx.db).await {
        Ok(hosts) => hosts,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list local PVE hosts for cluster row set");
            Vec::new()
        }
    };
    let cluster_rows: Vec<proxmox_host_state::Model> = all_pve_hosts
        .into_iter()
        .filter(|h| {
            h.host_id == host_id_str
                || (multi_node
                    && h.pve_node_name
                        .as_deref()
                        .is_some_and(|n| cluster_nodes.iter().any(|c| c == n)))
        })
        .collect();
    let cluster_ids: Vec<String> = cluster_rows.iter().map(|h| h.host_id.clone()).collect();

    // Branch 4 evidence: new_pve_plugin_config_id ack marker across the
    // cluster row set. A bare pve_plugin_config_id never satisfies reuse.
    let evidence_id: Option<String> = {
        let mut ids: Vec<String> = cluster_rows
            .iter()
            .filter_map(|h| h.new_pve_plugin_config_id.clone())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        match ids.len() {
            0 => None,
            1 => ids.into_iter().next(),
            _ => {
                tracing::warn!(
                    candidates = ?ids,
                    "cluster peers disagree on new_pve_plugin_config_id (likely duplicate \
                     configs from a failed bootstrap dedup); using newest"
                );
                ids.into_iter().max()
            }
        }
    };

    // Reuse is invalidated only when this run's read succeeded AND confirmed
    // the token absent — a degraded read never invalidates reuse.
    let confirmed_token_absent = matches!(&state_result, Ok(s) if !s.our_token_exists);

    let mut outcome: PveCredentialOutcome;
    let mut report: Option<PluginConfigReport> = None;
    let mut existing_config_id: Option<String> = None;

    if let Some(id) = evidence_id.clone().filter(|_| !confirmed_token_absent) {
        // Branch 4: reuse. Persist the resolved id onto THIS flow host's own
        // row via the coalesce-based setter — it fills a NULL operative
        // column (a fresh peer picking up a cluster-mate's marker) while
        // leaving a legacy row's operative id untouched until phase 2
        // promotes it. A bare `upsert_host_state` would clobber that legacy
        // id mid-migration, so it must not be used here.
        if let Err(e) = db_ops::set_new_plugin_config_id(ctx.db, &host_id_str, &id).await {
            tracing::warn!(error = %e, "failed to persist reused PVE plugin config id");
            lines.push(
                "PVE API credentials: reused existing plugin config, but failed to \
                persist the reused id locally (see agent logs)"
                    .to_string(),
            );
        } else {
            lines.push("PVE API credentials: reused existing plugin config".to_string());
        }
        outcome = PveCredentialOutcome::Reused;
        existing_config_id = Some(id);
    } else if state.our_token_exists {
        // Branch 6: token confirmed present but no reusable evidence anywhere
        // (ack marker lost) — regenerate.
        //
        // NOTE: `regenerate_pve_api_token` calls `ensure_pve_acls` but not
        // `ensure_pve_roles` (see pve_setup.rs). This is the spec-mandated
        // shape for both this branch and the degraded-create path below
        // (which also falls through to regenerate): a token-holding node is
        // assumed to already have its roles from an earlier provisioning
        // run. If a genuinely fresh node ever reaches a regenerate call with
        // roles never created, ACLs would be granted against roles that do
        // not exist. This is a known limitation, not a bug to fix here.
        match pve_setup::regenerate_pve_api_token(executor, tenant_id).await {
            Ok(creds) => match pve_setup::prove_token_on_node(executor, &creds.api_token).await {
                Ok(()) => {
                    report = build_pve_config_report(cfg_name.clone(), &creds);
                    outcome = PveCredentialOutcome::Regenerated;
                    lines.push("PVE API token regenerated".to_string());
                }
                Err(e) => {
                    tracing::warn!(error = %e, "PVE token proof failed after regeneration");
                    outcome = PveCredentialOutcome::Failed;
                    lines.push("PVE credential setup failed (see agent logs)".to_string());
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "failed to regenerate PVE API token");
                outcome = PveCredentialOutcome::Failed;
                lines.push("PVE credential setup failed (see agent logs)".to_string());
            }
        }
    } else {
        // Branch 5: no token — create. Under a degraded read the state we
        // decided this from is a default, not a verified absence, so use the
        // guarded add-user-then-regenerate shape instead of the unguarded
        // create path (which would fail loudly if a token already exists).
        let create_result = if degraded {
            let add_user_cmd = format!(
                "pveum user add '{}' --comment 'Uptrakit managed user' 2>&1 || true",
                pve_setup::PVE_USER
            );
            if let Err(e) = executor.exec_command(&add_user_cmd).await {
                tracing::warn!(
                    error = %e,
                    "failed to ensure the shared PVE user exists before a degraded-read regenerate"
                );
            }
            pve_setup::regenerate_pve_api_token(executor, tenant_id).await
        } else {
            pve_setup::create_pve_api_credentials(executor, tenant_id).await
        };
        match create_result {
            Ok(creds) => match pve_setup::prove_token_on_node(executor, &creds.api_token).await {
                Ok(()) => {
                    report = build_pve_config_report(cfg_name.clone(), &creds);
                    outcome = PveCredentialOutcome::Provisioned;
                    lines.push("PVE API credentials created".to_string());
                }
                Err(e) => {
                    tracing::warn!(error = %e, "PVE token proof failed after creation");
                    outcome = PveCredentialOutcome::Failed;
                    lines.push("PVE credential setup failed (see agent logs)".to_string());
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "failed to create PVE API credentials");
                outcome = PveCredentialOutcome::Failed;
                lines.push("PVE credential setup failed (see agent logs)".to_string());
            }
        }
    }

    // Branches 7-9: migration bookkeeping, skipped entirely under a degraded
    // read (destructive/state-changing arms must never fire off unverified
    // state).
    let mut migration_pending = false;
    if degraded {
        if cluster_rows.iter().any(|h| h.legacy_pve_user.is_some()) {
            let err_text = state_result
                .as_ref()
                .err()
                .map_or_else(String::new, |e| e.to_string());
            lines.push(format!(
                "PVE state read degraded; migration paused this run ({err_text})"
            ));
            migration_pending = true;
        }
    } else {
        // Branch 7: phase 1 — record a freshly-detected legacy user.
        if let Some(name) = state.legacy_user.clone() {
            match db_ops::set_legacy_pve_user(ctx.db, &cluster_ids, Some(name)).await {
                Ok(()) => {
                    lines.push("legacy PVE user detected; migration phase 1 recorded".to_string());
                }
                Err(e) => tracing::warn!(error = %e, "failed to record legacy PVE user marker"),
            }
        }

        let any_legacy_stored =
            state.legacy_user.is_some() || cluster_rows.iter().any(|h| h.legacy_pve_user.is_some());

        if any_legacy_stored {
            migration_pending = true;

            if state.legacy_user.is_none() {
                // Branch 9: recovery — a legacy marker is stored but this
                // run's successful read shows the legacy user is gone.
                if let Some(id) = evidence_id.clone() {
                    match db_ops::promote_cluster_rows(ctx.db, &cluster_ids, &id).await {
                        Ok(()) => {
                            migration_pending = false;
                            lines
                                .push("legacy PVE user already gone; state reconciled".to_string());
                        }
                        Err(e) => tracing::warn!(
                            error = %e,
                            "failed to promote cluster rows during recovery"
                        ),
                    }
                } else {
                    match db_ops::set_legacy_pve_user(ctx.db, &cluster_ids, None).await {
                        Ok(()) => {
                            migration_pending = false;
                            lines
                                .push("legacy PVE user already gone; state reconciled".to_string());
                        }
                        Err(e) => tracing::warn!(
                            error = %e,
                            "failed to clear stale legacy PVE user marker"
                        ),
                    }
                }
            } else if let Some(new_id) = evidence_id.clone().filter(|_| state.our_token_exists) {
                // Branch 8: phase 2 — prove-then-delete promotion.
                let legacy_name = state.legacy_user.clone().unwrap_or_default();
                match pve_setup::delete_pve_user(executor, &legacy_name).await {
                    Ok(()) => match db_ops::promote_cluster_rows(ctx.db, &cluster_ids, &new_id)
                        .await
                    {
                        Ok(()) => {
                            lines.push("legacy PVE user removed; migration complete".to_string());
                            migration_pending = false;
                        }
                        Err(e) => tracing::warn!(
                            error = %e,
                            "failed to promote cluster rows after legacy user removal"
                        ),
                    },
                    Err(e) => {
                        if let Err(inc_err) =
                            db_ops::increment_migration_attempts(ctx.db, &cluster_ids).await
                        {
                            tracing::warn!(
                                error = %inc_err,
                                "failed to increment migration attempt counter"
                            );
                        }
                        let attempts = cluster_migration_attempts_max(ctx.db, &cluster_ids)
                            .await
                            .unwrap_or(0);
                        if attempts >= MAX_MIGRATION_ATTEMPTS {
                            lines.push(format!("migration STUCK after {attempts} attempts: {e}"));
                        } else {
                            lines.push(format!(
                                "migration pending: legacy user removal failed (attempt {attempts})"
                            ));
                        }
                    }
                }
            }
            // else: phase-2 gate not met (no ack marker, or the token isn't
            // confirmed present yet) — nothing to do this run.
        }
    }

    // Branch 10: MigrationPending takes precedence over whatever 4-6
    // produced when the cluster is still mid-migration after this run —
    // except a genuine Failed outcome from 4-6, which must never be masked
    // by a merely-pending migration.
    if migration_pending && outcome != PveCredentialOutcome::Failed {
        outcome = PveCredentialOutcome::MigrationPending;
    }

    CredentialFlowOutput {
        report,
        existing_config_id,
        outcome,
        summary_lines: lines,
        degraded,
    }
}

/// Re-read the given cluster rows and return the maximum `migration_attempts`
/// across them. `None` on a DB read failure or an empty match set.
async fn cluster_migration_attempts_max(
    db: &DatabaseConnection,
    host_ids: &[String],
) -> Option<i32> {
    let hosts = db_ops::find_pve_hosts(db).await.ok()?;
    hosts
        .into_iter()
        .filter(|h| host_ids.iter().any(|id| id == &h.host_id))
        .map(|h| h.migration_attempts)
        .max()
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;
    use uptrakit_command::test_support::ScriptedRemoteExecutor;
    use uptrakit_command::{RemoteCommandResult, RemoteExecutor};
    use uptrakit_plugin_infrastructure_core::agent_infra::{
        GuestBootstrapError, GuestBootstrapExecutor, GuestBootstrapParams, GuestBootstrapResult,
        InfraPluginContext,
    };
    use uptrakit_plugin_infrastructure_core::testing::RecordingActionInvoker;

    use super::*;
    use crate::agent::db_ops;

    fn ok(stdout: impl Into<String>) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn err(exit_code: u32, stdout: impl Into<String>) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code,
        }
    }

    fn tenant(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(n)
    }

    fn cluster_status(cluster: Option<&str>, nodes: &[&str]) -> String {
        let mut entries: Vec<serde_json::Value> = Vec::new();
        if let Some(c) = cluster {
            entries.push(json!({"type": "cluster", "name": c}));
        }
        for n in nodes {
            entries.push(json!({"type": "node", "name": n}));
        }
        serde_json::Value::Array(entries).to_string()
    }

    fn token_list_empty() -> RemoteCommandResult {
        ok("[]")
    }

    fn user_list_empty() -> RemoteCommandResult {
        ok("[]")
    }

    fn token_add_ok(secret: &str) -> RemoteCommandResult {
        ok(format!(
            r#"{{"full-tokenid":"uptrakit@pve!x","info":{{"privsep":"1"}},"value":"{secret}"}}"#
        ))
    }

    async fn setup_agent_db() -> sea_orm::DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        let manager = sea_orm_migration::SchemaManager::new(&db);
        for migration in crate::agent::migration::agent_migrations() {
            migration.up(&manager).await.expect("agent migration");
        }
        db
    }

    struct UnusedGuestBootstrap;

    #[async_trait]
    impl GuestBootstrapExecutor for UnusedGuestBootstrap {
        async fn bootstrap_guest(
            &self,
            _params: GuestBootstrapParams,
        ) -> std::result::Result<GuestBootstrapResult, GuestBootstrapError> {
            Err(GuestBootstrapError::from(
                "guest bootstrap must not be invoked by credential-flow tests",
            ))
        }
    }

    fn make_ctx<'a>(
        db: &'a DatabaseConnection,
        tenant_id: Option<&'a str>,
        invoker: &'a RecordingActionInvoker,
        guest_bootstrap: &'a UnusedGuestBootstrap,
    ) -> InfraPluginContext<'a> {
        InfraPluginContext {
            db,
            tenant_id,
            service_id: None,
            state_dir: std::path::Path::new("."),
            private_key_der: None,
            action_invoker: invoker,
            guest_bootstrap,
            provision_credentials: true,
        }
    }

    /// A `RemoteExecutor` used by [`cluster_lock_serializes_concurrent_flows`]
    /// that records start/end marks around a deliberately delayed response to
    /// the `pveum user token list` call so the two concurrent flows' timing
    /// can be checked for non-interleaving.
    struct TimelineExecutor {
        label: &'static str,
        timeline: Arc<parking_lot::Mutex<Vec<&'static str>>>,
        cluster_json: String,
    }

    #[async_trait]
    impl RemoteExecutor for TimelineExecutor {
        async fn exec_command(
            &self,
            command: &str,
        ) -> uptrakit_command::Result<RemoteCommandResult> {
            if command.contains("pvesh get /cluster/status") {
                return Ok(ok(self.cluster_json.clone()));
            }
            if command.contains("pveum user token list") {
                self.timeline.lock().push(self.label);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                self.timeline.lock().push(self.label);
                return Ok(ok("[]"));
            }
            if command.contains("pveum user list") {
                return Ok(ok("[]"));
            }
            if command.contains("pveum user token add") {
                return Ok(token_add_ok("secret-timeline"));
            }
            if command.contains("hostname -f") {
                return Ok(ok("pve.example.com"));
            }
            if command.contains("curl") {
                return Ok(ok("200"));
            }
            Ok(ok(""))
        }
    }

    #[tokio::test]
    async fn fresh_cluster_provisions_and_reports() {
        // Variant A: multi-node cluster -> cfg name "pve-{cluster}".
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(1);
        db_ops::upsert_host_state(
            &db,
            &host_id.to_string(),
            true,
            None,
            Some("pve1".to_string()),
        )
        .await
        .expect("seed host");
        let tid = tenant(1);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("MYCLUSTER"), &["pve1", "pve2"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-a")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;

        assert_eq!(out.outcome, PveCredentialOutcome::Provisioned);
        assert!(!out.degraded);
        let report = out.report.expect("report built on fresh provision");
        assert_eq!(report.name, "pve-MYCLUSTER");
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("PVE API credentials created")),
            "{:?}",
            out.summary_lines
        );
        let calls = executor.recorded_calls();
        assert!(calls.iter().any(|c| c.contains("pveum user token add")));
        assert!(calls.iter().any(|c| c.contains("curl")));
        assert!(
            calls
                .iter()
                .any(|c| c.contains("pveum user token add") && c.contains("--privsep=1")),
            "the API token must be created with privilege separation on, not inheriting the \
             user's full privilege set: {calls:?}"
        );

        // Variant B: standalone (no cluster) -> cfg name "pve-{node}-{short id}".
        let db2 = setup_agent_db().await;
        let host_id2 = uuid::Uuid::from_u128(2);
        db_ops::upsert_host_state(
            &db2,
            &host_id2.to_string(),
            true,
            None,
            Some("solo1".to_string()),
        )
        .await
        .expect("seed host");
        let tid2 = tenant(2);
        let invoker2 = RecordingActionInvoker::new();
        let guest2 = UnusedGuestBootstrap;
        let tid2_str = tid2.to_string();
        let ctx2 = make_ctx(&db2, Some(&tid2_str), &invoker2, &guest2);
        let cluster_json2 = cluster_status(None, &["solo1"]);
        let executor2 = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json2)),
            ("pveum user token list", token_list_empty()),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-b")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out2 = run_credential_flow(&ctx2, &executor2, host_id2, Some("solo1")).await;
        assert_eq!(out2.outcome, PveCredentialOutcome::Provisioned);
        let report2 = out2.report.expect("report built");
        let short_id: String = host_id2.to_string().chars().take(8).collect();
        assert_eq!(report2.name, format!("pve-solo1-{short_id}"));
    }

    #[tokio::test]
    async fn coexisting_tenant_tokens_untouched() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(3);
        db_ops::upsert_host_state(
            &db,
            &host_id.to_string(),
            true,
            None,
            Some("pve1".to_string()),
        )
        .await
        .expect("seed host");
        let tid = tenant(3);
        let other_tid = tenant(30);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("COTENANTCLUSTER"), &["pve1"]);
        let token_list = ok(format!(r#"[{{"tokenid":"tenant-{other_tid}"}}]"#));
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-c")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(out.outcome, PveCredentialOutcome::Provisioned);
        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("token remove")),
            "another tenant's coexisting token must never trigger a remove: {calls:?}"
        );
        let add_call = calls
            .iter()
            .find(|c| c.contains("pveum user token add"))
            .expect("token add recorded");
        assert!(
            add_call.contains(&format!("tenant-{tid}")),
            "must add OUR tenant's token id: {add_call}"
        );
    }

    #[tokio::test]
    async fn reuse_bare_operative_id_without_ack_marker_is_not_reused() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(10);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(
            &db,
            &host_id_str,
            true,
            Some("legacy-cfg".to_string()),
            Some("pve1".to_string()),
        )
        .await
        .expect("seed host with bare operative id");
        let tid = tenant(10);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("BAREOPCLUSTER"), &["pve1"]);
        // The token IS present (branch 6, regenerate) — a bare operative id
        // with no ack marker must fall through to regenerate, not create.
        // Scripting an empty token list would land on branch 5 (create)
        // instead, which does not exercise the claimed branch.
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            ("pveum user list", user_list_empty()),
            ("pveum user token remove", ok("")),
            ("pveum user token add", token_add_ok("secret-d")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_ne!(
            out.outcome,
            PveCredentialOutcome::Reused,
            "a bare pve_plugin_config_id with no ack marker must never satisfy reuse"
        );
        assert_eq!(
            out.outcome,
            PveCredentialOutcome::Regenerated,
            "confirmed-present token with no ack marker must regenerate, not create"
        );
        let calls = executor.recorded_calls();
        assert!(
            calls.iter().any(|c| c.contains("pveum user token remove")),
            "expected the regenerate shape to run, not the create shape: {calls:?}"
        );
        assert!(out.existing_config_id.is_none());
    }

    #[tokio::test]
    async fn reuse_with_ack_marker_and_confirmed_token_reuses() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(11);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        db_ops::set_new_plugin_config_id(&db, &host_id_str, "ack-cfg")
            .await
            .expect("ack marker");
        let tid = tenant(11);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("ACKREUSECLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            ("pveum user list", user_list_empty()),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(out.outcome, PveCredentialOutcome::Reused);
        assert_eq!(out.existing_config_id.as_deref(), Some("ack-cfg"));
        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("pveum user token add")),
            "reuse must not create/regenerate: {calls:?}"
        );
    }

    #[tokio::test]
    async fn reuse_multiple_ack_markers_uses_max() {
        let db = setup_agent_db().await;
        let host_a = uuid::Uuid::from_u128(12);
        let host_b = uuid::Uuid::from_u128(13);
        let host_a_str = host_a.to_string();
        let host_b_str = host_b.to_string();
        db_ops::upsert_host_state(&db, &host_a_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed a");
        db_ops::upsert_host_state(&db, &host_b_str, true, None, Some("pve2".to_string()))
            .await
            .expect("seed b");
        db_ops::set_new_plugin_config_id(&db, &host_a_str, "cfg-a")
            .await
            .expect("ack a");
        db_ops::set_new_plugin_config_id(&db, &host_b_str, "cfg-b")
            .await
            .expect("ack b");
        let tid = tenant(12);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("DUPCLUSTER"), &["pve1", "pve2"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            ("pveum user list", user_list_empty()),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_a, Some("pve1")).await;
        assert_eq!(out.outcome, PveCredentialOutcome::Reused);
        assert_eq!(
            out.existing_config_id.as_deref(),
            Some("cfg-b"),
            "must pick the lexicographic max of disagreeing peer ack markers"
        );
    }

    #[tokio::test]
    async fn reuse_persists_peer_evidence_id_onto_flow_hosts_own_row() {
        // Regression test for the bug where the reuse branch computed
        // `existing_config_id` for the in-memory report but never persisted
        // it onto the flow host's own row, silently dropping a peer-reusing
        // cluster node out of the guest-bootstrap surface (which filters on
        // `h.pve_plugin_config_id.is_some()`). The flow host's row starts
        // completely NULL; only the peer carries the ack marker.
        let db = setup_agent_db().await;
        let flow_host = uuid::Uuid::from_u128(16);
        let flow_host_str = flow_host.to_string();
        let peer_host = uuid::Uuid::from_u128(17);
        let peer_host_str = peer_host.to_string();
        db_ops::upsert_host_state(&db, &flow_host_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed flow host with NULL operative id and no ack marker");
        db_ops::upsert_host_state(&db, &peer_host_str, true, None, Some("pve2".to_string()))
            .await
            .expect("seed peer host");
        db_ops::set_new_plugin_config_id(&db, &peer_host_str, "peer-cfg")
            .await
            .expect("seed peer ack marker");

        let tid = tenant(16);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("PEEREVIDENCECLUSTER"), &["pve1", "pve2"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            ("pveum user list", user_list_empty()),
        ]);

        let out = run_credential_flow(&ctx, &executor, flow_host, Some("pve1")).await;
        assert_eq!(out.outcome, PveCredentialOutcome::Reused);
        assert_eq!(out.existing_config_id.as_deref(), Some("peer-cfg"));

        // The load-bearing assertion: deleting the B-1 fix's
        // `set_new_plugin_config_id` call on the reuse branch would leave
        // both of these columns NULL, since nothing else writes to the flow
        // host's own row on this path.
        let row = db_ops::find_host_state(&db, &flow_host_str)
            .await
            .expect("query")
            .expect("flow host row exists");
        assert_eq!(
            row.new_pve_plugin_config_id.as_deref(),
            Some("peer-cfg"),
            "the ack marker must be persisted onto the flow host's own row, not just returned \
             in the in-memory report"
        );
        assert_eq!(
            row.pve_plugin_config_id.as_deref(),
            Some("peer-cfg"),
            "the coalesce fill must also promote the flow host's own NULL operative column so \
             the guest-bootstrap surface's `pve_plugin_config_id.is_some()` filter picks it up"
        );
    }

    #[tokio::test]
    async fn reuse_dead_token_evidence_falls_through_to_create() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(14);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        db_ops::set_new_plugin_config_id(&db, &host_id_str, "stale-cfg")
            .await
            .expect("ack marker");
        let tid = tenant(14);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("DEADTOKENCLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-e")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_ne!(
            out.outcome,
            PveCredentialOutcome::Reused,
            "an ack marker with THIS run's read confirming token absence must not be reused"
        );
        assert_eq!(out.outcome, PveCredentialOutcome::Provisioned);
        let report = out
            .report
            .as_ref()
            .expect("provisioned run must carry a report");
        assert_eq!(
            report.name, "pve-DEADTOKENCLUSTER",
            "the re-created report must carry the same Step-3 config name as the original \
             provisioning would have used, not a fresh/mismatched one"
        );
    }

    #[tokio::test]
    async fn reuse_standalone_peer_isolation() {
        let db = setup_agent_db().await;
        // A stray row for a DIFFERENT host that happens to share the node
        // name, carrying an ack marker that must never leak into a
        // standalone run.
        let stray_id = "stray-host";
        db_ops::upsert_host_state(&db, stray_id, true, None, Some("pve1".to_string()))
            .await
            .expect("seed stray");
        db_ops::set_new_plugin_config_id(&db, stray_id, "stray-cfg")
            .await
            .expect("stray ack");

        let host_id = uuid::Uuid::from_u128(15);
        let tid = tenant(15);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        // Standalone: exactly one cluster node -> multi_node = false.
        let cluster_json = cluster_status(None, &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-f")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_ne!(
            out.outcome,
            PveCredentialOutcome::Reused,
            "standalone flow must not inherit a stray host's ack marker even with a matching node name"
        );
        assert!(out.existing_config_id.is_none());
    }

    #[tokio::test]
    async fn phase1_records_legacy_and_keeps_it_alive() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(20);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        let tid = tenant(20);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("PHASE1CLUSTER"), &["pve1"]);
        let legacy_name = format!("uptrakit-{tid}@pve");
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-g")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert!(!out.degraded);
        assert_eq!(
            out.outcome,
            PveCredentialOutcome::MigrationPending,
            "a freshly detected legacy user must gate the outcome to MigrationPending"
        );
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("migration phase 1 recorded")),
            "{:?}",
            out.summary_lines
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.legacy_pve_user.as_deref(), Some(legacy_name.as_str()));
        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("pveum user delete")),
            "phase 1 only records the legacy user; it must never delete it: {calls:?}"
        );
    }

    #[tokio::test]
    async fn legacy_stored_without_ack_marker_never_deletes() {
        // Every row's bare `pve_plugin_config_id` is Some — only the ack
        // marker (`new_pve_plugin_config_id`) is missing everywhere. Branch 8
        // (prove-then-delete) is gated on the ack marker, not the bare
        // operative id, so this must never delete despite a stored legacy
        // user and a non-NULL operative column on every row.
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(200);
        let host_id_str = host_id.to_string();
        let tid = tenant(200);
        let legacy_name = format!("uptrakit-{tid}@pve");
        db_ops::upsert_host_state(
            &db,
            &host_id_str,
            true,
            Some("legacy-cfg".to_string()),
            Some("pve1".to_string()),
        )
        .await
        .expect("seed host with bare operative id, no ack marker");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some(legacy_name.clone()),
        )
        .await
        .expect("seed legacy marker");

        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("NOACKCLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum user token remove", ok("")),
            ("pveum user token add", token_add_ok("secret-noack")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(
            out.outcome,
            PveCredentialOutcome::MigrationPending,
            "legacy user still stored must gate the outcome to MigrationPending"
        );
        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("pveum user delete")),
            "the ack marker gate must hold even when a bare operative id is set on every row: \
             {calls:?}"
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(
            row.legacy_pve_user.as_deref(),
            Some(legacy_name.as_str()),
            "the legacy marker must survive since nothing promoted or deleted it"
        );
        assert_eq!(
            row.pve_plugin_config_id.as_deref(),
            Some("legacy-cfg"),
            "the bare operative id must be untouched by a phase-2 gate that never fired"
        );
    }

    #[tokio::test]
    async fn phase2_prove_then_delete_promotes_on_success() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(21);
        let host_id_str = host_id.to_string();
        let tid = tenant(21);
        let legacy_name = format!("uptrakit-{tid}@pve");
        // Operative id starts at a DIFFERENT value ("legacy-cfg") than the
        // ack marker ("ack-cfg") so promotion must observably change the
        // column — seeding the ack marker onto a NULL operative column (as
        // the previous fixture did) pre-satisfies the headline assertion via
        // `set_new_plugin_config_id`'s own coalesce fill, before
        // `promote_cluster_rows` ever runs.
        db_ops::upsert_host_state(
            &db,
            &host_id_str,
            true,
            Some("legacy-cfg".to_string()),
            Some("pve1".to_string()),
        )
        .await
        .expect("seed host with legacy operative id");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some(legacy_name.clone()),
        )
        .await
        .expect("seed legacy marker");
        db_ops::set_new_plugin_config_id(&db, &host_id_str, "ack-cfg")
            .await
            .expect("seed ack marker");
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("PHASE2SUCCESSCLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum user delete", ok("")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(
            out.outcome,
            PveCredentialOutcome::Reused,
            "phase-2 success must not override an otherwise-Reused outcome"
        );
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("legacy PVE user removed; migration complete")),
            "{:?}",
            out.summary_lines
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.legacy_pve_user, None);
        assert_eq!(
            row.pve_plugin_config_id.as_deref(),
            Some("ack-cfg"),
            "promotion must observably move the operative id off its pre-seeded legacy value"
        );
        assert_eq!(row.migration_attempts, 0);
    }

    #[tokio::test]
    async fn phase2_promotes_both_rows_in_a_multi_node_cluster() {
        // Only single-row (above) and negative-scope (below,
        // `phase2_excludes_peer_row_with_null_node_name` /
        // `phase2_standalone_write_scope_isolated`) promotion tests existed;
        // nothing asserted the positive multi-row write scope — that a real
        // two-node cluster's peer row is ALSO promoted, not just the flow
        // host's own row.
        let db = setup_agent_db().await;
        let host_a = uuid::Uuid::from_u128(23);
        let host_a_str = host_a.to_string();
        let host_b = uuid::Uuid::from_u128(24);
        let host_b_str = host_b.to_string();
        let tid = tenant(23);
        let legacy_name = format!("uptrakit-{tid}@pve");

        for (id, node) in [(&host_a_str, "pve1"), (&host_b_str, "pve2")] {
            db_ops::upsert_host_state(
                &db,
                id,
                true,
                Some("legacy-cfg".to_string()),
                Some(node.to_string()),
            )
            .await
            .expect("seed cluster row with legacy operative id");
            db_ops::set_legacy_pve_user(&db, std::slice::from_ref(id), Some(legacy_name.clone()))
                .await
                .expect("seed legacy marker");
        }
        db_ops::set_new_plugin_config_id(&db, &host_a_str, "ack-cfg")
            .await
            .expect("seed ack marker on the flow host's row");

        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("MULTIROWPROMOTECLUSTER"), &["pve1", "pve2"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum user delete", ok("")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_a, Some("pve1")).await;
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("legacy PVE user removed; migration complete")),
            "{:?}",
            out.summary_lines
        );

        let row_a = db_ops::find_host_state(&db, &host_a_str)
            .await
            .expect("query a")
            .expect("row a exists");
        let row_b = db_ops::find_host_state(&db, &host_b_str)
            .await
            .expect("query b")
            .expect("row b exists");
        assert_eq!(
            row_a.pve_plugin_config_id.as_deref(),
            Some("ack-cfg"),
            "the flow host's own row must be promoted"
        );
        assert_eq!(
            row_b.pve_plugin_config_id.as_deref(),
            Some("ack-cfg"),
            "the peer row must ALSO be promoted — this is the multi-row write scope the \
             single-row test above cannot exercise"
        );
        assert_eq!(row_a.legacy_pve_user, None);
        assert_eq!(row_b.legacy_pve_user, None);
    }

    #[tokio::test]
    async fn phase2_delete_failure_increments_attempts_and_pends() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(22);
        let host_id_str = host_id.to_string();
        let tid = tenant(22);
        let legacy_name = format!("uptrakit-{tid}@pve");
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some(legacy_name.clone()),
        )
        .await
        .expect("seed legacy marker");
        db_ops::set_new_plugin_config_id(&db, &host_id_str, "ack-cfg")
            .await
            .expect("seed ack marker");
        // Pre-seed migration_attempts = 3 so the mandated post-increment
        // re-read is exercised for real: on a first attempt (0 -> 1) the
        // pre-increment-plus-one and the post-increment re-read both yield 1,
        // so that path cannot distinguish a re-read from a locally-tracked
        // counter. Seeding a non-trivial starting value pins the re-read.
        for _ in 0..3 {
            db_ops::increment_migration_attempts(&db, std::slice::from_ref(&host_id_str))
                .await
                .expect("pre-seed attempts");
        }
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("PHASE2FAILCLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum user delete", err(1, "denied")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(
            out.outcome,
            PveCredentialOutcome::MigrationPending,
            "a failed legacy-user delete must pend the outcome"
        );
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("migration pending: legacy user removal failed (attempt 4)")),
            "expected the post-increment re-read (3 -> 4), not a locally-tracked value: {:?}",
            out.summary_lines
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.migration_attempts, 4);
        assert_eq!(
            row.legacy_pve_user.as_deref(),
            Some(legacy_name.as_str()),
            "marker survives a failed removal attempt"
        );
    }

    #[tokio::test]
    async fn phase2_delete_failure_at_cap_reports_stuck() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(220);
        let host_id_str = host_id.to_string();
        let tid = tenant(220);
        let legacy_name = format!("uptrakit-{tid}@pve");
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some(legacy_name.clone()),
        )
        .await
        .expect("seed legacy marker");
        db_ops::set_new_plugin_config_id(&db, &host_id_str, "ack-cfg")
            .await
            .expect("seed ack marker");
        // MAX_MIGRATION_ATTEMPTS - 1 = 4 pre-seeded attempts: this run's
        // failure increments to 5, exactly at the cap, so the STUCK wording
        // must fire.
        for _ in 0..(MAX_MIGRATION_ATTEMPTS - 1) {
            db_ops::increment_migration_attempts(&db, std::slice::from_ref(&host_id_str))
                .await
                .expect("pre-seed attempts to cap - 1");
        }
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("STUCKCLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum user delete", err(1, "denied")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(out.outcome, PveCredentialOutcome::MigrationPending);
        assert!(
            out.summary_lines.iter().any(|l| l.contains(&format!(
                "migration STUCK after {MAX_MIGRATION_ATTEMPTS} attempts"
            ))),
            "{:?}",
            out.summary_lines
        );
        assert!(
            !out.summary_lines
                .iter()
                .any(|l| l.contains("migration pending: legacy user removal failed")),
            "the STUCK wording must replace, not accompany, the pending wording: {:?}",
            out.summary_lines
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.migration_attempts, MAX_MIGRATION_ATTEMPTS);
    }

    #[tokio::test]
    async fn phase2_excludes_peer_row_with_null_node_name() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(23);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed own");
        // Peer row: is_pve_node but pve_node_name is NULL (never detected)
        // and carries a stale legacy marker; must be excluded from the
        // cluster row set entirely so it can never spuriously gate this
        // run's outcome.
        db_ops::upsert_host_state(&db, "peer-no-name", true, None, None)
            .await
            .expect("seed peer");
        db_ops::set_legacy_pve_user(
            &db,
            &["peer-no-name".to_string()],
            Some("uptrakit-stale@pve".to_string()),
        )
        .await
        .expect("seed peer legacy");

        let tid = tenant(23);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("MULTICLUSTER"), &["pve1", "pve2"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-h")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_ne!(
            out.outcome,
            PveCredentialOutcome::MigrationPending,
            "a peer row with a NULL pve_node_name must never be pulled into the cluster row set"
        );

        let peer = db_ops::find_host_state(&db, "peer-no-name")
            .await
            .expect("query")
            .expect("peer exists");
        assert_eq!(
            peer.legacy_pve_user.as_deref(),
            Some("uptrakit-stale@pve"),
            "excluded peer must be untouched"
        );
    }

    #[tokio::test]
    async fn phase2_standalone_write_scope_isolated() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(24);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed own");
        // An unrelated row that coincidentally shares the node name;
        // standalone (single-node cluster) must never treat it as a peer.
        db_ops::upsert_host_state(&db, "other-host", true, None, Some("pve1".to_string()))
            .await
            .expect("seed other");

        let tid = tenant(24);
        let legacy_name = format!("uptrakit-{tid}@pve");
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(None, &["pve1"]); // single node -> standalone
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-i")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;

        let own = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("own exists");
        assert_eq!(
            own.legacy_pve_user.as_deref(),
            Some(legacy_name.as_str()),
            "own row gets the phase-1 marker"
        );

        let other = db_ops::find_host_state(&db, "other-host")
            .await
            .expect("query")
            .expect("other exists");
        assert_eq!(
            other.legacy_pve_user, None,
            "standalone must never write to a same-node-name peer row"
        );
    }

    #[tokio::test]
    async fn degraded_read_never_fires_destructive_arms() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(30);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some("uptrakit-stuck@pve".to_string()),
        )
        .await
        .expect("seed stuck legacy marker");
        let tid = tenant(30);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("DEGRADEDCLUSTER"), &["pve1"]);
        // A real token-add response is scripted (unlike the previous fixture,
        // which left `pveum user token add` unmatched — ScriptedRemoteExecutor
        // then defaulted it to exit 0/empty stdout, `parse_token_value` failed,
        // and the flow genuinely landed on `Failed`, which the buggy
        // unconditional MigrationPending overwrite masked). With that overwrite
        // now conditional on `outcome != Failed`, this fixture must make the
        // guarded degraded-create shape actually succeed so MigrationPending is
        // reached honestly (via the still-open legacy marker), not by masking
        // a hidden Failed.
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                err(1, "permission denied: totally unrelated"),
            ),
            ("pveum user add", ok("")),
            ("pveum user token remove", ok("")),
            ("pveum user token add", token_add_ok("secret-degraded")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert!(out.degraded);
        assert_eq!(out.outcome, PveCredentialOutcome::MigrationPending);
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("PVE state read degraded; migration paused this run")),
            "{:?}",
            out.summary_lines
        );

        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("pveum user delete")),
            "a degraded read must never fire the phase-2 delete: {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.contains("pveum role add")),
            "a degraded read must fall back to the guarded shape, not ensure_pve_roles: {calls:?}"
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(
            row.legacy_pve_user.as_deref(),
            Some("uptrakit-stuck@pve"),
            "the pre-existing marker must survive untouched"
        );
    }

    #[tokio::test]
    async fn recovery_arm_promotes_when_marker_known() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(31);
        let host_id_str = host_id.to_string();
        // Operative id starts at a DIFFERENT value ("legacy-cfg") than the ack
        // marker ("ack-cfg") — seeding the marker onto a NULL operative
        // column would let `set_new_plugin_config_id`'s own coalesce fill
        // pre-satisfy the promotion assertion before recovery ever runs.
        db_ops::upsert_host_state(
            &db,
            &host_id_str,
            true,
            Some("legacy-cfg".to_string()),
            Some("pve1".to_string()),
        )
        .await
        .expect("seed host with legacy operative id");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some("uptrakit-old@pve".to_string()),
        )
        .await
        .expect("seed legacy marker");
        db_ops::set_new_plugin_config_id(&db, &host_id_str, "ack-cfg")
            .await
            .expect("seed ack marker");

        let tid = tenant(31);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("RECOVERYCLUSTER"), &["pve1"]);
        // This run's read succeeds and shows the legacy user already gone,
        // and (deliberately) no token either -- invalidates reuse via
        // confirmed absence, so create fires alongside the recovery arm.
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            ("pveum user list", user_list_empty()),
            ("pveum role add", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", token_add_ok("secret-j")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("legacy PVE user already gone; state reconciled")),
            "{:?}",
            out.summary_lines
        );
        assert_ne!(
            out.outcome,
            PveCredentialOutcome::MigrationPending,
            "recovery must clear the pending gate"
        );

        let row = db_ops::find_host_state(&db, &host_id_str)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.legacy_pve_user, None);
        assert_eq!(
            row.pve_plugin_config_id.as_deref(),
            Some("ack-cfg"),
            "recovery promotes to the known ack marker"
        );
        assert_eq!(row.migration_attempts, 0);
        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("pveum user delete")),
            "the recovery arm reconciles state because the user is already gone; it must \
             never itself issue a delete: {calls:?}"
        );
    }

    #[tokio::test]
    async fn regenerate_on_ack_loss() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(32);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        let tid = tenant(32);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("ACKLOSSCLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                ok(format!(r#"[{{"tokenid":"tenant-{tid}"}}]"#)),
            ),
            ("pveum user list", user_list_empty()),
            ("pveum user token remove", ok("")),
            ("pveum user token add", token_add_ok("secret-k")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert_eq!(out.outcome, PveCredentialOutcome::Regenerated);
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("PVE API token regenerated"))
        );
        assert!(
            out.report.is_some(),
            "a regenerated token must still produce a plugin-config report to push"
        );
        let calls = executor.recorded_calls();
        assert!(calls.iter().any(|c| c.contains("pveum user token remove")));
        assert!(
            !calls.iter().any(|c| c.contains("pveum role add")),
            "regenerate must not re-ensure roles: {calls:?}"
        );
    }

    #[tokio::test]
    async fn cluster_lock_serializes_concurrent_flows() {
        let db = setup_agent_db().await;
        // DB-backed test: stays on real time per the SQLx/SeaORM exception in
        // docs/development/testing.md (a paused clock auto-advances the
        // pool's acquire/idle timers and produces a spurious timeout); the
        // 20ms sleeps below are real and well under the 200ms bound.
        let timeline: Arc<parking_lot::Mutex<Vec<&'static str>>> =
            Arc::new(parking_lot::Mutex::new(Vec::new()));
        let cluster_json = cluster_status(Some("SHAREDCLUSTER"), &["pve1", "pve2"]);

        let exec_a = TimelineExecutor {
            label: "A",
            timeline: Arc::clone(&timeline),
            cluster_json: cluster_json.clone(),
        };
        let exec_b = TimelineExecutor {
            label: "B",
            timeline: Arc::clone(&timeline),
            cluster_json: cluster_json.clone(),
        };

        let host_a = uuid::Uuid::from_u128(40);
        let host_b = uuid::Uuid::from_u128(41);
        let tid_a = tenant(40);
        let tid_b = tenant(41);
        let invoker_a = RecordingActionInvoker::new();
        let invoker_b = RecordingActionInvoker::new();
        let guest_a = UnusedGuestBootstrap;
        let guest_b = UnusedGuestBootstrap;
        let tid_a_str = tid_a.to_string();
        let tid_b_str = tid_b.to_string();
        let ctx_a = make_ctx(&db, Some(&tid_a_str), &invoker_a, &guest_a);
        let ctx_b = make_ctx(&db, Some(&tid_b_str), &invoker_b, &guest_b);

        let (out_a, out_b) = tokio::join!(
            run_credential_flow(&ctx_a, &exec_a, host_a, Some("pve1")),
            run_credential_flow(&ctx_b, &exec_b, host_b, Some("pve2")),
        );

        assert_eq!(out_a.outcome, PveCredentialOutcome::Provisioned);
        assert_eq!(out_b.outcome, PveCredentialOutcome::Provisioned);

        let timeline = timeline.lock().clone();
        assert_eq!(
            timeline.len(),
            4,
            "each flow's timed section must record a start and end mark: {timeline:?}"
        );
        assert_eq!(
            timeline[0], timeline[1],
            "the first flow's start/end marks must not be interleaved with the second's: {timeline:?}"
        );
        assert_eq!(timeline[2], timeline[3]);
        assert_ne!(
            timeline[0], timeline[2],
            "the per-cluster lock must serialize the two flows, not run them concurrently: {timeline:?}"
        );
    }

    #[tokio::test]
    async fn missing_tenant_skips_with_summary() {
        let db = setup_agent_db().await;
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let ctx = make_ctx(&db, None, &invoker, &guest);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![]);
        let host_id = uuid::Uuid::from_u128(50);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;

        assert_eq!(out.outcome, PveCredentialOutcome::SkippedNoTenant);
        assert!(!out.degraded);
        assert!(out.report.is_none());
        assert!(out.existing_config_id.is_none());
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("no tenant context"))
        );
        assert!(
            executor.recorded_calls().is_empty(),
            "the no-tenant branch must never touch the remote host"
        );
    }

    #[tokio::test]
    async fn degraded_create_uses_regenerate_shape() {
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(51);
        let host_id_str = host_id.to_string();
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        let tid = tenant(51);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("DEGRADEDCREATECLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            (
                "pveum user token list",
                err(1, "permission denied: totally unrelated"),
            ),
            ("pveum user add", ok("")),
            ("pveum user token remove", ok("")),
            ("pveum user token add", token_add_ok("secret-l")),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
            ("curl", ok("200")),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;
        assert!(out.degraded);
        assert_eq!(out.outcome, PveCredentialOutcome::Provisioned);

        let calls = executor.recorded_calls();
        let add_idx = calls
            .iter()
            .position(|c| c.contains("pveum user add"))
            .expect("guarded add-user call recorded");
        let remove_idx = calls
            .iter()
            .position(|c| c.contains("pveum user token remove"))
            .expect("regenerate-shape remove recorded");
        let create_idx = calls
            .iter()
            .position(|c| c.contains("pveum user token add"))
            .expect("token add recorded");
        assert!(
            add_idx < remove_idx && remove_idx < create_idx,
            "expected order: guarded add-user, then remove, then add: {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.contains("pveum role add")),
            "degraded create must skip ensure_pve_roles: {calls:?}"
        );
    }

    #[tokio::test]
    async fn empty_cluster_nodes_defers() {
        let db = setup_agent_db().await;
        let tid = tenant(60);
        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        // No "type":"node" entries -> detection is treated as a hard failure.
        let executor =
            ScriptedRemoteExecutor::with_matcher(vec![("pvesh get /cluster/status", ok("[]"))]);
        let host_id = uuid::Uuid::from_u128(61);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;

        assert_eq!(out.outcome, PveCredentialOutcome::Failed);
        assert!(!out.degraded);
        assert!(
            out.summary_lines
                .iter()
                .any(|l| l.contains("PVE cluster detection failed"))
        );
        assert_eq!(
            executor.recorded_calls().len(),
            2,
            "only the two best-effort cluster-detection reads, nothing else: {:?}",
            executor.recorded_calls()
        );
    }

    #[tokio::test]
    async fn failed_creation_outranks_a_pending_migration() {
        // A stored legacy user makes `migration_pending` true, but this run's
        // token creation genuinely fails. Branch 10's `outcome != Failed`
        // guard must keep the real Failed outcome instead of letting the
        // merely-pending migration mask it as MigrationPending.
        let db = setup_agent_db().await;
        let host_id = uuid::Uuid::from_u128(70);
        let host_id_str = host_id.to_string();
        let tid = tenant(70);
        let legacy_name = format!("uptrakit-{tid}@pve");
        db_ops::upsert_host_state(&db, &host_id_str, true, None, Some("pve1".to_string()))
            .await
            .expect("seed host");
        db_ops::set_legacy_pve_user(
            &db,
            std::slice::from_ref(&host_id_str),
            Some(legacy_name.clone()),
        )
        .await
        .expect("seed legacy marker");

        let invoker = RecordingActionInvoker::new();
        let guest = UnusedGuestBootstrap;
        let tid_str = tid.to_string();
        let ctx = make_ctx(&db, Some(&tid_str), &invoker, &guest);
        let cluster_json = cluster_status(Some("FAILEDCREATECLUSTER"), &["pve1"]);
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pvesh get /cluster/status", ok(cluster_json)),
            ("pveum user token list", token_list_empty()),
            (
                "pveum user list",
                ok(format!(r#"[{{"userid":"{legacy_name}"}}]"#)),
            ),
            (
                "pveum user token add",
                err(1, "permission denied: token add failed"),
            ),
        ]);

        let out = run_credential_flow(&ctx, &executor, host_id, Some("pve1")).await;

        assert_eq!(
            out.outcome,
            PveCredentialOutcome::Failed,
            "a genuine creation failure must not be masked by the pending migration"
        );
        assert!(!out.degraded);
    }
}
