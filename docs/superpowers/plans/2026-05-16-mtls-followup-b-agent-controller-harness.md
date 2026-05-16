# mTLS Follow-ups — Plan B: AgentControllerHarness

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a Docker-based `AgentControllerHarness` that unblocks the two stubbed SPIFFE/cert-rotation
integration tests, and add `cert_serial_number` to `ServiceResponse` to enable the renewal-detection polling
it needs.

**Architecture:** HTTP-only harness wraps existing `ControllerContainer` + `ServiceContainer`. Two new test-utils
endpoints (`POST /api/v1/test/services/{id}/request-renewal` and `.../disconnect`) are compiled under the
`test-utils` Cargo feature and protected at runtime by `UPTRAKIT_TEST_UTILS_ENABLED=true`. `ServiceResponse`
gains `cert_serial_number: Option<String>` (detail endpoint only) and `#[non_exhaustive]`, requiring a `::new()`
constructor and fixes to four external struct-literal sites.

**Tech Stack:** Rust (edition 2024), axum, testcontainers, serde_json, `uptrakit-openapi-client` (raw_request escape hatch)

---

## File Map

**New files:**

| File                                                                      | Role                                                        |
| ------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `crates/ui/web-api/src/routes/test_utils.rs`                              | Test-utils endpoints (feature-gated)                        |
| `crates/core/integration-tests/tests/helpers/agent_controller_harness.rs` | `AgentControllerHarness`, `ControllerHandle`, `AgentHandle` |

**Modified files:**

| File                                                            | Change                                                                                 |
| --------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `crates/shared/web-api-types/src/services.rs`                   | `cert_serial_number` field, `#[non_exhaustive]`, `::new()` constructor                 |
| `crates/ui/web-api-queries/src/queries/services.rs`             | Fix `model_to_response` literal; add `get_active_service_detail` + `fetch_cert_serial` |
| `crates/ui/web-api/src/routes/services.rs`                      | Fix `reject_service` literal; update `get_service` to call `get_active_service_detail` |
| `crates/ui/cli/src/commands/services.rs`                        | Fix `sample_service` literal                                                           |
| `crates/ui/cli/tests/command_execution.rs`                      | Fix `sample_service` literal                                                           |
| `crates/ui/web-api/Cargo.toml`                                  | Add `test-utils` feature                                                               |
| `crates/core/controller-runtime/Cargo.toml`                     | Add `test-utils = ["uptrakit-web-api/test-utils"]`                                     |
| `crates/core/controller-standalone/Cargo.toml`                  | Add `test-utils = ["uptrakit-controller-runtime/test-utils"]`                          |
| `crates/ui/web-api/src/router.rs`                               | Register test-utils routes under `#[cfg(feature = "test-utils")]`                      |
| `crates/core/integration-tests/tests/helpers/containers.rs`     | Add `start_with_trust_domain`, `UPTRAKIT_TEST_UTILS_ENABLED` env var                   |
| `crates/core/integration-tests/tests/helpers/api_client.rs`     | Add `raw_get` helper                                                                   |
| `crates/core/integration-tests/tests/helpers/mod.rs`            | Expose `agent_controller_harness` module                                               |
| `crates/core/integration-tests/tests/spiffe_identity.rs`        | Implement one test; demote other to comment                                            |
| `crates/core/integration-tests/tests/cert_rotation_hot_swap.rs` | Implement test                                                                         |
| `docker/Dockerfile.test`                                        | Build controller-standalone with `test-utils` feature                                  |

---

## Task 1: Add `cert_serial_number` to `ServiceResponse` + `#[non_exhaustive]` + `::new()` constructor

**Files:**

- Modify: `crates/shared/web-api-types/src/services.rs:11-58`

- [ ] **Step 1: Open `crates/shared/web-api-types/src/services.rs`. Replace the `ServiceResponse` struct definition (lines 11–58) with:**

```rust
/// Unified response for any service (agent or MQTT).
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServiceResponse {
    pub id: Uuid,
    pub capabilities: Vec<String>,
    pub service_label: String,
    pub hostname: String,
    pub friendly_name: String,
    pub is_embedded: bool,
    pub ip_address: Option<String>,
    pub status: ServiceStatus,
    pub client_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub last_seen_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub updated_at: OffsetDateTime,
    /// Custom ping interval override in seconds. `None` means the
    /// service-profile default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_interval_seconds: Option<u32>,
    /// Per-service certificate lifetime override in hours. `None` means the
    /// global default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_lifetime_hours: Option<u32>,
    /// External service IDs currently causing this embedded service to yield.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yielded_to: Option<Vec<Uuid>>,
    /// SPIFFE identity URI from the service's current certificate.
    ///
    /// Present only when the controller has a trust domain configured and the
    /// service certificate contains a SPIFFE URI SAN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spiffe_id: Option<String>,
    /// Serial number of the most recent non-revoked service certificate.
    ///
    /// Populated only on the detail endpoint (`GET /api/v1/services/{id}`).
    /// Absent (`None`) on the list endpoint to avoid N+1 queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_serial_number: Option<String>,
}
```

- [ ] **Step 2: Add a `::new()` constructor immediately after the struct (before `ListServicesQuery`). The coding
      standards rule requires a parameter struct when a function exceeds Clippy's argument threshold. For a
      `#[non_exhaustive]` struct constructor there is no alternative — a builder would require external callers to
      import it, and partial construction via `Default` would silently zero-out required fields. Suppress with
      `#[expect]` (not `#[allow]`), which becomes a compile error if the suppression is ever no longer needed:**

```rust
impl ServiceResponse {
    #[expect(
        clippy::too_many_arguments,
        reason = "ServiceResponse has 17 fields; all are required at construction"
    )]
    pub fn new(
        id: Uuid,
        capabilities: Vec<String>,
        service_label: String,
        hostname: String,
        friendly_name: String,
        is_embedded: bool,
        ip_address: Option<String>,
        status: ServiceStatus,
        client_version: Option<String>,
        last_seen_at: Option<OffsetDateTime>,
        created_at: OffsetDateTime,
        updated_at: OffsetDateTime,
        ping_interval_seconds: Option<u32>,
        cert_lifetime_hours: Option<u32>,
        yielded_to: Option<Vec<Uuid>>,
        spiffe_id: Option<String>,
        cert_serial_number: Option<String>,
    ) -> Self {
        Self {
            id,
            capabilities,
            service_label,
            hostname,
            friendly_name,
            is_embedded,
            ip_address,
            status,
            client_version,
            last_seen_at,
            created_at,
            updated_at,
            ping_interval_seconds,
            cert_lifetime_hours,
            yielded_to,
            spiffe_id,
            cert_serial_number,
        }
    }
}
```

- [ ] **Step 3: Check `web-api-types` compiles (this crate defines the type; downstream breaks come in Task 2):**

```bash
cargo check -p uptrakit-web-api-types --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Commit:**

```bash
git add crates/shared/web-api-types/src/services.rs
git commit -m "feat(web-api-types): add cert_serial_number to ServiceResponse; add #[non_exhaustive] + ::new()"
```

---

## Task 2: Fix external struct literal sites broken by `#[non_exhaustive]`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/services.rs:142`
- Modify: `crates/ui/web-api/src/routes/services.rs:697`
- Modify: `crates/ui/cli/src/commands/services.rs:550`
- Modify: `crates/ui/cli/tests/command_execution.rs:81`

`#[non_exhaustive]` forbids struct literals in external crates. Replace all four with `ServiceResponse::new(...)`.

- [ ] **Step 1: Replace `model_to_response` in `crates/ui/web-api-queries/src/queries/services.rs` (lines
      137–160). Change the `ServiceResponse { ... }` literal to:**

```rust
fn model_to_response(m: service::Model, yielded_to: Option<Vec<Uuid>>) -> ServiceResponse {
    let caps = parse_capabilities(&m.capabilities);
    let profile = ServiceProfile::from_capabilities(&caps);
    let has_ssh = caps.contains(&Capability::SshRemote);
    let cap_strings: Vec<String> = caps.iter().map(|c| c.as_str().to_string()).collect();
    ServiceResponse::new(
        m.id,
        cap_strings,
        profile.service_label(has_ssh).to_string(),
        m.hostname,
        m.friendly_name,
        m.is_embedded,
        m.ip_address,
        m.status,
        m.client_version,
        m.last_seen_at,
        m.created_at,
        m.updated_at,
        m.ping_interval_seconds.map(|v| v as u32),
        m.cert_lifetime_hours.map(|v| v as u32),
        yielded_to,
        None, // spiffe_id — populated by the handler if trust domain is set
        None, // cert_serial_number — populated only by get_active_service_detail
    )
}
```

- [ ] **Step 2: Replace the `ServiceResponse { ... }` literal in `reject_service` in
      `crates/ui/web-api/src/routes/services.rs` (lines 697–714). The existing literal starts with
      `let resp = ServiceResponse {`. Replace with:**

```rust
let resp = ServiceResponse::new(
    after.id,
    cap_strings,
    profile.service_label(has_ssh).to_string(),
    after.hostname.clone(),
    after.friendly_name.clone(),
    after.is_embedded,
    after.ip_address.clone(),
    WireStatus::Rejected,
    after.client_version.clone(),
    after.last_seen_at,
    after.created_at,
    after.updated_at,
    after.ping_interval_seconds.map(|v| v as u32),
    after.cert_lifetime_hours.map(|v| v as u32),
    None, // yielded_to
    None, // spiffe_id
    None, // cert_serial_number
);
```

- [ ] **Step 3: Replace `sample_service` in `crates/ui/cli/src/commands/services.rs` (line 550). The literal spans lines 550–573. Replace with:**

```rust
fn sample_service() -> ServiceResponse {
    ServiceResponse::new(
        "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
            .parse::<Uuid>()
            .unwrap(),
        vec![
            "graceful_shutdown".to_string(),
            "software_discovery".to_string(),
            "update_hooks".to_string(),
        ],
        "Agent".to_string(),
        "agent-host.local".to_string(),
        "Test Agent".to_string(),
        false,
        None,
        "approved".parse().unwrap(),
        Some("1.0.0".to_string()),
        Some(datetime!(2025-01-01 00:00:00 UTC)),
        datetime!(2025-01-01 00:00:00 UTC),
        datetime!(2025-01-01 00:00:00 UTC),
        None,
        None,
        None,
        None,
        None,
    )
}
```

- [ ] **Step 4: Replace `sample_service` in `crates/ui/cli/tests/command_execution.rs` (line 80). Replace the literal (lines 80–103) with:**

```rust
fn sample_service() -> ServiceResponse {
    ServiceResponse::new(
        service_id(),
        vec![
            "graceful_shutdown".to_string(),
            "software_discovery".to_string(),
            "update_hooks".to_string(),
        ],
        "Agent".to_string(),
        "agent-host.local".to_string(),
        "Test Agent".to_string(),
        false,
        None,
        "approved".parse().unwrap(),
        Some("1.0.0".to_string()),
        Some(datetime!(2025-01-01 00:00:00 UTC)),
        datetime!(2025-01-01 00:00:00 UTC),
        datetime!(2025-01-01 00:00:00 UTC),
        None,
        None,
        None,
        None,
        None,
    )
}
```

- [ ] **Step 5: Check all four crates compile:**

```bash
cargo check -p uptrakit-web-api-queries --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-web-api --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-cli --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Commit:**

```bash
git add crates/ui/web-api-queries/src/queries/services.rs \
        crates/ui/web-api/src/routes/services.rs \
        crates/ui/cli/src/commands/services.rs \
        crates/ui/cli/tests/command_execution.rs
git commit -m "refactor(services): migrate ServiceResponse struct literals to ::new() constructor"
```

---

## Task 3: Add cert serial to the detail endpoint

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/services.rs`
- Modify: `crates/ui/web-api/src/routes/services.rs`

- [ ] **Step 1: In `crates/ui/web-api-queries/src/queries/services.rs`, add two functions after
      `build_service_responses` (around line 200). The import `use uptrakit_shared_db::entity::service_certificate;`
      is already present at line 10.**

```rust
/// Fetch the serial number of the most recent non-revoked certificate for a service.
///
/// `service_certificate` has no `tenant_id` column. Tenant isolation is enforced
/// by joining through `service::Entity` (which is `TenantScoped`), per the
/// coding-standards rule: "service_host has no tenant_id; scope queries via
/// TenantDb::find_via_tenant_join through service (TenantScoped)".
async fn fetch_cert_serial(
    tenant_db: &TenantDb,
    service_id: Uuid,
) -> Result<Option<String>> {
    let cert = tenant_db
        .find_via_tenant_join::<service_certificate::Entity, service::Entity>(
            service_certificate::Relation::Service.def(),
        )
        .filter(service_certificate::Column::ServiceId.eq(service_id))
        .filter(service_certificate::Column::RevokedAt.is_null())
        .order_by_desc(service_certificate::Column::CreatedAt)
        .one(tenant_db.db())
        .await
        .context_to()?;
    Ok(cert.map(|c| c.serial_number))
}

/// Like `get_active_service` but also populates `cert_serial_number`.
///
/// Use only for the detail endpoint — calling this for a list would produce N+1 queries.
pub async fn get_active_service_detail(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<ServiceResponse>> {
    let Some(mut svc) = get_active_service(tenant_db, id).await? else {
        return Ok(None);
    };
    svc.cert_serial_number = fetch_cert_serial(tenant_db, id).await?;
    Ok(Some(svc))
}
```

- [ ] **Step 2: In `crates/ui/web-api/src/routes/services.rs`, find the `get_service` handler (line ~166).
      It currently calls `svc_queries::get_active_service`. Change it to `svc_queries::get_active_service_detail`:**

Find:

```rust
match svc_queries::get_active_service(&tenant_db, service_id).await {
    Ok(Some(mut resp)) => {
```

Replace with:

```rust
match svc_queries::get_active_service_detail(&tenant_db, service_id).await {
    Ok(Some(mut resp)) => {
```

- [ ] **Step 3: Verify compilation:**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-web-api-queries --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Run services unit tests to confirm the handler still works:**

```bash
cargo test -p uptrakit-web-api --all-features -- routes::services 2>&1 | tail -20
```

Expected: all services tests pass.

- [ ] **Step 5: Commit:**

```bash
git add crates/ui/web-api-queries/src/queries/services.rs \
        crates/ui/web-api/src/routes/services.rs
git commit -m "feat(services): populate cert_serial_number on the service detail endpoint"
```

---

## Task 4: Add `test-utils` feature to Cargo manifests and Dockerfile

**Files:**

- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `crates/core/controller-standalone/Cargo.toml`
- Modify: `docker/Dockerfile.test`

- [ ] **Step 1: In `crates/ui/web-api/Cargo.toml`, add `test-utils` to the `[features]` section (after `interactive = []`):**

```toml
interactive = []
test-utils = []
```

- [ ] **Step 2: In `crates/core/controller-runtime/Cargo.toml`, add to the `[features]` section (after `interactive = [...]`):**

```toml
test-utils = ["uptrakit-web-api/test-utils"]
```

- [ ] **Step 3: In `crates/core/controller-standalone/Cargo.toml`, add to the `[features]` section (after `notifications-email = [...]`):**

```toml
test-utils = ["uptrakit-controller-runtime/test-utils"]
```

- [ ] **Step 4: In `docker/Dockerfile.test`, find the `cargo chef cook` and `cargo build` lines for
      `uptrakit-controller-standalone`. Add `test-utils` to both `--features` flags:**

Find (two occurrences — cook + build):

```sh
-p uptrakit-controller-standalone --features nats
```

Replace both with:

```sh
-p uptrakit-controller-standalone --features nats,test-utils
```

- [ ] **Step 5: Verify the feature chain compiles with test-utils:**

```bash
cargo check -p uptrakit-controller-standalone --features test-utils 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 6: Commit:**

```bash
git add crates/ui/web-api/Cargo.toml \
        crates/core/controller-runtime/Cargo.toml \
        crates/core/controller-standalone/Cargo.toml \
        docker/Dockerfile.test
git commit -m "feat(test-utils): add test-utils feature flag to web-api, controller-runtime, controller-standalone, and Dockerfile.test"
```

---

## Task 5: Implement test-utils endpoints and wire them into the router

**Files:**

- Create: `crates/ui/web-api/src/routes/test_utils.rs`
- Modify: `crates/ui/web-api/src/router.rs`

The two endpoints are **only compiled under `#[cfg(feature = "test-utils")]`** and also guarded at runtime by
`UPTRAKIT_TEST_UTILS_ENABLED=true`. The runtime guard means an accidentally compiled binary won't expose these
routes unless the operator explicitly sets the env var.

- [ ] **Step 1: Create `crates/ui/web-api/src/routes/test_utils.rs` with the following content:**

```rust
//! Test-utilities endpoints — only compiled with the `test-utils` Cargo feature.
//!
//! All handlers check `UPTRAKIT_TEST_UTILS_ENABLED=true` at runtime before acting.
//! If the env var is absent or not `"true"`, every handler returns 404, making
//! the endpoints invisible to clients not in a test context.
#![cfg(feature = "test-utils")]

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;
use uptrakit_wire::{ControllerMessage, RequestCertRenewalPayload};

use crate::AppState;

fn test_utils_allowed() -> bool {
    std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref() == Ok("true")
}

/// Send `RequestCertRenewal` to a specific connected service.
///
/// Returns 200 if the message was sent, 404 if the service is not connected
/// or if `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"`.
pub(crate) async fn request_service_renewal(
    State(state): State<Arc<AppState>>,
    Path(service_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let msg = ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
        reason: "test-utils: forced renewal".to_string(),
    });
    if state.service_connections.send(&service_id, msg).await {
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Close the WebSocket for a specific service, triggering its reconnect loop.
///
/// Returns 200 unconditionally (disconnect is a no-op if already disconnected).
/// Returns 404 if `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"`.
pub(crate) async fn disconnect_service(
    State(state): State<Arc<AppState>>,
    Path(service_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    state.service_connections.force_disconnect(&service_id).await;
    StatusCode::OK.into_response()
}
```

- [ ] **Step 2: In `crates/ui/web-api/src/router.rs`, add the routes module declaration. Find where other route
      modules are declared (look near the top of the file or in `src/routes/mod.rs`):**

```bash
grep -n "mod test_utils\|pub mod " crates/ui/web-api/src/routes/mod.rs 2>/dev/null | head -10
ls crates/ui/web-api/src/routes/mod.rs 2>/dev/null && echo "has mod.rs" || echo "no mod.rs"
```

- [ ] **Step 3: If there is a `crates/ui/web-api/src/routes/mod.rs`, add:**

```rust
#[cfg(feature = "test-utils")]
pub(crate) mod test_utils;
```

If routes are declared as `pub(crate) mod X;` directly in `lib.rs` or `router.rs`, add the same line there
alongside other route module declarations.

- [ ] **Step 4: In `crates/ui/web-api/src/router.rs`, find the `#[cfg(feature = "interactive")]` block
      (around lines 964–971). Add the test-utils routes immediately after it:**

```rust
#[cfg(feature = "test-utils")]
{
    router = router.route(
        "/api/v1/test/services/{id}/request-renewal",
        axum::routing::post(crate::routes::test_utils::request_service_renewal),
    );
    router = router.route(
        "/api/v1/test/services/{id}/disconnect",
        axum::routing::post(crate::routes::test_utils::disconnect_service),
    );
}
```

- [ ] **Step 5: Verify compilation with and without the feature:**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep -E "^error" | head -20
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
```

Expected: no errors in either invocation.

- [ ] **Step 6: Commit:**

```bash
git add crates/ui/web-api/src/routes/test_utils.rs \
        crates/ui/web-api/src/router.rs
git commit -m "feat(test-utils): add request-renewal and disconnect endpoints behind test-utils feature"
```

(Also add `crates/ui/web-api/src/routes/mod.rs` if you modified it in Step 3.)

---

## Task 6: Implement `AgentControllerHarness`

**Files:**

- Modify: `crates/core/integration-tests/tests/helpers/containers.rs`
- Modify: `crates/core/integration-tests/tests/helpers/api_client.rs`
- Create: `crates/core/integration-tests/tests/helpers/agent_controller_harness.rs`
- Modify: `crates/core/integration-tests/tests/helpers/mod.rs`

- [ ] **Step 1: In `containers.rs`, add `start_with_trust_domain` (public) and `start_internal` (private).
      Modify the existing `start` to delegate to `start_internal(network, None)`. Add at the end of
      `impl ControllerContainer`:**

```rust
/// Start a controller container with a SPIFFE trust domain configured.
///
/// Used by [`AgentControllerHarness`] when tests need SPIFFE URI SAN validation.
pub(crate) async fn start_with_trust_domain(network: &str, trust_domain: &str) -> Self {
    Self::start_internal(network, Some(trust_domain)).await
}

/// Internal: start with an optional trust domain.
async fn start_internal(network: &str, trust_domain: Option<&str>) -> Self {
    let nats_name = format!("nats-{}", uuid::Uuid::now_v7());
    let nats_container = GenericImage::new("nats", "latest")
        .with_wait_for(WaitFor::Log(
            LogWaitStrategy::stdout_or_stderr("Server is ready").with_times(1),
        ))
        .with_cmd(vec!["-js".to_string()])
        .with_network(network)
        .with_container_name(&nats_name)
        .with_hostname(&nats_name)
        .start()
        .await
        .expect("start nats container");

    let container_name = format!("controller-{}", uuid::Uuid::now_v7());

    let mut config_file = NamedTempFile::new().expect("create temp config file");
    let tls_section = trust_domain
        .filter(|d| !d.is_empty())
        .map(|d| format!("\n[tls]\ntrust_domain = \"{d}\"\n"))
        .unwrap_or_default();
    write!(
        config_file,
        r#"
[db]
url = "sqlite:///data/state/controller.db?mode=rwc"

[master_key]
path = "/tmp/dummy-overridden-by-cli"

[network.https]
addr = "[::]:8443"

[network.pki]
addr = "[::]:8444"

[nats]
url = "nats://{nats_name}:{NATS_PORT}"

[audit]
filter = "all"
retention_days = 90

[log]
path = "/data/state/controller.log"
level = "info"{tls_section}
"#
    )
    .expect("write config file");

    let container = GenericImage::new(TEST_IMAGE, TEST_IMAGE_TAG)
        .with_exposed_port(CONTROLLER_PORT.tcp())
        .with_wait_for(WaitFor::Log(
            LogWaitStrategy::stdout_or_stderr("HTTPS server listening on").with_times(1),
        ))
        .with_cmd(vec![
            "uptrakit-controller-standalone".to_string(),
            "--master-key-from".to_string(),
            "env:UPTRAKIT_TEST_MASTER_KEY".to_string(),
        ])
        .with_mount(
            Mount::bind_mount(
                config_file.path().to_str().expect("config path"),
                "/etc/uptrakit/controller.toml",
            )
            .with_access_mode(AccessMode::ReadOnly),
        )
        .with_env_var("UPTRAKIT_TEST_MASTER_KEY", TEST_MASTER_KEY)
        .with_env_var("UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN", ENROLLMENT_TOKEN)
        .with_env_var("UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_MAX_USES", "100")
        .with_env_var("UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN_TTL", "3600")
        .with_env_var(
            "UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN",
            SYSTEM_ENROLLMENT_TOKEN,
        )
        .with_env_var("UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_MAX_USES", "100")
        .with_env_var("UPTRAKIT_BOOTSTRAP_SYSTEM_ENROLLMENT_TOKEN_TTL", "3600")
        .with_env_var("UPTRAKIT_TEST_UTILS_ENABLED", "true")
        .with_network(network)
        .with_container_name(&container_name)
        .with_hostname(&container_name)
        .start()
        .await
        .expect("start controller container");

    let host_port = container
        .get_host_port_ipv4(CONTROLLER_PORT.tcp())
        .await
        .expect("get controller mapped port");

    let registration_token =
        container.stderr_to_vec().await.ok().and_then(|stderr| {
            parse_initial_registration_token(&String::from_utf8_lossy(&stderr))
        });

    Self {
        _nats_container: nats_container,
        _controller_container: container,
        _config_file: config_file,
        host_port,
        container_name,
        registration_token,
    }
}
```

- [ ] **Step 2: Modify the existing `start` function to delegate to `start_internal`:**

Replace the body of `pub(crate) async fn start(network: &str) -> Self` with:

```rust
pub(crate) async fn start(network: &str) -> Self {
    Self::start_internal(network, None).await
}
```

- [ ] **Step 3: In `api_client.rs`, add a `raw_get` helper method to `impl ApiClient` (after `wait_for_service_count`):**

```rust
/// Execute a raw GET to the given path and return the response body as JSON.
///
/// Uses the authenticated client. Panics on network error.
pub(crate) async fn raw_get(&self, path: &str) -> serde_json::Value {
    let resp = self
        .authenticated()
        .raw_request("GET", path, None)
        .await
        .expect("raw GET request");
    resp.body
}

/// Execute a raw POST to the given path with no body and return the HTTP status code.
pub(crate) async fn raw_post(&self, path: &str) -> reqwest::StatusCode {
    let resp = self
        .authenticated()
        .raw_request("POST", path, None)
        .await
        .expect("raw POST request");
    resp.status
}
```

- [ ] **Step 4: Create `crates/core/integration-tests/tests/helpers/agent_controller_harness.rs`:**

```rust
//! Docker-based harness for SPIFFE and cert-rotation integration tests.
//!
//! Wraps [`ControllerContainer`] and [`ServiceContainer`] with higher-level
//! polling helpers that integration tests need but that do not belong in the
//! general-purpose [`ApiClient`].
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration test infrastructure: panics are acceptable in harness helpers"
)]

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::api_client::ApiClient;
use super::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Options for starting an `AgentControllerHarness`.
pub(crate) struct HarnessOptions {
    /// SPIFFE trust domain to configure in the controller.
    ///
    /// Empty string disables SPIFFE SAN validation (controller runs without a
    /// trust domain and `spiffe_id` is never populated in service responses).
    pub trust_domain: String,
}

/// High-level test harness: one controller + N agents on a shared Docker network.
///
/// Dropped: containers are stopped, Docker network is removed.
pub(crate) struct AgentControllerHarness {
    network: String,
    pub controller: ControllerHandle,
}

/// Handle to the running controller — exposes test-utils API operations.
pub(crate) struct ControllerHandle {
    _container: ControllerContainer,
    api: Arc<ApiClient>,
}

/// Handle to a running agent container.
///
/// Created by [`AgentControllerHarness::spawn_agent`].
pub(crate) struct AgentHandle {
    _container: ServiceContainer,
    /// Service IDs that existed before this agent was spawned.
    /// Used by `wait_for_connected` to identify the new service.
    known_before: Vec<Uuid>,
    /// Cached once `wait_for_connected` resolves the new service.
    service_id: std::sync::OnceLock<Uuid>,
    api: Arc<ApiClient>,
}

impl AgentControllerHarness {
    /// Start a controller on a fresh Docker network and return a harness.
    ///
    /// Creates the Docker network, starts the controller, waits for readiness,
    /// and logs in as the first user.
    pub(crate) async fn start_with(opts: HarnessOptions) -> Self {
        let network = test_network_name();
        std::process::Command::new("docker")
            .args(["network", "create", &network])
            .status()
            .expect("create Docker test network");

        let container = if opts.trust_domain.is_empty() {
            ControllerContainer::start(&network).await
        } else {
            ControllerContainer::start_with_trust_domain(&network, &opts.trust_domain).await
        };

        let host_port = container.host_port();
        let registration_token = container.registration_token().map(str::to_owned);

        let mut api = ApiClient::new(host_port);
        api.wait_for_ready(Duration::from_secs(30)).await;
        api.register_and_login_with_token(registration_token.as_deref())
            .await;

        let api = Arc::new(api);
        let controller = ControllerHandle {
            _container: container,
            api,
        };

        Self { network, controller }
    }

    /// Spawn an agent on the same Docker network.
    ///
    /// Snapshots existing services before starting the container so
    /// `wait_for_connected` can identify the new service unambiguously.
    pub(crate) async fn spawn_agent(&self, _name: &str) -> AgentHandle {
        let known_before: Vec<Uuid> = self
            .controller
            .api
            .list_services()
            .await
            .into_iter()
            .map(|s| s.id)
            .collect();

        let container = ServiceContainer::start_agent(
            &self.network,
            self.controller._container.container_name(),
        )
        .await;

        AgentHandle {
            _container: container,
            known_before,
            service_id: std::sync::OnceLock::new(),
            api: Arc::clone(&self.controller.api),
        }
    }
}

impl Drop for AgentControllerHarness {
    fn drop(&mut self) {
        // Containers are already stopped by their own Drop impls (testcontainers).
        // Remove the Docker network that was created by start_with.
        let _ = std::process::Command::new("docker")
            .args(["network", "rm", &self.network])
            .status();
    }
}

impl ControllerHandle {
    /// Return the SPIFFE identity of a service (calls the detail endpoint).
    pub(crate) async fn identity_of(&self, service_id: Uuid) -> ServiceIdentity {
        let body = self
            .api
            .raw_get(&format!("/api/v1/services/{service_id}"))
            .await;
        ServiceIdentity {
            spiffe_id: body["spiffe_id"].as_str().map(str::to_owned),
        }
    }

    /// Send `RequestCertRenewal` to the connected service via the test-utils endpoint.
    ///
    /// Retries up to 5 times at 500ms intervals if the service is not yet
    /// connected (the message can only be delivered to a live WebSocket).
    /// Panics if the service is not connected after all retries.
    pub(crate) async fn request_cert_renewal(&self, service_id: Uuid) {
        for attempt in 1..=5u32 {
            let status = self
                .api
                .raw_post(&format!(
                    "/api/v1/test/services/{service_id}/request-renewal"
                ))
                .await;
            if status == reqwest::StatusCode::OK {
                return;
            }
            if attempt < 5 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        panic!("request_cert_renewal: service {service_id} not connected after 5 attempts");
    }

    /// Close the WebSocket for a service, triggering its reconnect loop.
    pub(crate) async fn disconnect_service(&self, service_id: Uuid) {
        let status = self
            .api
            .raw_post(&format!("/api/v1/test/services/{service_id}/disconnect"))
            .await;
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "disconnect_service returned {status}"
        );
    }

    /// Poll until the service with `service_id` is `Approved` again.
    ///
    /// Used after `disconnect_service` to wait for the agent to reconnect.
    pub(crate) async fn wait_for_service_approved(&self, service_id: Uuid) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let body = self
                .api
                .raw_get(&format!("/api/v1/services/{service_id}"))
                .await;
            if body["status"].as_str() == Some("approved") {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "service {service_id} did not return to Approved within 60s after disconnect"
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// SPIFFE identity information for a service.
pub(crate) struct ServiceIdentity {
    pub spiffe_id: Option<String>,
}

impl AgentHandle {
    /// Poll until this agent's service appears as `Approved` in the controller.
    ///
    /// On success, caches the service ID for use by other methods.
    /// Panics if no new `Approved` service appears within 60 seconds.
    pub(crate) async fn wait_for_connected(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let current = self.api.list_services().await;
            if let Some(svc) = current.iter().find(|s| {
                !self.known_before.contains(&s.id)
                    && s.status.to_string() == "approved"
            }) {
                self.service_id
                    .set(svc.id)
                    .expect("wait_for_connected called more than once");
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("agent did not appear as Approved within 60s");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Return the service ID cached by `wait_for_connected`.
    ///
    /// Panics if `wait_for_connected` has not been called yet.
    pub(crate) fn service_id(&self) -> Uuid {
        *self
            .service_id
            .get()
            .expect("service_id called before wait_for_connected")
    }

    /// Return the current cert serial from the controller's service detail endpoint.
    pub(crate) async fn cert_serial_number(&self) -> String {
        let id = self.service_id();
        let body = self
            .api
            .raw_get(&format!("/api/v1/services/{id}"))
            .await;
        body["cert_serial_number"]
            .as_str()
            .expect("cert_serial_number missing in service detail response")
            .to_owned()
    }

    /// Poll until the cert serial in the controller differs from `before_serial`.
    ///
    /// Panics if the serial does not change within 30 seconds.
    pub(crate) async fn wait_for_cert_renewed(&self, before_serial: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(serial) = self
                .api
                .raw_get(&format!("/api/v1/services/{}", self.service_id()))
                .await["cert_serial_number"]
                .as_str()
            {
                if serial != before_serial {
                    return;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "cert serial did not change from {before_serial} within 30s"
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
```

- [ ] **Step 5: Expose the new module in `crates/core/integration-tests/tests/helpers/mod.rs`
      (currently exposes only `api_client` and `containers`). Add:**

```rust
pub(crate) mod agent_controller_harness;
```

- [ ] **Step 6: Verify the helpers compile (no test run needed yet):**

```bash
cargo check -p uptrakit-integration-tests --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 7: Commit:**

```bash
git add crates/core/integration-tests/tests/helpers/agent_controller_harness.rs \
        crates/core/integration-tests/tests/helpers/mod.rs \
        crates/core/integration-tests/tests/helpers/containers.rs \
        crates/core/integration-tests/tests/helpers/api_client.rs
git commit -m "feat(integration-tests): add AgentControllerHarness, ControllerHandle, AgentHandle"
```

---

## Task 7: Implement SPIFFE identity integration tests

**Files:**

- Modify: `crates/core/integration-tests/tests/spiffe_identity.rs`

The second stub (`agent_with_wrong_trust_domain_csr_rejected`) is demoted: the unit test already exists at
`controller-runtime/src/cert_signer.rs::spiffe_san_wrong_trust_domain_rejected`. Replace the stub with a
comment pointing there.

- [ ] **Step 1: Replace the entire content of `crates/core/integration-tests/tests/spiffe_identity.rs` with:**

````rust
//! End-to-end: Agent CSR → Controller signs (SPIFFE SAN preserved) → Agent
//! reconnects → Controller extracts service identity via SPIFFE SAN.
//!
//! Run:
//! ```sh
//! docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
//! cargo test -p uptrakit-integration-tests spiffe_identity -- --ignored --nocapture
//! ```

use crate::helpers::agent_controller_harness::{AgentControllerHarness, HarnessOptions};

/// Verify that an Agent enrolls with a SPIFFE URI SAN in its CSR, the Controller
/// signs it preserving the SAN, and the Controller returns the SPIFFE identity
/// in the service detail response.
#[tokio::test]
#[ignore = "requires Docker"]
async fn agent_enrolls_and_authenticates_via_spiffe_san() {
    let harness = AgentControllerHarness::start_with(HarnessOptions {
        trust_domain: "controller.test.local".into(),
    })
    .await;

    let agent = harness.spawn_agent("agent-spiffe").await;
    agent.wait_for_connected().await;

    let identity = harness.controller.identity_of(agent.service_id()).await;
    let spiffe_id = identity
        .spiffe_id
        .expect("SPIFFE ID must be present when trust domain is configured");

    assert_eq!(
        spiffe_id,
        format!(
            "spiffe://controller.test.local/service/{}",
            agent.service_id()
        ),
        "SPIFFE URI must match the configured trust domain and service ID"
    );
}

// The wrong-trust-domain rejection path is tested at the unit level in
// `controller-runtime/src/cert_signer.rs::spiffe_san_wrong_trust_domain_rejected`.
// No Docker integration test is needed: the rejection happens synchronously
// inside `RcgenAgentCertSigner::sign_agent_csr` before any network traffic.
````

- [ ] **Step 2: Verify the test file compiles:**

```bash
cargo check -p uptrakit-integration-tests --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit:**

```bash
git add crates/core/integration-tests/tests/spiffe_identity.rs
git commit -m "feat(integration-tests): implement agent_enrolls_and_authenticates_via_spiffe_san; demote wrong-trust-domain test to cert_signer.rs unit test"
```

---

## Task 8: Implement cert rotation integration test

**Files:**

- Modify: `crates/core/integration-tests/tests/cert_rotation_hot_swap.rs`

The test verifies: cert renewal via the resolver hot-swap path does NOT disrupt the existing session (service
stays `Approved` throughout), and the new cert is presented on the next TLS handshake (verified by checking cert
serial changes after a forced disconnect + reconnect).

- [ ] **Step 1: Replace the entire content of `crates/core/integration-tests/tests/cert_rotation_hot_swap.rs` with:**

````rust
//! End-to-end verification that Agent cert renewal via the resolver hot-swap
//! path keeps the existing WebSocket session alive, then presents the new cert
//! on the next TLS handshake.
//!
//! Run:
//! ```sh
//! docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
//! cargo test -p uptrakit-integration-tests cert_rotation_hot_swap -- --ignored --nocapture
//! ```

use crate::helpers::agent_controller_harness::{AgentControllerHarness, HarnessOptions};

/// Verify that cert renewal via the resolver hot-swap path keeps the session
/// alive and the new cert is presented on the next TLS handshake.
///
/// Sequence:
/// 1. Spawn agent, wait for it to be Approved.
/// 2. Record the current cert serial (before renewal).
/// 3. Trigger renewal via test-utils endpoint.
/// 4. Wait for the cert serial to change in the controller DB (hot-swap complete).
/// 5. Verify the service is still Approved (no session disruption during renewal).
/// 6. Force a disconnect to trigger a new TLS handshake.
/// 7. Wait for the agent to reconnect.
/// 8. Verify the serial is still the new value (new cert in use).
#[tokio::test]
#[ignore = "requires Docker"]
async fn agent_cert_renewal_via_resolver_keeps_session_alive() {
    let harness = AgentControllerHarness::start_with(HarnessOptions {
        trust_domain: String::new(),
    })
    .await;

    let agent = harness.spawn_agent("agent-1").await;
    agent.wait_for_connected().await;

    let before_serial = agent.cert_serial_number().await;

    harness
        .controller
        .request_cert_renewal(agent.service_id())
        .await;

    // Hot-swap complete: cert serial changed in the controller DB.
    agent.wait_for_cert_renewed(&before_serial).await;

    // Session must still be alive — service stays Approved without reconnecting.
    harness
        .controller
        .wait_for_service_approved(agent.service_id())
        .await;

    // Force a new TLS handshake by closing the WebSocket.
    harness
        .controller
        .disconnect_service(agent.service_id())
        .await;

    // Agent reconnects automatically; wait for it to be Approved again.
    harness
        .controller
        .wait_for_service_approved(agent.service_id())
        .await;

    let after_serial = agent.cert_serial_number().await;
    assert_ne!(
        before_serial, after_serial,
        "cert serial must differ after renewal and reconnect"
    );
}
````

- [ ] **Step 2: Verify the test file compiles:**

```bash
cargo check -p uptrakit-integration-tests --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit:**

```bash
git add crates/core/integration-tests/tests/cert_rotation_hot_swap.rs
git commit -m "feat(integration-tests): implement agent_cert_renewal_via_resolver_keeps_session_alive"
```

---

## Task 9: Full quality gates

- [ ] **Step 1: Full cargo check (both feature sets):**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | grep -E "^error" | head -20
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 2: Full clippy:**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error\|^warning" | grep -v "^warning.*unused import" | head -30
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error\|^warning" | grep -v "^warning.*unused import" | head -30
```

Expected: no errors. Any new clippy warnings must be addressed before merging.

- [ ] **Step 3: Full test suite (unit + integration, no Docker):**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all tests pass. The new `#[ignore = "requires Docker"]` tests are skipped.

- [ ] **Step 4: cargo deny:**

```bash
cargo deny check
```

Expected: no violations.

- [ ] **Step 5: Markdown lint:**

```bash
npx markdownlint --config .markdownlint.json '**/*.md'
```

Expected: no errors.

- [ ] **Step 6: Build the Docker test image with test-utils:**

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
```

Expected: image builds successfully with `test-utils` feature compiled in.

- [ ] **Step 7: Run all Docker integration tests including the newly unblocked tests:**

```bash
cargo test -p uptrakit-integration-tests -- --ignored --nocapture 2>&1 | tail -50
```

Expected output includes:

- `test agent_enrolls_and_authenticates_via_spiffe_san ... ok`
- `test agent_cert_renewal_via_resolver_keeps_session_alive ... ok`
- All previously passing tests still pass.

- [ ] **Step 8: Run reverse-proxy integration tests (unchanged but re-verify with P-384 keys from Plan A):**

```bash
cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored --nocapture 2>&1 | tail -20
```

Expected: all reverse_proxy tests pass.
