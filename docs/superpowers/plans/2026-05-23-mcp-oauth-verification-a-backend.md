# MCP OAuth Verification — Plan A: Backend Prerequisites

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the OAuth boot path so `OAuthState` is live in production, and implement the
auto-enable resolver so `mcp_enabled` defaults to `true` when `canonical_host` is set.

**Architecture:** `resolve_mcp_enabled` is a pure function in `web-api/src/oauth/mod.rs`.
`boot_oauth_state` lives in `web-api/src/oauth/boot.rs` and is called from `controller-runtime`
startup. A single `BEGIN IMMEDIATE` transaction covers the signing-secret read-or-generate and the
peer-registration check. `validate_and_register` is widened from `&DatabaseConnection` to
`&impl ConnectionTrait` so the outer transaction handle can be threaded through.

**Tech Stack:** Rust 2024, SeaORM 1.x (`ConnectionTrait`, `TransactionTrait`), rootcause, rand 0.9,
hex 0.4, sha2, time.

---

## File Map

| Action | Path                                                               |
| ------ | ------------------------------------------------------------------ |
| Modify | `crates/ui/web-api/src/oauth/mod.rs`                               |
| Modify | `crates/ui/web-api/src/oauth/boot.rs`                              |
| Modify | `crates/ui/web-api/src/integration_tests/oauth_boot_validation.rs` |
| Modify | `crates/ui/web-api/src/routes/settings_oauth.rs`                   |
| Modify | `crates/core/controller-runtime/src/startup/oauth.rs`              |
| Modify | `crates/core/controller-runtime/src/lib.rs`                        |

---

## Task 1: `resolve_mcp_enabled` — pure resolver + unit tests

**Files:**

- Modify: `crates/ui/web-api/src/oauth/mod.rs`

- [ ] **Step 1: Write failing test in `oauth/mod.rs`**

Add at the bottom of `crates/ui/web-api/src/oauth/mod.rs` (inside the file, after
`impl OAuthState`):

```rust
/// Resolve whether MCP OAuth is enabled, applying the auto-enable rule:
/// when no explicit row exists and `canonical_host` is set, treat it as enabled.
pub fn resolve_mcp_enabled(explicit: Option<bool>, canonical_host: Option<&str>) -> bool {
    match explicit {
        Some(v) => v,
        None => canonical_host.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_mcp_enabled_all_table_rows() {
        // Row absent, no host → false
        assert!(!resolve_mcp_enabled(None, None));
        // Row absent, host set → true (auto-enable)
        assert!(resolve_mcp_enabled(None, Some("example.com")));
        // Explicit false, host set → false (operator override wins)
        assert!(!resolve_mcp_enabled(Some(false), Some("example.com")));
        // Explicit false, no host → false
        assert!(!resolve_mcp_enabled(Some(false), None));
        // Explicit true, host set → true
        assert!(resolve_mcp_enabled(Some(true), Some("example.com")));
        // Explicit true, no host → true (CanonicalHostMissing fires during boot)
        assert!(resolve_mcp_enabled(Some(true), None));
    }
}
```

- [ ] **Step 2: Run tests to confirm they pass (function is inline)**

```bash
cargo test --all-features -p uptrakit-web-api -- resolve_mcp_enabled 2>&1 | tail -5
```

Expected: `test oauth::tests::resolve_mcp_enabled_all_table_rows ... ok`

- [ ] **Step 3: Commit**

```bash
git add crates/ui/web-api/src/oauth/mod.rs
git commit -m "feat(oauth): add resolve_mcp_enabled pure resolver with auto-enable logic"
```

---

## Task 2: Widen `validate_and_register` to `&impl ConnectionTrait`

**Files:**

- Modify: `crates/ui/web-api/src/oauth/boot.rs`

Context: `validate_and_register` currently opens its own `BEGIN IMMEDIATE` transaction internally.
We need to remove that so `boot_oauth_state` (Task 3) can pass an already-open transaction, making
the secret-write and peer-check atomic.

- [ ] **Step 1: Add `ConnectionTrait` import and `Settings(String)` error variant**

In `crates/ui/web-api/src/oauth/boot.rs`, change the `sea_orm` import line:

```rust
// Before:
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

// After:
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
```

Add `std::sync::Arc` import (needed for `boot_oauth_state` in Task 3 — add now to avoid churn):

```rust
use std::sync::Arc;
```

Add a `Settings(String)` variant to `OAuthBootError`:

```rust
pub enum OAuthBootError {
    #[error("oauth.canonical_host is required when oauth.mcp_enabled is true")]
    CanonicalHostMissing,
    #[error("another controller instance is active with a different signing-secret fingerprint")]
    PeerWithDifferentFingerprint,
    #[error(
        "another controller instance is active with the same fingerprint; \
         set oauth.allow_multi_controller_unsafe = true to permit"
    )]
    PeerWithSameFingerprintNotPermitted,
    #[error("database error")]
    Database(sea_orm::DbErr),
    #[error("OAuth settings error: {0}")]
    Settings(String),
}
```

- [ ] **Step 2: Change `validate_and_register` signature and remove internal transaction**

Replace the current `validate_and_register` function signature and body. The key changes:

1. Change `db: &DatabaseConnection` → `db: &impl ConnectionTrait`
2. Remove lines that open the transaction (`db.begin_with_options(...)`) and get `txn`
3. Change all `&txn` references in query calls to `db`
4. Remove `txn.commit().await.context_to()?`

After edit, the function must look like this (full replacement):

```rust
pub async fn validate_and_register(
    db: &impl ConnectionTrait,
    settings: &OAuthBootSettings,
    now: OffsetDateTime,
) -> Result<Uuid> {
    // 1. canonical_host must be set and non-empty.
    if settings.canonical_host.as_deref().unwrap_or("").is_empty() {
        bail!(OAuthBootError::CanonicalHostMissing);
    }

    // 2. Compute fingerprint.
    let fp = fingerprint(&settings.jwt_signing_secret);
    let instance_id = Uuid::now_v7();

    // 3. Prune rows older than 24 h.
    let stale_cutoff = now - time::Duration::hours(STALE_TTL_HOURS);
    oauth_controller_instance::Entity::delete_many()
        .filter(oauth_controller_instance::Column::LastSeenAt.lt(stale_cutoff))
        .exec(db)
        .await
        .context_to()?;

    // 4. Scan for active rows (last_seen_at within the last 90 s).
    let fresh_cutoff = now - time::Duration::seconds(HEARTBEAT_FRESH_SECONDS);
    let peers = oauth_controller_instance::Entity::find()
        .filter(oauth_controller_instance::Column::LastSeenAt.gte(fresh_cutoff))
        .all(db)
        .await
        .context_to()?;

    // 5. Check peers.
    for peer in &peers {
        if peer.jwt_secret_fingerprint != fp {
            bail!(OAuthBootError::PeerWithDifferentFingerprint);
        }
        if !settings.allow_multi_controller_unsafe {
            bail!(OAuthBootError::PeerWithSameFingerprintNotPermitted);
        }
        tracing::warn!(
            peer_instance_id = %peer.instance_id,
            "multi-controller unsafe mode: peer with same fingerprint active"
        );
    }

    // 6. INSERT this controller's row.
    oauth_controller_instance::ActiveModel {
        instance_id: Set(instance_id),
        jwt_secret_fingerprint: Set(fp),
        started_at: Set(now),
        last_seen_at: Set(now),
    }
    .insert(db)
    .await
    .context_to()?;

    Ok(instance_id)
}
```

- [ ] **Step 3: Verify existing tests still compile and pass**

The tests in `boot.rs` pass `&db` (a `DatabaseConnection`) which still satisfies
`&impl ConnectionTrait`. No test changes needed.

```bash
cargo test --all-features -p uptrakit-web-api -- boot:: 2>&1 | tail -10
```

Expected: all `boot::tests::*` tests pass.

Also verify `oauth_boot_validation.rs` tests still pass (they also pass `&db`):

```bash
cargo test --all-features -p uptrakit-web-api -- oauth_boot_validation 2>&1 | tail -10
```

Expected: all three tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/oauth/boot.rs
git commit -m "refactor(oauth): widen validate_and_register to &impl ConnectionTrait, remove internal tx"
```

---

## Task 3: Add `boot_oauth_state` + extend `oauth_boot_validation.rs`

**Files:**

- Modify: `crates/ui/web-api/src/oauth/boot.rs`
- Modify: `crates/ui/web-api/src/integration_tests/oauth_boot_validation.rs`

- [ ] **Step 1: Write the failing test in `oauth_boot_validation.rs`**

Add at the bottom of `crates/ui/web-api/src/integration_tests/oauth_boot_validation.rs`:

```rust
use crate::oauth::boot::boot_oauth_state;
use crate::settings_store::upsert_global_setting_raw;

/// When `canonical_host` is set and no `mcp_enabled` row exists,
/// `boot_oauth_state` must return a live (enabled) `OAuthState`.
#[tokio::test]
async fn boot_oauth_state_auto_enables_when_canonical_host_set() {
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    upsert_global_setting_raw(&db, "oauth.canonical_host", serde_json::json!("example.com"))
        .await
        .expect("write canonical_host");
    // No mcp_enabled row — auto-enable should fire.
    let state = boot_oauth_state(&db)
        .await
        .expect("boot_oauth_state must succeed");
    assert!(state.enabled, "OAuthState must be enabled");
    assert!(!state.instance_id.is_nil(), "instance_id must not be nil");
}

/// When no `canonical_host` is set and no `mcp_enabled` row exists,
/// `boot_oauth_state` returns a disabled `OAuthState`.
#[tokio::test]
async fn boot_oauth_state_disabled_when_no_host_and_no_explicit_flag() {
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    let state = boot_oauth_state(&db)
        .await
        .expect("boot_oauth_state must succeed");
    assert!(!state.enabled, "OAuthState must be disabled");
}
```

- [ ] **Step 2: Run tests to confirm they fail (function not yet defined)**

```bash
cargo test --all-features -p uptrakit-web-api -- boot_oauth_state 2>&1 | tail -5
```

Expected: compile error — `boot_oauth_state` not found.

- [ ] **Step 3: Add `boot_oauth_state` to `boot.rs`**

Add the following after the `spawn_heartbeat` function in `crates/ui/web-api/src/oauth/boot.rs`:

```rust
/// Boot-time OAuth state initialisation.
///
/// Reads the DB to resolve whether MCP OAuth should be enabled, then (when
/// enabled) generates or reloads the JWT signing secret and registers this
/// controller instance — all within a single `BEGIN IMMEDIATE` transaction
/// to prevent split-brain on rapid restart loops.
///
/// Returns `OAuthState::disabled()` fast-path when resolved-enabled is false.
pub async fn boot_oauth_state(
    db: &DatabaseConnection,
) -> Result<super::OAuthState> {
    use rand::Rng;

    // ── Step 1: Read resolve inputs (no transaction needed — reads only) ──
    // Propagate DB errors: a transient failure here must crash the boot, not silently
    // default to disabled and let the controller run without OAuth.
    let mcp_raw: Option<bool> =
        crate::settings_store::load_global_setting_raw(db, "oauth.mcp_enabled")
            .await
            .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?
            .and_then(|v| v.as_bool());
    let canonical_host_str: Option<String> =
        crate::settings_store::load_global_setting_raw(db, "oauth.canonical_host")
            .await
            .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?
            .and_then(|v| v.as_str().map(ToOwned::to_owned));

    // ── Step 2: Resolve → fast-path disabled ──
    if !super::resolve_mcp_enabled(mcp_raw, canonical_host_str.as_deref()) {
        return Ok(super::OAuthState::disabled());
    }

    if mcp_raw.is_none() {
        tracing::warn!(
            canonical_host = canonical_host_str.as_deref().unwrap_or(""),
            "OAuth auto-enabling: oauth.mcp_enabled row absent but canonical_host is \
             configured; to disable, write oauth.mcp_enabled = false"
        );
    }

    // ── Step 3: Read-only settings (outside tx) ──
    // Note: these reads are not in the transaction. A concurrent change between here and
    // the commit is low-probability at boot time and treated as accepted low-severity TOCTOU.
    // The signing-secret write and peer-check are the only writes that must be atomic.
    let accepted_audience_hosts: Vec<String> =
        crate::settings_store::load_global_setting_raw(db, "oauth.accepted_audience_hosts")
            .await
            .unwrap_or(None)
            .and_then(|v| v.as_array().cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
    let allow_multi =
        crate::settings_store::load_global_setting_raw(db, "oauth.allow_multi_controller_unsafe")
            .await
            .unwrap_or(None)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let dcr_enabled =
        crate::settings_store::load_global_setting_raw(db, "oauth.dcr_enabled")
            .await
            .unwrap_or(None)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let cimd_enabled =
        crate::settings_store::load_global_setting_raw(db, "oauth.cimd_enabled")
            .await
            .unwrap_or(None)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    // ── Step 4: BEGIN IMMEDIATE — secret read-or-generate + peer registration ──
    //
    // Single transaction prevents two overlapping boots from diverging on the
    // signing secret and producing a spurious PeerWithDifferentFingerprint.
    let tx = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .context_to()?;

    let signing_secret: Vec<u8> = {
        let raw =
            crate::settings_store::load_global_setting_raw(&tx, "oauth.jwt_signing_secret")
                .await
                .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?;
        let existing = raw.and_then(|v| v.as_str().map(hex::decode).and_then(Result::ok));
        match existing {
            Some(bytes) => bytes,
            None => {
                let mut bytes = [0u8; 32];
                rand::rng().fill(&mut bytes);
                crate::settings_store::upsert_global_setting_raw(
                    &tx,
                    "oauth.jwt_signing_secret",
                    serde_json::json!(hex::encode(bytes)),
                )
                .await
                .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?;
                bytes.to_vec()
            }
        }
    };

    let boot_settings = OAuthBootSettings::new(
        canonical_host_str,
        accepted_audience_hosts.clone(),
        signing_secret.clone(),
        allow_multi,
    );
    let instance_id =
        validate_and_register(&tx, &boot_settings, OffsetDateTime::now_utc()).await?;

    tx.commit().await.context_to()?;

    // ── Step 5: Spawn heartbeat and construct live OAuthState ──
    spawn_heartbeat(db.clone(), instance_id);

    let canonical = super::canonical_url::load_canonical_url_config(db)
        .await
        .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?;
    let issuer = canonical.issuer().as_str().to_owned();

    Ok(super::OAuthState {
        enabled: true,
        canonical,
        signer: Arc::new(super::jwt::McpOAuthJwtSigner::new(&signing_secret)),
        verifier: Arc::new(super::jwt::McpOAuthJwtVerifier::new(
            &signing_secret,
            issuer,
            vec![],
        )),
        clock: Arc::new(OffsetDateTime::now_utc),
        instance_id,
        dcr_enabled,
        cimd_enabled,
    })
}
```

- [ ] **Step 4: Run new tests**

```bash
cargo test --all-features -p uptrakit-web-api -- boot_oauth_state 2>&1 | tail -10
```

Expected: both new tests pass.

- [ ] **Step 5: Run all OAuth boot validation tests**

```bash
cargo test --all-features -p uptrakit-web-api -- oauth_boot_validation 2>&1 | tail -10
```

Expected: all five tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/oauth/boot.rs \
        crates/ui/web-api/src/integration_tests/oauth_boot_validation.rs
git commit -m "feat(oauth): add boot_oauth_state with BEGIN IMMEDIATE secret gen + auto-enable"
```

---

## Task 4: Fix `load_oauth_settings_from_db` — preserve `Option<bool>` for `mcp`

**Files:**

- Modify: `crates/ui/web-api/src/routes/settings_oauth.rs`

Context: the current code collapses `Option<bool>` to `bool` immediately with `.unwrap_or(false)`.
When `mcp_enabled` row is absent and `canonical_host` is set, `boot_oauth_state` will boot with
`enabled = true`. But `load_oauth_settings_from_db` returns `mcp = false`, making `restart_required`
permanently `true` (compares `false` vs `state.oauth.enabled = true`).

- [ ] **Step 1: Write the failing test**

There is no existing unit test for this function (it's async and requires DB state). Verify the bug
exists at the logic level by reading the code:

```bash
grep -n "unwrap_or(false)" crates/ui/web-api/src/routes/settings_oauth.rs
```

Expected: line 38 — `unwrap_or(false)` on mcp bool read.

- [ ] **Step 2: Apply the fix to `load_oauth_settings_from_db`**

Replace the `mcp` read block in `crates/ui/web-api/src/routes/settings_oauth.rs`:

```rust
// Before (lines ~34-38):
let mcp = load_global_setting_raw(state.db(), "oauth.mcp_enabled")
    .await
    .unwrap_or(None)
    .and_then(|v| v.as_bool())
    .unwrap_or(false);

// After:
let canonical_host = load_global_setting_raw(state.db(), "oauth.canonical_host")
    .await
    .unwrap_or(None)
    .and_then(|v| v.as_str().map(ToOwned::to_owned));

let mcp_raw: Option<bool> = load_global_setting_raw(state.db(), "oauth.mcp_enabled")
    .await
    .unwrap_or(None)
    .and_then(|v| v.as_bool());
let mcp = uptrakit_web_api::oauth::resolve_mcp_enabled(mcp_raw, canonical_host.as_deref());
```

Note: `canonical_host` was previously read after `mcp`. Move the `canonical_host` read to BEFORE the
mcp computation so we can pass it to `resolve_mcp_enabled`. Remove the old `canonical_host` read
that comes after (around line 49). The `OAuthSettingsFromDb` struct already has
`canonical_host: Option<String>` — use the value computed here.

The `import` for `resolve_mcp_enabled` is needed. At the top of `settings_oauth.rs` the crate is
`crate` itself (`uptrakit_web_api`), so add an import using `crate::oauth::resolve_mcp_enabled` (or
just call it inline as a free function):

Add to the `use` block at the top of `settings_oauth.rs`:

```rust
use crate::oauth::resolve_mcp_enabled;
```

Then the mcp computation becomes:

```rust
let mcp = resolve_mcp_enabled(mcp_raw, canonical_host.as_deref());
```

- [ ] **Step 3: Verify `restart_required` logic is correct**

`OAuthSettingsFromDb::restart_required` compares `self.mcp != state.oauth.enabled`. After the fix,
`mcp` is `true` when auto-enabled — matching `state.oauth.enabled = true` after a boot with
`canonical_host` set. `restart_required` returns `false` as expected.

- [ ] **Step 4: Compile check**

```bash
cargo check --all-features -p uptrakit-web-api 2>&1 | tail -10
```

Expected: clean compile.

- [ ] **Step 5: Run all web-api tests**

```bash
cargo test --all-features -p uptrakit-web-api 2>&1 | tail -15
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/routes/settings_oauth.rs
git commit -m "fix(oauth/settings): preserve Option<bool> for mcp_enabled to fix restart_required banner"
```

---

## Task 5: Remove `OauthMcpEnabled` false seed

**Files:**

- Modify: `crates/core/controller-runtime/src/startup/oauth.rs`

Context: `seed_oauth_defaults` writes `mcp_enabled = false` on every fresh boot. This permanently
defeats the auto-enable logic for new installations. `insert_global_setting_if_absent` is idempotent
— existing `false` rows are unaffected; only new installs are fixed.

- [ ] **Step 1: Remove the seed line**

In `crates/core/controller-runtime/src/startup/oauth.rs`, delete line 25:

```rust
// Remove this line:
seed!(SettingKey::OauthMcpEnabled, serde_json::json!(false));
```

- [ ] **Step 2: Verify compile**

```bash
cargo check --all-features -p uptrakit-controller-runtime 2>&1 | tail -5
```

Expected: clean compile.

- [ ] **Step 3: Run tests**

```bash
cargo test --all-features -p uptrakit-controller-runtime 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/core/controller-runtime/src/startup/oauth.rs
git commit -m "fix(oauth/seed): remove false seed for OauthMcpEnabled to allow auto-enable"
```

---

## Task 6: Wire Phase 7d boot and `AppStateBuilder.oauth(oauth_state)`

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs`

Context: `AppStateBuilder` never calls `.oauth(...)` — `OAuthState` is permanently
`OAuthState::disabled()` in production regardless of DB settings. Adding Phase 7d between Phase 7c
(`seed_oauth_defaults`) and Phase 8 (`validate_configuration`) fixes this.

- [ ] **Step 1: Find insertion point**

```bash
grep -n "seed_oauth_defaults\|validate_configuration\|Phase 7\|Phase 8" \
  crates/core/controller-runtime/src/lib.rs | head -10
```

Identify the line where `seed_oauth_defaults` is called (Phase 7c) and where
`validate_configuration` is called (Phase 8). Phase 7d must be inserted between them.

- [ ] **Step 2: Add `boot_oauth_state` import**

`boot_oauth_state` is `pub` in `uptrakit_web_api::oauth::boot`. Add to the relevant `use` block in
`lib.rs`:

```rust
use uptrakit_web_api::oauth::boot::boot_oauth_state;
```

- [ ] **Step 3: Add Phase 7d — `boot_oauth_state` call**

After the `seed_oauth_defaults` call (Phase 7c) and before `validate_configuration` (Phase 8), add:

```rust
// Phase 7d: OAuth boot — wire OAuthState when mcp_enabled resolves to true.
let oauth_state = boot_oauth_state(&db_conn)
    .await
    .map_err(|e| e.change_context(AppError::Config("OAuth boot failed".into())))?;
```

- [ ] **Step 4: Wire `.oauth(oauth_state)` into `AppStateBuilder` chain**

Locate the `builder` chain around line 753 in `lib.rs`. Add `.oauth(oauth_state)` to the chain
before the closing `.reject_dangerous_commands(true)`:

```rust
let builder = AppState::builder()
    // ... all existing calls ...
    .reject_dangerous_commands(true)
    .oauth(oauth_state);  // ← add this
```

- [ ] **Step 5: Compile check (all features)**

```bash
cargo check --all-features -p uptrakit-controller-runtime 2>&1 | tail -10
```

Expected: clean compile.

- [ ] **Step 6: Run clippy**

```bash
cargo clippy --all-targets --all-features -p uptrakit-controller-runtime 2>&1 | tail -15
```

Expected: no new warnings or errors.

- [ ] **Step 7: Run all crate tests**

```bash
cargo test --all-features -p uptrakit-controller-runtime 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 8: Full workspace check**

```bash
cargo check --all-features 2>&1 | tail -10
cargo test --all-features -p uptrakit-web-api 2>&1 | tail -10
```

Expected: both clean.

- [ ] **Step 9: Commit**

```bash
git add crates/core/controller-runtime/src/lib.rs
git commit -m "feat(oauth/boot): wire Phase 7d boot_oauth_state into AppStateBuilder"
```

---

## Task 7: Update documentation deliverables

**Files:**

- Modify: `docs/admin/oauth-clients.md`
- Modify: `docs/development/oauth-mcp.md`

- [ ] **Step 1: Update `docs/admin/oauth-clients.md` first-run checklist**

Open `docs/admin/oauth-clients.md` and locate the first-run / initial setup section.

Replace or extend the checklist to reflect the auto-enable behaviour:

```markdown
## First-run checklist

1. Set `oauth.canonical_host` to the public hostname your controller is reachable at (e.g.
   `auth.example.com`). OAuth auto-enables as soon as this field is set — no separate
   `mcp_enabled = true` step is required.
2. Restart (or `reexec`) the controller after changing any OAuth setting to apply the change.
3. (Optional) Enable Dynamic Client Registration: set `oauth.dcr_enabled = true`.
4. (Optional) Enable Client-Initiated Metadata Discovery: set `oauth.cimd_enabled = true`.
```

If a first-run checklist already exists, update only the `mcp_enabled` step — do not duplicate the
rest of the checklist content.

- [ ] **Step 2: Verify markdownlint on updated file**

```bash
npx markdownlint --config .markdownlint.json docs/admin/oauth-clients.md
```

Expected: no errors.

- [ ] **Step 3: Update `docs/development/oauth-mcp.md` boot-sequence section**

Open `docs/development/oauth-mcp.md` and locate the section describing the OAuth boot sequence (look
for headings like "Boot", "Startup", or "AppState").

Add (or extend) a `## OAuth boot sequence` section containing:

- One paragraph stating `boot_oauth_state` in `crates/ui/web-api/src/oauth/boot.rs` is called during
  Phase 7d (after `seed_oauth_defaults`, before `validate_configuration`).
- One paragraph describing `resolve_mcp_enabled(explicit: Option<bool>, canonical_host: Option<&str>) -> bool`
  with the five-row truth table (absent+no host→false; absent+host→true; explicit false→false;
  explicit true+no host→true with guard; explicit true+host→true).
- One paragraph explaining the single `BEGIN IMMEDIATE` transaction: reads or generates
  `jwt_signing_secret`, then calls `validate_and_register` in the same transaction to prevent
  split-brain on rapid restart loops.
- A `### Integration test pattern` subsection with the following Rust snippet:

```rust
api_client.update_oauth_settings("127.0.0.1:<port>").await;
let current_gen = /* read X-Reexec-Generation from GET /healthz */;
api_client.force_reexec().await;
api_client.wait_for_generation(current_gen + 1, Duration::from_secs(30)).await;
```

- [ ] **Step 4: Verify markdownlint on updated file**

```bash
npx markdownlint --config .markdownlint.json docs/development/oauth-mcp.md
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add docs/admin/oauth-clients.md docs/development/oauth-mcp.md
git commit -m "docs(oauth): update first-run checklist and boot-sequence dev docs for auto-enable"
```

---

## Quality Gates

Run before marking this plan complete:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite -p uptrakit-web-api
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features -p uptrakit-web-api
cargo test --all-features -p uptrakit-controller-runtime
cargo deny check
```
