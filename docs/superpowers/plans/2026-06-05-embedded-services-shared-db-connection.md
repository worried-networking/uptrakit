# Embedded Services — Shared Database Connection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thread the controller's existing `DatabaseConnection` through all spawned-task call sites in
`agent-ssh-runtime`, eliminating six `db::init_db()` calls that break SSH surface actions in embedded mode.

**Architecture:** `SurfaceRuntimeContext` already carries `db: &'a sea_orm::DatabaseConnection`. Each spawn site
clones it (`ctx.db.clone()`) before entering the closure, then passes it into operations functions as
`db: &DatabaseConnection`. `load_and_validate_pve_host` and the Proxmox chain drop their `state_dir` parameter
(used only for DB init). The standard bootstrap and sync chains keep `state_dir` (needed for SSH key files and
sudoers). The embedded registration path gains three SSH crypto init calls.

**Tech Stack:** Rust / SeaORM / `sea_orm::DatabaseConnection` (cheap `Arc<Pool>` clone) / `tokio::spawn` / `controller-runtime` `builtins.rs`

---

## File Map

| File                                                                | Change                                                                                                                                                                                                             |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/core/agent-ssh-runtime/src/surface_runtime/` (directory)    | **Delete entirely** — dead code, no `mod.rs`                                                                                                                                                                       |
| `crates/core/agent-ssh-runtime/src/operations/bootstrap_proxmox.rs` | Remove `init_db`; thread `db` through Proxmox chain; change `AgentGuestBootstrapExecutor`                                                                                                                          |
| `crates/core/agent-ssh-runtime/src/operations/bootstrap.rs`         | Add `db` param to `bootstrap_connect` and `bootstrap_execute`; remove `init_db`                                                                                                                                    |
| `crates/core/agent-ssh-runtime/src/surface_runtime.rs`              | Add `BootstrapConnectArgs`; refactor `run_bootstrap_connect`; add `db` to `BootstrapExecuteArgs`; update bootstrap spawn sites (Task 3) + sync spawn sites (Task 4); `spawn_infra_plugin_action` updated in Task 2 |
| `crates/core/controller-runtime/src/service_host/builtins.rs`       | Add stale-file warn + three SSH crypto init calls in `register_agent_ssh`                                                                                                                                          |
| `CHANGELOG.md`                                                      | Note `agent-ssh.db` no longer used in embedded mode                                                                                                                                                                |
| `CONTEXT.md`                                                        | Amend **Embedded Mode** entry                                                                                                                                                                                      |
| `docs/adr/0005-service-binary-runtime-boundary.md`                  | Amend Consequences to cover spawned tasks                                                                                                                                                                          |

---

### Task 1: Delete dead `surface_runtime/` subdirectory

`lib.rs` declares `pub mod surface_runtime` which resolves to the monolithic `.rs` file —
the `surface_runtime/` directory is unreachable dead code.

**Files:**

- Delete: `crates/core/agent-ssh-runtime/src/surface_runtime/` (entire directory, 13 files)

- [ ] **Step 1: Verify directory is unreachable**

```bash
grep -r 'surface_runtime' crates/core/agent-ssh-runtime/src/lib.rs
```

Expected output: one line, `pub mod surface_runtime;` — no path hint, resolves to
`surface_runtime.rs` (the monolith), not the directory.

- [ ] **Step 2: Delete the directory**

```bash
rm -rf crates/core/agent-ssh-runtime/src/surface_runtime/
```

- [ ] **Step 3: Verify build still passes**

```bash
cargo check -p uptrakit-agent-ssh-runtime --all-features 2>&1 | head -20
```

Expected: zero errors (directory was dead code).

- [ ] **Step 4: Format and commit**

```bash
cargo fmt --all
git add -A crates/core/agent-ssh-runtime/src/surface_runtime/
git commit -m "refactor(agent-ssh-runtime): delete unreachable surface_runtime/ dead-code directory"
```

---

### Task 2: Thread `db` through the Proxmox bootstrap chain

`load_and_validate_pve_host` is the sole `init_db` call site in the Proxmox path. After
removing it, `state_dir` is no longer needed anywhere in the Proxmox chain, so it is
replaced throughout by `db: &sea_orm::DatabaseConnection`. `AgentGuestBootstrapExecutor`
replaces its `state_dir` field with `db: sea_orm::DatabaseConnection`.

**Files:**

- Modify: `crates/core/agent-ssh-runtime/src/operations/bootstrap_proxmox.rs`

- [ ] **Step 1: Change `load_and_validate_pve_host`**

Old signature (line 165):

```rust
async fn load_and_validate_pve_host(
    state_dir: &Path,
    params: &ProxmoxBootstrapParams,
) -> Result<(
    sea_orm::DatabaseConnection,
    crate::db::entity::ssh_host::Model,
)> {
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(sea_orm::DbErr::Custom(format!(
            "failed to initialize local database: {e}"
        ))))
    })?;

    let pve_host = host_ops::find_host(&db, &params.pve_host_id)
        .await?
        .ok_or_else(|| {
            report!(Error::HostNotFound(format!(
                "PVE host '{}' not found",
                params.pve_host_id
            )))
        })?;

    // Check name uniqueness.
    let existing = host_ops::find_host(&db, &params.name).await?;
    if existing.is_some() {
        bail!(Error::HostNameConflict(params.name.clone()));
    }

    Ok((db, pve_host))
}
```

New signature (replace entire function):

```rust
async fn load_and_validate_pve_host(
    db: &sea_orm::DatabaseConnection,
    params: &ProxmoxBootstrapParams,
) -> Result<crate::db::entity::ssh_host::Model> {
    let pve_host = host_ops::find_host(db, &params.pve_host_id)
        .await?
        .ok_or_else(|| {
            report!(Error::HostNotFound(format!(
                "PVE host '{}' not found",
                params.pve_host_id
            )))
        })?;

    // Check name uniqueness.
    let existing = host_ops::find_host(db, &params.name).await?;
    if existing.is_some() {
        bail!(Error::HostNameConflict(params.name.clone()));
    }

    Ok(pve_host)
}
```

- [ ] **Step 2: Change `proxmox_bootstrap_connect`**

Old signature (line 291):

```rust
pub(crate) async fn proxmox_bootstrap_connect(
    state_dir: &Path,
    params: &ProxmoxBootstrapParams,
) -> Result<ProxmoxBootstrapPlan> {
    let (_db, pve_host) = load_and_validate_pve_host(state_dir, params).await?;
```

New:

```rust
pub(crate) async fn proxmox_bootstrap_connect(
    db: &sea_orm::DatabaseConnection,
    params: &ProxmoxBootstrapParams,
) -> Result<ProxmoxBootstrapPlan> {
    let pve_host = load_and_validate_pve_host(db, params).await?;
```

- [ ] **Step 3: Change `proxmox_bootstrap_execute`**

Old signature (line 427):

```rust
pub(crate) async fn proxmox_bootstrap_execute(
    state_dir: &Path,
    params: ProxmoxBootstrapParams,
    skip_actions: &HashSet<String>,
) -> Result<ProxmoxBootstrapResult> {
    // 1. LOAD PVE HOST
    let (db, pve_host) = load_and_validate_pve_host(state_dir, &params).await?;
```

New:

```rust
pub(crate) async fn proxmox_bootstrap_execute(
    db: &sea_orm::DatabaseConnection,
    params: ProxmoxBootstrapParams,
    skip_actions: &HashSet<String>,
) -> Result<ProxmoxBootstrapResult> {
    // 1. LOAD PVE HOST
    let pve_host = load_and_validate_pve_host(db, &params).await?;
```

After the signature change, `db` is `&sea_orm::DatabaseConnection` (a reference parameter). Every
internal call site that previously passed `&db` (borrowing the locally-owned result of `init_db`)
must now pass `db` directly (since `db` is already a `&`). Apply `&db` → `db` throughout the
function body — example:

```rust
// Before (db was owned, so &db was a borrow):
host_ops::add_host(&db, ...).await?;
// After (db is already &DatabaseConnection):
host_ops::add_host(db, ...).await?;
```

Search for every `&db` occurrence inside `proxmox_bootstrap_execute` and apply the same
substitution. The function body otherwise remains unchanged.

- [ ] **Step 4: Change `run_proxmox_bootstrap`**

Old (line 152):

```rust
pub(crate) async fn run_proxmox_bootstrap(
    state_dir: &Path,
    params: ProxmoxBootstrapParams,
) -> Result<ProxmoxBootstrapResult> {
    let _plan = proxmox_bootstrap_connect(state_dir, &params).await?;
    proxmox_bootstrap_execute(state_dir, params, &HashSet::new()).await
}
```

New:

```rust
pub(crate) async fn run_proxmox_bootstrap(
    db: &sea_orm::DatabaseConnection,
    params: ProxmoxBootstrapParams,
) -> Result<ProxmoxBootstrapResult> {
    let _plan = proxmox_bootstrap_connect(db, &params).await?;
    proxmox_bootstrap_execute(db, params, &HashSet::new()).await
}
```

- [ ] **Step 5: Change `AgentGuestBootstrapExecutor`**

Old struct and impl (line 103):

```rust
pub(crate) struct AgentGuestBootstrapExecutor {
    pub state_dir: std::path::PathBuf,
    pub service_id: Option<uuid::Uuid>,
}

#[async_trait]
impl GuestBootstrapExecutor for AgentGuestBootstrapExecutor {
    async fn bootstrap_guest(
        &self,
        params: uptrakit_plugin_infrastructure_registry::agent_infra::GuestBootstrapParams,
    ) -> std::result::Result<
        uptrakit_plugin_infrastructure_registry::agent_infra::GuestBootstrapResult,
        GuestBootstrapError,
    > {
        let proxmox_params = ProxmoxBootstrapParams {
            // ... fields including self.service_id ...
            service_id: params.service_id.or(self.service_id),
        };

        run_proxmox_bootstrap(&self.state_dir, proxmox_params)
            .await
```

New (replace `state_dir` field with `db`, update `bootstrap_guest` call):

```rust
pub(crate) struct AgentGuestBootstrapExecutor {
    pub db: sea_orm::DatabaseConnection,
    pub service_id: Option<uuid::Uuid>,
}

#[async_trait]
impl GuestBootstrapExecutor for AgentGuestBootstrapExecutor {
    async fn bootstrap_guest(
        &self,
        params: uptrakit_plugin_infrastructure_registry::agent_infra::GuestBootstrapParams,
    ) -> std::result::Result<
        uptrakit_plugin_infrastructure_registry::agent_infra::GuestBootstrapResult,
        GuestBootstrapError,
    > {
        let proxmox_params = ProxmoxBootstrapParams {
            // ... fields including self.service_id — unchanged ...
            service_id: params.service_id.or(self.service_id),
        };

        run_proxmox_bootstrap(&self.db, proxmox_params)
            .await
```

- [ ] **Step 6: Update `spawn_infra_plugin_action` in `surface_runtime.rs` to use the new `db` field**

`AgentGuestBootstrapExecutor` just lost its `state_dir` field (Step 5), so `surface_runtime.rs`
must be updated in the same commit to keep the workspace compile-clean.

Old block inside `spawn_infra_plugin_action` (line 1035):

```rust
fn spawn_infra_plugin_action(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let proxy = std::sync::Arc::clone(ctx.surface_proxy);
    let infra_bundles = std::sync::Arc::clone(&ctx.infra_bundles);
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());

    tokio::spawn(async move {
        let db = match crate::db::init_db(&state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request.request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let tenant_id_str = tenant_id.map(|t| t.to_string());
        let action_invoker = InfraActionInvokerImpl::new(&proxy, &bg_tx, tenant_id);
        let guest_bootstrap = AgentGuestBootstrapExecutor {
            state_dir: state_dir.clone(),
            service_id,
        };
```

New (add `let db = ctx.db.clone()` before spawn, remove `init_db` block, use new `db` field):

```rust
fn spawn_infra_plugin_action(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let db = ctx.db.clone();
    let bg_tx = ctx.bg_tx.clone();
    let proxy = std::sync::Arc::clone(ctx.surface_proxy);
    let infra_bundles = std::sync::Arc::clone(&ctx.infra_bundles);
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());

    tokio::spawn(async move {
        let tenant_id_str = tenant_id.map(|t| t.to_string());
        let action_invoker = InfraActionInvokerImpl::new(&proxy, &bg_tx, tenant_id);
        let guest_bootstrap = AgentGuestBootstrapExecutor {
            db: db.clone(),
            service_id,
        };
```

`state_dir` stays — it is still used in `InfraPluginContext { ..., state_dir: &state_dir, ... }`
later in the closure. The `InfraPluginContext { db: &db, ... }` line stays unchanged.

- [ ] **Step 7: Verify compilation is clean**

```bash
cargo check -p uptrakit-agent-ssh-runtime --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: zero errors. `bootstrap_proxmox.rs` and `surface_runtime.rs` are both updated
in this task, so the workspace is compile-clean at commit time.

- [ ] **Step 8: Format and commit**

```bash
cargo fmt --all
git add crates/core/agent-ssh-runtime/src/operations/bootstrap_proxmox.rs \
        crates/core/agent-ssh-runtime/src/surface_runtime.rs
git commit -m "refactor(agent-ssh-runtime): thread db through Proxmox bootstrap chain, drop init_db"
```

---

### Task 3: Thread `db` through standard bootstrap operations

`bootstrap_connect` and `bootstrap_execute` in `bootstrap.rs` each call `init_db` internally.
Both functions also use `state_dir` for SSH key files and sudoers — `state_dir` is kept,
`db` is added as the first parameter.

`run_bootstrap_connect` in `surface_runtime.rs` currently has 7 parameters; adding `db` would
exceed clippy's 7-arg limit (workspace `warnings=deny`). Mirror the existing `BootstrapExecuteArgs`
pattern by introducing `BootstrapConnectArgs`.

**Files:**

- Modify: `crates/core/agent-ssh-runtime/src/operations/bootstrap.rs`
- Modify: `crates/core/agent-ssh-runtime/src/surface_runtime.rs`

- [ ] **Step 1: Update `bootstrap_connect` in `bootstrap.rs`**

Old signature (line 202):

```rust
pub(crate) async fn bootstrap_connect(
    state_dir: &Path,
    params: &BootstrapParams,
) -> Result<BootstrapPlan> {
    // 1. VALIDATE INPUTS
    validate_bootstrap_inputs(params)?;

    // Fail fast: check host name is not in DB.
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(sea_orm::DbErr::Custom(format!(
            "failed to initialize local database: {e}"
        ))))
    })?;
    let existing = host_ops::find_host(&db, &params.name).await?;
```

New (add `db` as first param, remove `init_db` block, update `find_host` and `gather_remote_host_info` call):

```rust
pub(crate) async fn bootstrap_connect(
    db: &sea_orm::DatabaseConnection,
    state_dir: &Path,
    params: &BootstrapParams,
) -> Result<BootstrapPlan> {
    // 1. VALIDATE INPUTS
    validate_bootstrap_inputs(params)?;

    // Fail fast: check host name is not in DB.
    let existing = host_ops::find_host(db, &params.name).await?;
```

Also update the `gather_remote_host_info` call later in the same function (line 268):

Old:

```rust
    let remote_info =
        gather_remote_host_info(&session, &executor, params, use_sudo, state_dir, &db).await?;
```

New:

```rust
    let remote_info =
        gather_remote_host_info(&session, &executor, params, use_sudo, state_dir, db).await?;
```

- [ ] **Step 2: Update `bootstrap_execute` in `bootstrap.rs`**

Old signature (line ~534):

```rust
pub(crate) async fn bootstrap_execute(
    state_dir: &Path,
    params: BootstrapParams,
    skip_actions: &HashSet<String>,
) -> Result<BootstrapResult> {
```

Somewhere inside (line 558):

```rust
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(sea_orm::DbErr::Custom(format!(
            "failed to initialize local database: {e}"
        ))))
    })?;
```

New signature:

```rust
pub(crate) async fn bootstrap_execute(
    db: &sea_orm::DatabaseConnection,
    state_dir: &Path,
    params: BootstrapParams,
    skip_actions: &HashSet<String>,
) -> Result<BootstrapResult> {
```

Remove the `let db = crate::db::init_db(...)` block. All subsequent uses of `db` in the
function body (`execute_bootstrap_routeros(&ros_params, session, &db)` and others) now
refer to the parameter. Change `&db` → `db` for those call sites (since `db` is already `&`).

Specifically, find every call site in `bootstrap_execute` that passes `&db` and change to `db`:

```rust
// Before:
execute_bootstrap_routeros(&ros_params, session, &db).await?;
// After:
execute_bootstrap_routeros(&ros_params, session, db).await?;
```

And any other `&db` → `db` references inside `bootstrap_execute`.

- [ ] **Step 3: Add `BootstrapConnectArgs` struct in `surface_runtime.rs`**

Insert the following struct **immediately before** the existing `BootstrapExecuteArgs` struct
(around line 1718):

```rust
/// Arguments for the bootstrap-connect handler, bundled to stay within the 7-arg clippy limit.
struct BootstrapConnectArgs<'a> {
    request_id: uuid::Uuid,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    db: sea_orm::DatabaseConnection,
}
```

- [ ] **Step 4: Refactor `run_bootstrap_connect` in `surface_runtime.rs`**

Old function (line 1679):

```rust
/// The bootstrap-connect handler: probe the host and return a plan.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_connect(
    request_id: uuid::Uuid,
    params: &serde_json::Value,
    sensitive_params_sealed: Option<&str>,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &Path,
) -> SurfaceActionResponse {
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            sensitive_params_sealed,
            private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    let bootstrap_params =
        match parse_bootstrap_params(params, sensitive.as_ref(), service_id, tenant_id) {
            Ok(p) => p,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    match bootstrap::bootstrap_connect(state_dir, &bootstrap_params).await {
        Ok(plan) => match serde_json::to_value(&plan) {
            Ok(data) => make_surface_success_response(request_id, data),
            Err(e) => {
                make_surface_error_response(request_id, &format!("failed to serialize plan: {e}"))
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "bootstrap-connect failed");
            make_surface_error_response(request_id, &format!("bootstrap connect failed: {e}"))
        }
    }
}
```

New:

```rust
/// The bootstrap-connect handler: probe the host and return a plan.
#[tracing::instrument(skip_all, fields(request_id = %args.request_id))]
async fn run_bootstrap_connect(args: BootstrapConnectArgs<'_>) -> SurfaceActionResponse {
    let request_id = args.request_id;
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            args.sensitive_params_sealed,
            args.private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    let bootstrap_params =
        match parse_bootstrap_params(args.params, sensitive.as_ref(), args.service_id, args.tenant_id) {
            Ok(p) => p,
            Err(msg) => return make_surface_error_response(request_id, &msg),
        };

    match bootstrap::bootstrap_connect(&args.db, args.state_dir, &bootstrap_params).await {
        Ok(plan) => match serde_json::to_value(&plan) {
            Ok(data) => make_surface_success_response(request_id, data),
            Err(e) => {
                make_surface_error_response(request_id, &format!("failed to serialize plan: {e}"))
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "bootstrap-connect failed");
            make_surface_error_response(request_id, &format!("bootstrap connect failed: {e}"))
        }
    }
}
```

- [ ] **Step 5: Add `db` field to `BootstrapExecuteArgs` in `surface_runtime.rs`**

Old struct (line 1719):

```rust
struct BootstrapExecuteArgs<'a> {
    request_id: uuid::Uuid,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
}
```

New (add `db` field):

```rust
struct BootstrapExecuteArgs<'a> {
    request_id: uuid::Uuid,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    db: sea_orm::DatabaseConnection,
}
```

- [ ] **Step 6: Update `run_bootstrap_execute` to pass `db`**

Old call inside `run_bootstrap_execute` (line ~1757):

```rust
    match bootstrap::bootstrap_execute(args.state_dir, bootstrap_params, &skip_actions).await {
```

New:

```rust
    match bootstrap::bootstrap_execute(&args.db, args.state_dir, bootstrap_params, &skip_actions).await {
```

- [ ] **Step 7: Update `spawn_bootstrap_connect` to supply `db`**

Old `spawn_bootstrap_connect` (line 1288):

```rust
fn spawn_bootstrap_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_connect(
            request_id,
            &params,
            sensitive_params_sealed.as_deref(),
            private_key_der.as_deref(),
            service_id,
            tenant_id,
            &state_dir,
        )
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-connect result via bg_tx");
        }
    });
}
```

New:

```rust
fn spawn_bootstrap_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let db = ctx.db.clone();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_connect(BootstrapConnectArgs {
            request_id,
            params: &params,
            sensitive_params_sealed: sensitive_params_sealed.as_deref(),
            private_key_der: private_key_der.as_deref(),
            service_id,
            tenant_id,
            state_dir: &state_dir,
            db,
        })
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-connect result via bg_tx");
        }
    });
}
```

- [ ] **Step 8: Update `spawn_bootstrap_execute` to supply `db`**

Replace the entire function (line 1319). The complete new version adds
`let db = ctx.db.clone()` before the spawn and adds `db` to the struct literal.
The `emit_surface_mutation_audit` call between `run_bootstrap_execute` and the
final send **must be preserved** — it is not shown in the old snippet but exists
in the source at lines 1343–1351:

```rust
fn spawn_bootstrap_execute(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let db = ctx.db.clone();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let response = run_bootstrap_execute(BootstrapExecuteArgs {
            request_id,
            params: &params,
            sensitive_params_sealed: sensitive_params_sealed.as_deref(),
            private_key_der: private_key_der.as_deref(),
            service_id,
            tenant_id,
            state_dir: &state_dir,
            bg_tx: &bg_tx,
            db,
        })
        .await;
        emit_surface_mutation_audit(
            &bg_tx,
            tenant_id,
            "bootstrap-execute",
            request_id,
            &params,
            &response,
        )
        .await;
        let msg = ServiceMessage::SurfaceActionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-execute result via bg_tx");
        }
    });
}
```

- [ ] **Step 9: Verify compilation**

```bash
cargo check -p uptrakit-agent-ssh-runtime --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: zero errors. `spawn_infra_plugin_action` was already updated in Task 2 Step 6
to use the new `AgentGuestBootstrapExecutor.db` field, so no residual breakage remains.

- [ ] **Step 10: Format and commit**

```bash
cargo fmt --all
git add crates/core/agent-ssh-runtime/src/operations/bootstrap.rs \
        crates/core/agent-ssh-runtime/src/surface_runtime.rs
git commit -m "refactor(agent-ssh-runtime): thread db through bootstrap operations, add BootstrapConnectArgs"
```

---

### Task 4: Replace `init_db` in sync spawn sites

`spawn_infra_plugin_action` was already updated in Task 2 (to keep that commit compile-clean).
Two sync spawn functions remain.

**Files:**

- Modify: `crates/core/agent-ssh-runtime/src/surface_runtime.rs`

- [ ] **Step 1: Fix `spawn_sync_connect`** (verify `spawn_infra_plugin_action` already shows `db: db.clone()` from Task 2)

`db_state_dir` was only used for `init_db`. Remove it and the `init_db` block; clone `ctx.db`
instead.

Old (line 1364):

```rust
fn spawn_sync_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let Some((host_id, auth_override)) = resolve_sync_auth(
            &params,
            sensitive_params_sealed.as_deref(),
            request_id,
            private_key_der.as_deref(),
            &bg_tx,
        )
        .await
        else {
            return;
        };

        let allow_all = param_bool(&params, "allow_all");

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let response =
            match sync::sync_connect(&host_id, &db, tenant_id, auth_override.as_ref(), allow_all)
```

New (remove `db_state_dir`, add `let db = ctx.db.clone()`, remove `init_db` block):

```rust
fn spawn_sync_connect(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let db = ctx.db.clone();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id;
    let params = serde_json::Value::Object(request.params);
    let sensitive_params_sealed = request
        .encrypted_sensitive_params
        .map(|value| value.ciphertext_b64);

    tokio::spawn(async move {
        let Some((host_id, auth_override)) = resolve_sync_auth(
            &params,
            sensitive_params_sealed.as_deref(),
            request_id,
            private_key_der.as_deref(),
            &bg_tx,
        )
        .await
        else {
            return;
        };

        let allow_all = param_bool(&params, "allow_all");

        let response =
            match sync::sync_connect(&host_id, &db, tenant_id, auth_override.as_ref(), allow_all)
```

- [ ] **Step 2: Fix `spawn_sync_execute`**

Same pattern as `spawn_sync_connect`. `db_state_dir` only used for `init_db`.

Old (line 1428):

```rust
fn spawn_sync_execute(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    // ...
    tokio::spawn(async move {
        // ...
        let allow_all = param_bool(&params, "allow_all");
        let skip_actions = parse_skip_actions(&params);

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_surface_error_response(
                    request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx
                    .send(ServiceMessage::SurfaceActionResponse(resp))
                    .await;
                return;
            }
        };

        let response = match sync::sync_execute(
            &host_id,
            &db,
```

New:

```rust
fn spawn_sync_execute(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>) {
    let db = ctx.db.clone();
    let bg_tx = ctx.bg_tx.clone();
    // ... (all other let bindings unchanged except db_state_dir removed)
    tokio::spawn(async move {
        // ...
        let allow_all = param_bool(&params, "allow_all");
        let skip_actions = parse_skip_actions(&params);

        let response = match sync::sync_execute(
            &host_id,
            &db,
```

- [ ] **Step 3: Verify full compilation**

```bash
cargo check -p uptrakit-agent-ssh-runtime --all-features 2>&1 | grep -E "^error"
cargo check -p uptrakit-controller-runtime --all-features 2>&1 | grep -E "^error"
```

Expected: zero errors.

- [ ] **Step 4: Run unit tests**

```bash
cargo test -p uptrakit-agent-ssh-runtime --all-features 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --all
git add crates/core/agent-ssh-runtime/src/surface_runtime.rs
git commit -m "refactor(agent-ssh-runtime): replace init_db in sync spawn closures with ctx.db.clone()"
```

---

### Task 5: Add SSH crypto init in embedded registration

`register_agent_ssh` in `builtins.rs` is the embedded entry point for Agent-SSH. Two SSH
crypto steps (`register_ssh_column_aad`, `reencrypt_ssh_to_v3`) are genuinely missing in the
embedded path and must run before `AgentSshHandler::new`. `init_ssh_data_key_ring` is
idempotent (Phase 4c already ran it; it will emit a harmless `warn!` for "already
initialized") — included for symmetry. A stale-file warning for legacy `agent-ssh.db` is
added before the crypto calls.

**Files:**

- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`

- [ ] **Step 1: Add stale-file warning and three crypto init calls**

Current `register_agent_ssh` body (line 321):

```rust
    let ssh_caps = crate::ssh_agent::ssh_agent_capabilities();
    let default_tenant_id = app_state.default_tenant_id;
    let db_for_ssh = app_state.db().clone();

    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(db_for_ssh, state_dir);
```

New:

```rust
    // Warn about legacy standalone DB file — no longer used in embedded mode.
    // Use tokio::fs::metadata (non-blocking) — register_agent_ssh is async.
    let ssh_db_path = state_dir.join("agent-ssh.db");
    if let Ok(meta) = tokio::fs::metadata(&ssh_db_path).await {
        if meta.len() > 0 {
            tracing::warn!(
                path = %ssh_db_path.display(),
                "legacy agent-ssh.db found in state directory; \
                 this file is no longer used in embedded mode — \
                 SSH host data must be migrated manually if needed \
                 (see agent-ssh-runtime/src/db/entity/ for table schemas)"
            );
        }
    }

    uptrakit_agent_ssh_runtime::register_ssh_column_aad();
    uptrakit_agent_ssh_runtime::init_ssh_data_key_ring(app_state.db()).await;
    uptrakit_agent_ssh_runtime::reencrypt_ssh_to_v3(app_state.db()).await;

    let ssh_caps = crate::ssh_agent::ssh_agent_capabilities();
    let default_tenant_id = app_state.default_tenant_id;
    let db_for_ssh = app_state.db().clone();

    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(db_for_ssh, state_dir);
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p uptrakit-controller-runtime --all-features 2>&1 | grep -E "^error"
```

Expected: zero errors. `register_ssh_column_aad`, `init_ssh_data_key_ring`, and
`reencrypt_ssh_to_v3` are already `pub` in `agent-ssh-runtime/src/lib.rs`.

- [ ] **Step 3: Run full quality gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

All must pass. Fix any warnings before continuing.

- [ ] **Step 4: Commit**

```bash
git add crates/core/controller-runtime/src/service_host/builtins.rs
git commit -m "fix(controller-runtime): add SSH crypto init + stale-db warn in register_agent_ssh"
```

---

### Task 6: Documentation

Three docs need updating per the spec's Documentation section.

**Files:**

- Modify: `CHANGELOG.md`
- Modify: `CONTEXT.md`
- Modify: `docs/adr/0005-service-binary-runtime-boundary.md`

- [ ] **Step 1: Add CHANGELOG entry**

In `CHANGELOG.md` under the `## Unreleased` section, add:

```markdown
### Changed

- Embedded Agent-SSH now shares the controller's database connection. The
  `agent-ssh.db` file in the controller state directory is no longer created or
  used. SSH host data stored there (rare — this path was previously broken) must
  be migrated manually using the entity schemas in
  `crates/core/agent-ssh-runtime/src/db/entity/`.
```

- [ ] **Step 2: Update `CONTEXT.md` Embedded Mode entry**

Current entry (search for `**Embedded Mode**:`):

```markdown
**Embedded Mode**:
A deployment configuration (built via the `controller-standalone` crate) where some or all
Services run inside the Controller binary. Embedded Services are still displayed as separate
Services but marked "embedded."
_Avoid_: standalone (ambiguous), monolith
```

New:

```markdown
**Embedded Mode**:
A deployment configuration (built via the `controller-standalone` crate) where some or all
Services run inside the Controller binary. Embedded Services are still displayed as separate
Services but marked "embedded." Embedded Services share the controller's `DatabaseConnection`
rather than opening their own — no per-service DB files are created in embedded mode.
_Avoid_: standalone (ambiguous), monolith
```

- [ ] **Step 3: Amend ADR-0005 Consequences**

In `docs/adr/0005-service-binary-runtime-boundary.md`, locate the **Negative** bullet under
`## Consequences`:

```markdown
- Each embedded service's `ServiceHandler` must be constructible with controller-provided deps
  (DB connection, state dir, ECIES keypair). Handler constructors must not hardcode internal
  paths or open their own DB connections.
```

Append a second sentence:

```markdown
- Each embedded service's `ServiceHandler` must be constructible with controller-provided deps
  (DB connection, state dir, ECIES keypair). Handler constructors must not hardcode internal
  paths or open their own DB connections. This invariant extends to all tasks spawned by the
  handler at runtime — background tasks and surface-action handlers must thread the injected
  connection rather than calling `init_db` or equivalent.
```

- [ ] **Step 4: Lint markdown**

```bash
npx prettier --write CHANGELOG.md CONTEXT.md docs/adr/0005-service-binary-runtime-boundary.md
npx markdownlint --config .markdownlint.json CHANGELOG.md CONTEXT.md docs/adr/0005-service-binary-runtime-boundary.md
```

Expected: zero lint errors.

- [ ] **Step 5: Commit**

```bash
git add CHANGELOG.md CONTEXT.md docs/adr/0005-service-binary-runtime-boundary.md
git commit -m "docs: update CONTEXT.md, ADR-0005, CHANGELOG for embedded shared-db change"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                                                                         | Task                                      |
| -------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| Delete dead `surface_runtime/` directory                                                                 | Task 1                                    |
| `load_and_validate_pve_host` return type → `ssh_host::Model`                                             | Task 2                                    |
| Proxmox chain (`proxmox_bootstrap_connect/execute`, `run_proxmox_bootstrap`) drop `state_dir`, gain `db` | Task 2                                    |
| `AgentGuestBootstrapExecutor.state_dir` → `db`                                                           | Task 2                                    |
| `bootstrap_connect` / `bootstrap_execute` add `db` param, remove `init_db`                               | Task 3                                    |
| `BootstrapConnectArgs` struct (7-arg clippy limit)                                                       | Task 3                                    |
| `run_bootstrap_connect` → takes `BootstrapConnectArgs`                                                   | Task 3                                    |
| `BootstrapExecuteArgs` gains `db` field                                                                  | Task 3                                    |
| `spawn_bootstrap_connect/execute` clone `ctx.db`                                                         | Task 3                                    |
| `spawn_infra_plugin_action` remove `init_db`, use `ctx.db`                                               | Task 2 (Step 6, for compile-clean commit) |
| `spawn_sync_connect/execute` remove `init_db`, use `ctx.db`                                              | Task 4                                    |
| `register_ssh_column_aad()` call in `register_agent_ssh`                                                 | Task 5                                    |
| `init_ssh_data_key_ring()` call in `register_agent_ssh`                                                  | Task 5                                    |
| `reencrypt_ssh_to_v3()` call in `register_agent_ssh`                                                     | Task 5                                    |
| Stale `agent-ssh.db` file `tracing::warn!`                                                               | Task 5                                    |
| CHANGELOG entry                                                                                          | Task 6                                    |
| `CONTEXT.md` Embedded Mode amendment                                                                     | Task 6                                    |
| ADR-0005 Consequences amendment                                                                          | Task 6                                    |

All spec requirements covered. No gaps found.
