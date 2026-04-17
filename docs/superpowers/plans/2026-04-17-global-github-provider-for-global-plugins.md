# Global GitHub Provider For Global Plugins Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a host-owned global GitHub provider client for global plugins,
wire it into `dashboard-icons`, and remove the current branch-only fallback
that leaks global GitHub settings into tenant-scoped plugin materialization.

**Architecture:** Add a provider-agnostic lookup seam to singleton plugin
construction, define a GitHub-specific shared contract in its own crate,
implement the `octocrab`-backed runtime in `uptrakit-web-api`, and migrate
`dashboard-icons` to consume the injected handle. Keep tenant-scoped plugins on
plugin-local config only, and centralize auth, validated custom base URLs,
retry, rate limits,
invalidation, and diagnostics in the host layer.

**Tech Stack:** Rust, Tokio, SeaORM, reqwest, octocrab, tower, Axum integration tests, markdownlint

---

## File Structure

### New files

- Create: `crates/shared/global-github-provider/Cargo.toml`
- Create: `crates/shared/global-github-provider/src/lib.rs`
- Create: `crates/ui/web-api/src/global_providers/mod.rs`
- Create: `crates/ui/web-api/src/global_providers/github.rs`
- Create: `crates/ui/web-api/src/global_providers/tests.rs`

### Modified files

- Modify: `Cargo.toml`
- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/macros.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`
- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/plugins/enhancements/dashboard-icons/Cargo.toml`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/cache.rs`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`
- Modify: `crates/shared/db/Cargo.toml`
- Modify: `crates/shared/db/src/provider_settings.rs`
- Modify: `crates/shared/web-api-types/src/events.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/lib.rs`
- Modify: `crates/ui/web-api/src/routes/settings_provider_github.rs`
- Modify: `crates/ui/web-api/src/integration_tests/settings.rs`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Modify: `crates/ui/web-api/src/routes/system_alerts.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_triggers.rs`
- Modify: `crates/shared/scheduler-engine/src/executors/fetch_releases.rs`
- Modify: `crates/ui/web-api/src/api_error/mappings.rs`

### Test files

- Test: `crates/plugins/infrastructure/core/src/catalog.rs`
- Test: `crates/shared/global-github-provider/src/lib.rs`
- Test: `crates/ui/web-api/src/global_providers/tests.rs`
- Test: `crates/plugins/enhancements/dashboard-icons/src/cache.rs`
- Test: `crates/ui/web-api/src/integration_tests/settings.rs`

---

### Task 1: Add The Generic Singleton Provider Lookup Seam

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/macros.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`
- Test: `crates/plugins/infrastructure/core/src/catalog.rs`

- [ ] **Step 1: Write the failing metadata and lookup tests**

Add catalog-core tests covering the new provider consumer declaration and lookup defaults:

```rust
#[test]
fn catalog_config_defaults_without_global_provider_lookup() {
    let cfg = CatalogConfig::default();
    assert!(cfg.global_provider_lookup.is_none());
}

#[test]
fn descriptor_exposes_global_provider_consumers() {
    assert_eq!(
        TEST_LIFECYCLE_DESCRIPTOR.global_provider_consumers,
        &[GlobalProviderConsumerDecl::new("github")]
    );
}
```

- [ ] **Step 2: Run the targeted plugin-core tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core \
  catalog_config_defaults_without_global_provider_lookup -- --nocapture
cargo test -p uptrakit-plugin-infrastructure-core \
  descriptor_exposes_global_provider_consumers -- --nocapture
```

Expected: FAIL with missing `global_provider_lookup` / `global_provider_consumers` symbols.

- [ ] **Step 3: Add the generic-core lookup and metadata types**

Implement the provider-agnostic seam in `descriptor.rs`:

```rust
pub trait GlobalProviderLookup: Send + Sync {
    fn get(
        &self,
        provider_id: &'static str,
    ) -> Option<Arc<dyn std::any::Any + Send + Sync>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalProviderConsumerDecl {
    pub provider_id: &'static str,
}

impl GlobalProviderConsumerDecl {
    pub const fn new(provider_id: &'static str) -> Self {
        Self { provider_id }
    }
}

pub struct CatalogConfig {
    pub allow_private_urls: bool,
    pub http_client: Option<reqwest::Client>,
    pub cancellation_token: Option<CancellationToken>,
    pub global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
}

pub struct PluginDescriptor {
    // existing metadata fields...
    pub global_provider_consumers: &'static [GlobalProviderConsumerDecl],
}
```

Keep `GlobalProviderLookup` and `CatalogConfig.global_provider_lookup`
always-compiled. Unlike `http_client` and `cancellation_token`, the provider
lookup field must not be gated on the `catalog` feature because singleton
construction needs the same typed seam in every build that instantiates
`CatalogConfig`.

- [ ] **Step 4: Extend `declare_plugin!` to declare global provider consumers**

Add a new optional macro arm in `macros.rs`:

```rust
$(, global_provider_consumers: [ $( $provider_id:expr ),+ $(,)? ] )?
```

and populate the descriptor field:

```rust
global_provider_consumers: $crate::__or_empty_slice!(
    $(
        &[
            $(
                $crate::GlobalProviderConsumerDecl::new($provider_id)
            ),*
        ]
    )?
),
```

- [ ] **Step 5: Thread the field through catalog and re-exports**

Expose the new types in `lib.rs` and keep `CatalogConfig::default()` valid:

```rust
pub use descriptor::{CatalogConfig, GlobalProviderConsumerDecl, GlobalProviderLookup};
```

with default:

```rust
global_provider_lookup: None,
```

- [ ] **Step 6: Run the targeted plugin-core tests again**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core \
  catalog_config_defaults_without_global_provider_lookup -- --nocapture
cargo test -p uptrakit-plugin-infrastructure-core \
  descriptor_exposes_global_provider_consumers -- --nocapture
```

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add \
  crates/plugins/infrastructure/core/src/descriptor.rs \
  crates/plugins/infrastructure/core/src/macros.rs \
  crates/plugins/infrastructure/core/src/catalog.rs \
  crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugins): add global provider lookup seam"
```

---

### Task 2: Create The Shared GitHub Provider Contract Crate

**Files:**

- Create: `crates/shared/global-github-provider/Cargo.toml`
- Create: `crates/shared/global-github-provider/src/lib.rs`
- Test: `crates/shared/global-github-provider/src/lib.rs`

- [ ] **Step 1: Write the failing contract tests**

Add minimal unit tests for the exported constants and error classification:

```rust
#[test]
fn dashboard_icons_consumer_id_is_stable() {
    assert_eq!(DASHBOARD_ICONS.as_str(), "dashboard-icons");
}

#[test]
fn github_provider_error_auth_failed_is_not_retryable() {
    assert!(!GitHubProviderError::AuthFailed("bad token".into()).is_retryable());
}
```

- [ ] **Step 2: Run the new crate tests to verify they fail**

Run: `cargo test -p uptrakit-global-github-provider -- --nocapture`

Expected: FAIL because the crate and symbols do not exist yet.

- [ ] **Step 3: Create the crate and define the public contract**

Create `Cargo.toml` and `src/lib.rs` with the shared types. Add
`async-trait = { workspace = true }` to the new crate manifest and keep the
trait object-safe with `#[async_trait::async_trait]`. Also add
`uptrakit-plugin-infrastructure-core = { workspace = true }` because the crate
owns the typed lookup helper for `CatalogConfig`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalProviderConsumerId(&'static str);

impl GlobalProviderConsumerId {
    pub const fn new(value: &'static str) -> Self { Self(value) }
    pub const fn as_str(&self) -> &'static str { self.0 }
}

pub const DASHBOARD_ICONS: GlobalProviderConsumerId =
    GlobalProviderConsumerId::new("dashboard-icons");

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitHubProviderError {
    Throttled,
    AuthFailed(String),
    UpstreamUnavailable(String),
    RequestFailed(String),
    Misconfigured(String),
}

impl GitHubProviderError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Throttled | Self::UpstreamUnavailable(_) | Self::RequestFailed(_))
    }
}

#[async_trait::async_trait]
pub trait GitHubProviderClient: Send + Sync {
    async fn fetch_repository_tree(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<GitHubRepositoryTree, GitHubProviderError>;
}
```

- [ ] **Step 4: Define the response model that keeps `octocrab` out of plugins**

In the same crate, add an Uptrakit-owned tree model, the concrete tree-entry
kind enum, and the typed lookup helper:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepositoryTree {
    pub truncated: bool,
    pub entries: Vec<GitHubTreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubTreeEntry {
    pub path: String,
    pub kind: GitHubTreeEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubTreeEntryKind {
    Blob,
    Tree,
}

pub struct GitHubProviderHandle {
    client: Arc<dyn GitHubProviderClient>,
}

impl GitHubProviderHandle {
    pub fn new(client: Arc<dyn GitHubProviderClient>) -> Self {
        Self { client }
    }

    pub fn client(&self) -> Arc<dyn GitHubProviderClient> {
        Arc::clone(&self.client)
    }
}

pub fn lookup_github_provider(
    config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> Option<Arc<dyn GitHubProviderClient>> {
    let lookup = config.global_provider_lookup.as_ref()?;
    let handle = lookup.get("github")?.downcast::<GitHubProviderHandle>().ok()?;
    Some(handle.client())
}
```

- [ ] **Step 5: Run the crate tests again**

Run: `cargo test -p uptrakit-global-github-provider -- --nocapture`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/shared/global-github-provider
git commit -m "feat(provider): add shared GitHub provider contract"
```

---

### Task 3: Build The `octocrab`-Backed Provider Runtime And Invalidation Path

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/ui/web-api/Cargo.toml`
- Modify: `crates/shared/db/Cargo.toml`
- Create: `crates/ui/web-api/src/global_providers/mod.rs`
- Create: `crates/ui/web-api/src/global_providers/github.rs`
- Create: `crates/ui/web-api/src/global_providers/tests.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/lib.rs`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/shared/db/src/provider_settings.rs`
- Modify: `crates/ui/web-api/src/routes/settings_provider_github.rs`
- Modify: `crates/ui/web-api/src/integration_tests/settings.rs`

- [ ] **Step 1: Write the failing runtime tests**

Add tests that pin the runtime behavior before implementation:

```rust
#[tokio::test]
async fn invalidation_rebuilds_client_generation() {
    let runtime = test_runtime_with_factory(valid_defaults(), fake_factory());
    let _first = runtime.github_client().await.unwrap();
    let first_generation = runtime.cached_generation_for_tests().unwrap();

    update_defaults(runtime.db(), rotated_defaults()).await;
    runtime.invalidate();

    let _second = runtime.github_client().await.unwrap();
    let second_generation = runtime.cached_generation_for_tests().unwrap();
    assert_ne!(first_generation, second_generation);
}

#[tokio::test(start_paused = true)]
async fn other_instances_recheck_generation_within_thirty_seconds() {
    let (writer, reader) = paired_test_runtimes(valid_defaults(), fake_factory());
    let _ = reader.github_client().await.unwrap();
    let first_generation = reader.cached_generation_for_tests().unwrap();

    update_defaults(writer.db(), rotated_defaults()).await;
    writer.invalidate();

    tokio::time::advance(Duration::from_secs(31)).await;
    let _ = reader.github_client().await.unwrap();

    assert_ne!(first_generation, reader.cached_generation_for_tests().unwrap());
}

#[tokio::test]
async fn queue_wait_times_out_as_throttled() {
    let runtime = test_runtime_with_concurrency_limit(1);
    let _held = runtime.acquire_request_permit_for_tests().await;
    let err = runtime.github_client().await.unwrap()
        .fetch_repository_tree(DASHBOARD_ICONS, "o", "r", "main", true)
        .await
        .unwrap_err();
    assert!(matches!(err, GitHubProviderError::Throttled));
}

#[tokio::test]
async fn metrics_record_requests_retries_auth_failures_and_cooldowns() {
    let runtime = test_runtime_with_metrics(fake_metrics_recorder());
    assert_metric_labels(runtime, DASHBOARD_ICONS);
}

#[tokio::test]
async fn cooldown_is_shared_across_two_global_consumers() {
    let runtime = test_runtime_with_secondary_limit();
    assert_shared_cooldown(runtime, DASHBOARD_ICONS, TEST_GLOBAL_CONSUMER);
}

#[tokio::test]
async fn missing_global_credentials_builds_anonymous_public_client() {
    let runtime = test_runtime_without_global_defaults(fake_factory());
    let client = runtime.github_client().await.unwrap();
    client
        .fetch_repository_tree(DASHBOARD_ICONS, "homarr-labs", "dashboard-icons", "main", true)
        .await
        .unwrap();
    assert!(runtime.cached_generation_for_tests().is_some());
}
```

- [ ] **Step 2: Run the targeted runtime tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  global_providers -- --nocapture
```

Expected: FAIL because the runtime module and invalidation hooks do not exist.

- [ ] **Step 3: Wire the new runtime dependencies into the manifests**

Update the workspace and web-api manifests before implementation:

- add `octocrab = "0.49.7"` and `arc-swap = "1.9.1"` to the workspace
  dependencies in `Cargo.toml`
- add `metrics = "0.24.3"` to the workspace dependencies in `Cargo.toml`
- add `metrics-util = "0.20.1"` to the workspace dependencies in `Cargo.toml`
  for the runtime test recorder
- add `uptrakit-global-github-provider = { path = "crates/shared/global-github-provider" }`
  to `Cargo.toml` workspace dependencies
- add `octocrab`, `arc-swap`, and `metrics` to
  `crates/ui/web-api/Cargo.toml`
- add `uptrakit-global-github-provider = { workspace = true }` to
  `crates/ui/web-api/Cargo.toml`
- add `metrics-util` to `crates/ui/web-api/Cargo.toml` dev-dependencies
- add `sha2 = { workspace = true }` to `crates/shared/db/Cargo.toml`
- keep `sha2` and `async-trait` on workspace dependencies already in use

- [ ] **Step 4: Add provider-settings helpers for invariants and derived generation**

Extend `provider_settings.rs` with canonicalization and generation helpers:

```rust
pub fn normalize_github_provider_defaults(
    defaults: GitHubProviderDefaults,
) -> Result<Option<GitHubProviderDefaults>> { /* enforce the spec invariants */ }

pub fn github_provider_generation(defaults: &GitHubProviderDefaults) -> [u8; 32] {
    let mut hasher = sha2::Sha256::new();
    hasher.update(defaults.api_base_url.as_deref().unwrap_or_default().as_bytes());
    hasher.update([0]);
    hasher.update(defaults.auth_token.as_deref().unwrap_or_default().as_bytes());
    hasher.finalize().into()
}
```

- [ ] **Step 5: Implement the provider runtime module**

Create `global_providers/github.rs` with the state holder:

```rust
pub struct GitHubProviderRuntime {
    db: DatabaseConnection,
    client_slot: arc_swap::ArcSwapOption<GitHubProviderRuntimeState>,
    generation_recheck_interval: Duration,
    last_generation_check: parking_lot::Mutex<tokio::time::Instant>,
}

impl GitHubProviderRuntime {
    pub fn invalidate(&self) {
        self.client_slot.store(None);
    }

    pub async fn github_client(&self) -> Result<Arc<dyn GitHubProviderClient>, GitHubProviderError> {
        // lazily rebuild on local invalidation and re-check stored generation
        // on acquisition after the 30s bounded staleness window
    }
}

#[async_trait::async_trait]
impl GitHubProviderClient for GitHubProviderRuntime {
    async fn fetch_repository_tree(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<GitHubRepositoryTree, GitHubProviderError> {
        self.github_client()
            .await?
            .fetch_repository_tree(consumer, owner, repo, git_ref, recursive)
            .await
    }
}
```

Keep the test seams explicit instead of implying hidden methods:

- add `cached_generation_for_tests()` behind `#[cfg(test)]`
- add `acquire_request_permit_for_tests()` behind `#[cfg(test)]`
- inject a fake client factory in tests instead of reaching into `octocrab`

- [ ] **Step 6: Build the validated-URL `octocrab` client and bounded policy**

In `github.rs`, build the client with public-HTTPS base-URL validation and queue bound:

```rust
let octocrab = octocrab::OctocrabBuilder::new()
    .base_uri(base_url)?
    .personal_token(token.clone())
    .build()?;
```

and gate requests with:

```rust
let permit = tokio::time::timeout(Duration::from_secs(30), semaphore.acquire()).await;
```

Also make the runtime own:

- bounded exponential retry for retryable provider errors
- one shared cooldown state for the process-wide V1 provider runtime
- provider-level `metrics` counters and gauges:
  - `uptrakit_global_provider_requests_total`
  - `uptrakit_global_provider_retries_total`
  - `uptrakit_global_provider_cooldown_seconds`
  - `uptrakit_global_provider_auth_failures_total`

- [ ] **Step 7: Wire the runtime into `AppState` and controller startup**

Add the new state:

```rust
pub struct GlobalProviderState {
    pub github: Arc<GitHubProviderRuntime>,
}

impl GlobalProviderLookup for GlobalProviderState {
    fn get(
        &self,
        provider_id: &'static str,
    ) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        match provider_id {
            "github" => Some(Arc::new(GitHubProviderHandle::new(self.github.clone()))),
            _ => None,
        }
    }
}
```

and initialize it in `crates/core/controller/src/main.rs` before the catalog is built.

- [ ] **Step 8: Invalidate the runtime from the settings route**

In `update_github_provider_settings`, invalidate after a successful write:

```rust
state.global_providers.github.invalidate();
```

Add the integration test:

```rust
#[tokio::test]
async fn github_provider_settings_update_invalidates_runtime() {
    // set once, fetch runtime generation, update token, fetch again, assert changed
}
```

- [ ] **Step 9: Run the targeted runtime and settings tests**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  global_providers -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  github_provider_settings -- --nocapture
```

Expected: PASS

- [ ] **Step 10: Commit**

```bash
git add \
  Cargo.toml \
  crates/ui/web-api/Cargo.toml \
  crates/shared/db/Cargo.toml \
  crates/shared/db/src/provider_settings.rs \
  crates/ui/web-api/src/global_providers \
  crates/ui/web-api/src/app_state.rs \
  crates/ui/web-api/src/lib.rs \
  crates/ui/web-api/src/routes/settings_provider_github.rs \
  crates/ui/web-api/src/integration_tests/settings.rs \
  crates/core/controller/src/main.rs
git commit -m "feat(provider): add global GitHub provider runtime"
```

---

### Task 4: Migrate `dashboard-icons` To The Injected Provider Handle

**Files:**

- Modify: `crates/plugins/enhancements/dashboard-icons/Cargo.toml`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/cache.rs`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`
- Test: `crates/plugins/enhancements/dashboard-icons/src/cache.rs`

- [ ] **Step 1: Write the failing `dashboard-icons` provider tests**

Add tests for provider-backed refresh and stale-cache-on-error:

```rust
#[tokio::test]
async fn refresh_uses_injected_github_provider_tree() {
    let cache = DashboardIconCache::new(fake_provider_with_paths(&["svg/nginx.svg"]));
    cache.refresh().await.unwrap();
    assert!(cache.lookup("Nginx").is_some());
}

#[tokio::test]
async fn failed_refresh_keeps_existing_cache_contents() {
    let cache = DashboardIconCache::new(fake_provider_with_paths(&["svg/nginx.svg"]));
    cache.refresh().await.unwrap();
    cache.set_provider(fake_provider_with_error(GitHubProviderError::AuthFailed("bad".into())));
    assert!(cache.refresh().await.is_err());
    assert!(cache.lookup("Nginx").is_some());
}
```

- [ ] **Step 2: Run the targeted dashboard-icons tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-plugin-enhancement-dashboard-icons \
  refresh_uses_injected_github_provider_tree -- --nocapture
cargo test -p uptrakit-plugin-enhancement-dashboard-icons \
  failed_refresh_keeps_existing_cache_contents -- --nocapture
```

Expected: FAIL because the cache still uses raw `reqwest`.

- [ ] **Step 3: Replace raw GitHub tree fetches with the shared provider contract**

Add `uptrakit-global-github-provider = { workspace = true }` to
`crates/plugins/enhancements/dashboard-icons/Cargo.toml` and refactor
`DashboardIconCache`:

```rust
pub struct DashboardIconCache {
    slugs: RwLock<HashMap<String, IconVariants>>,
    github: Arc<dyn GitHubProviderClient>,
}

pub async fn refresh(&self) -> Result<usize> {
    let tree = self.github
        .fetch_repository_tree(DASHBOARD_ICONS, "homarr-labs", "dashboard-icons", "main", true)
        .await?;
    // map entries into slug variants
}
```

- [ ] **Step 4: Inject the provider into the singleton constructor**

Update `create_dashboard_icons_lifecycle`:

```rust
let github = lookup_github_provider(config)
    .ok_or_else(|| PluginError::PluginInternal("global github provider lookup not wired".into()))?;
let cache = Arc::new(DashboardIconCache::new(github));
```

and declare the consumer in `declare_plugin!`:

```rust
global_provider_consumers: ["github"],
```

The provider runtime remains responsible for unauthenticated fallback. When no
global credentials record exists, it still injects a public-GitHub handle, so
`dashboard-icons` does not branch on authentication mode.

- [ ] **Step 5: Keep the refresh loop behavior but remove the direct tree URL**

Delete the hardcoded API URL usage from `cache.rs`, keep the CDN URL logic, and preserve the periodic refresh loop:

```rust
if let Err(e) = cache.refresh().await {
    tracing::warn!(error = %e, "dashboard icons refresh failed");
}
```

- [ ] **Step 6: Run the dashboard-icons tests again**

Run: `cargo test -p uptrakit-plugin-enhancement-dashboard-icons -- --nocapture`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add \
  crates/plugins/enhancements/dashboard-icons/Cargo.toml \
  crates/plugins/enhancements/dashboard-icons/src/cache.rs \
  crates/plugins/enhancements/dashboard-icons/src/plugin.rs
git commit -m "feat(dashboard-icons): use injected global GitHub provider"
```

---

### Task 5: Remove The Branch-Only Global Fallback From Tenant-Scoped Plugin Paths

**Files:**

- Modify: `crates/shared/db/src/provider_settings.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_triggers.rs`
- Modify: `crates/shared/scheduler-engine/src/executors/fetch_releases.rs`
- Modify: `crates/ui/web-api/src/api_error/mappings.rs`
- Test: `crates/ui/web-api/src/integration_tests/settings.rs`

- [ ] **Step 1: Write the failing regression test that proves tenant-scoped paths ignore the global provider**

Add an integration or query-level regression test:

```rust
#[tokio::test]
async fn releases_github_paths_ignore_global_provider_defaults() {
    // configure global github settings
    // build a tenant-scoped github assignment with empty local auth
    // assert the materialized config remains plugin-local and does not inherit the global token
}
```

- [ ] **Step 2: Run the regression test to verify it fails**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  releases_github_paths_ignore_global_provider_defaults -- --nocapture
```

Expected: FAIL because the current branch still applies shared defaults to regular plugin paths.

- [ ] **Step 3: Delete the branch-only shared fallback helpers from `provider_settings.rs`**

Remove the regular-plugin applicability helpers:

```rust
// delete:
pub fn supports_github_provider_defaults(...)
pub fn apply_github_provider_defaults_for_plugin(...)
async fn provider_settings_blank_and_missing_fields_fallback()
async fn provider_settings_non_opt_in_plugin_bypass()
```

Retain only the storage and invariant helpers.

- [ ] **Step 4: Strip the default-merging calls out of tenant-scoped materialization**

In the controller fetch, update-dispatch, version-check, and replay paths, revert to plain effective config merging:

```rust
let merged = uptrakit_config_merge::resolve_effective_config(
    None,
    config_model.map(|c| &c.config),
    assignment.config.as_ref(),
);
```

and remove the `github_provider_defaults` plumbing from `ValidatedUpdateTarget`.

- [ ] **Step 5: Remove the now-dead provider-settings error mapping from tenant-scoped update dispatch**

Delete the unused error variant and mapping:

```rust
// remove TriggerUpdateError::ProviderSettings
// remove HandlerError::ProviderSettings
```

and simplify any callers that only existed to load global GitHub defaults.

- [ ] **Step 6: Run the regression and targeted compile tests again**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  releases_github_paths_ignore_global_provider_defaults -- --nocapture
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons
cargo check -p uptrakit-web-api-queries --no-default-features --features db-sqlite
cargo check -p uptrakit-scheduler-engine
```

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add \
  crates/shared/db/src/provider_settings.rs \
  crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs \
  crates/ui/web-api/src/routes/software_items/mod.rs \
  crates/ui/web-api/src/routes/service_ws/handler/mod.rs \
  crates/ui/web-api/src/routes/service_ws/handler/updates.rs \
  crates/ui/web-api-queries/src/queries/update_dispatch.rs \
  crates/ui/web-api-queries/src/queries/update_triggers.rs \
  crates/shared/scheduler-engine/src/executors/fetch_releases.rs \
  crates/ui/web-api/src/api_error/mappings.rs \
  crates/ui/web-api/src/integration_tests/settings.rs
git commit -m "fix(provider): keep global GitHub settings out of tenant plugin paths"
```

---

### Task 6: Add Diagnostics, Final Verification, And Cleanup

**Files:**

- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/shared/web-api-types/src/events.rs`
- Modify: `crates/ui/web-api/src/routes/system_alerts.rs`
- Modify: `crates/ui/web-api/src/routes/settings_provider_github.rs`
- Modify: `crates/ui/web-api/src/integration_tests/settings.rs`
- Modify: `crates/ui/web-api/src/global_providers/tests.rs`

- [ ] **Step 1: Write the failing diagnostics tests**

Add one alert-path assertion and one reload-path admin-event assertion:

```rust
#[tokio::test]
async fn system_alerts_include_invalid_global_github_provider_record() {
    let app = TestApp::new().await;
    seed_invalid_github_provider_record(app.db()).await;
    let alerts = get_system_alerts(&app).await;
    assert!(alerts.iter().any(|alert| alert.id == "global_github_provider_invalid"));
}

#[tokio::test]
async fn updating_github_provider_settings_broadcasts_admin_diagnostic_event() {
    let app = TestApp::new().await;
    let tenant = Uuid::now_v7();
    let mut rx = app.state.notification.event_broadcaster.subscribe(tenant).await;
    put_invalid_github_provider_settings(&app).await;
    let event = recv_admin_event(&mut rx).await;
    assert!(matches!(event, AdminEvent::GlobalGitHubProviderMisconfigured { .. }));
}
```

- [ ] **Step 2: Run the targeted diagnostics tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  system_alerts_include_invalid_global_github_provider_record -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  updating_github_provider_settings_broadcasts_admin_diagnostic_event -- --nocapture
```

Expected: FAIL because the startup/admin diagnostic path does not exist yet.

- [ ] **Step 3: Add the startup logs and system-alert diagnostics**

Implement one shared diagnostics helper that both controller startup and the
settings-write path call:

```rust
if let Some(problem) = detect_global_github_provider_problem(&db_conn).await? {
    tracing::warn!(problem = %problem, "global GitHub provider misconfiguration");
    state.notification.event_broadcaster
        .send_global(AdminEvent::GlobalGitHubProviderMisconfigured {
            problem: problem.to_string(),
        })
        .await;
}
```

Add the new `AdminEvent::GlobalGitHubProviderMisconfigured { problem: String }`
variant in `crates/shared/web-api-types/src/events.rs`, keep its `event_name()`
stable, and reuse the same helper after `update_github_provider_settings`
invalidates the runtime so settings reload emits the same push-style diagnostic.

Extend `routes/system_alerts.rs` to append a `SystemAlert` entry for invalid
global GitHub record state.

- [ ] **Step 4: Run the full targeted verification suite**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core -- --nocapture
cargo test -p uptrakit-global-github-provider -- --nocapture
cargo test -p uptrakit-plugin-enhancement-dashboard-icons -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  github_provider_settings -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  global_providers -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  system_alerts_include_invalid_global_github_provider_record -- --nocapture
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons \
  updating_github_provider_settings_broadcasts_admin_diagnostic_event -- --nocapture
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite,dashboard-icons
cargo check -p uptrakit-web-api-queries --no-default-features --features db-sqlite
cargo check -p uptrakit-scheduler-engine
cargo check -p uptrakit-controller --no-default-features --features db-sqlite,dashboard-icons
python3 ci/verify_db_access_policy.py
cargo fmt --all --check
```

Expected: all commands succeed.

- [ ] **Step 5: Commit**

```bash
git add \
  crates/core/controller/src/main.rs \
  crates/shared/web-api-types/src/events.rs \
  crates/ui/web-api/src/routes/settings_provider_github.rs \
  crates/ui/web-api/src/routes/system_alerts.rs \
  crates/ui/web-api/src/integration_tests/settings.rs \
  crates/ui/web-api/src/global_providers/tests.rs
git commit -m "test(provider): add diagnostics and final verification coverage"
```

---

## Spec Coverage Check

- Global-only GitHub credential record: covered in Tasks 3 and 6.
- Provider-agnostic singleton lookup seam: covered in Task 1.
- GitHub-specific shared contract outside generic core: covered in Task 2.
- `octocrab` behind host abstraction with validated custom base URLs and
  rate-limit policy: covered in Task 3.
- `dashboard-icons` migration with unauthenticated fallback via injected handle: covered in Task 4.
- Removal of tenant-scoped global fallback: covered in Task 5.
- 30-second multi-instance revalidation and process-local invalidation: covered in Task 3.
- Shared cooldown across two global consumers: covered in Task 3.
- Unauthenticated public-GitHub fallback when no global record exists: covered in Task 3.
- Provider metrics, diagnostics, and invalidation behavior for invalid global
  record states: covered in Tasks 3 and 6.

## Placeholder Scan

- No `TBD`, `TODO`, or deferred implementation placeholders remain in task steps.
- Every task names concrete files and verification commands.
- Every commit step has an explicit commit message.

## Type Consistency Check

- Generic lookup seam uses `GlobalProviderLookup` and `Arc<dyn Any + Send + Sync>` consistently.
- GitHub-specific shared contract uses `GitHubProviderClient`, `GlobalProviderConsumerId`, `GitHubRepositoryTree`, and `GitHubProviderError` consistently.
- The singleton injection seam is consistently `CatalogConfig`.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-17-global-github-provider-for-global-plugins.md`.

Two execution options:

1. Subagent-Driven (recommended) - I dispatch a fresh subagent per task, review between tasks, fast iteration
2. Inline Execution - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
