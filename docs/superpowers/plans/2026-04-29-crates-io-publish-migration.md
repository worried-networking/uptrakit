# crates.io Publish Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete vendored `generated/` snapshots from `service-sdk` and `openapi-client`, publish 7 shared crates
to crates.io, and remove the sync infrastructure that kept those snapshots fresh.

**Architecture:** Five sequential commits on a single PR. Each commit is independently buildable and passes all
quality gates before the next begins. The 5 shared type crates (`uptrakit-shared-macros`, `uptrakit-surfaces`,
`uptrakit-shared-types`, `uptrakit-wire`, `uptrakit-web-api-types`) gain `publish = true` first; then both SDK
crates drop their `workspace-internal` feature and vendored copies; then the sync infrastructure (xtask,
pre-commit hook, CI workflow) is deleted; finally `release-plz.toml` is updated to enable crates.io publishing.
Bootstrap (manual `cargo publish` for all 7) happens post-merge.

**Tech Stack:** Rust / Cargo workspace, release-plz, husky pre-commit hooks, GitHub Actions

---

## File Map

### Task 1 (Commit 1) — 5 shared crates

| Action | Path |
| ------ | ---- |
| Modify | `crates/shared/macros/Cargo.toml` |
| Modify | `crates/shared/surfaces/Cargo.toml` |
| Modify | `crates/shared/types/Cargo.toml` |
| Modify | `crates/shared/wire/Cargo.toml` |
| Modify | `crates/shared/web-api-types/Cargo.toml` |

### Task 2 (Commit 2) — service-sdk

| Action | Path |
| ------ | ---- |
| Modify | `crates/shared/service-sdk/Cargo.toml` |
| Modify | `crates/shared/service-sdk/src/lib.rs` |
| Delete | `crates/shared/service-sdk/src/generated/` |

### Task 3 (Commit 3) — openapi-client

| Action | Path |
| ------ | ---- |
| Modify | `crates/shared/openapi-client/Cargo.toml` |
| Modify | `crates/shared/openapi-client/src/lib.rs` |
| Modify | `crates/shared/openapi-client/src/error.rs` |
| Delete | `crates/shared/openapi-client/src/macros.rs` |
| Delete | `crates/shared/openapi-client/src/generated/` |

### Task 4 (Commit 4) — sync infrastructure

| Action | Path |
| ------ | ---- |
| Delete | `xtask/` (entire directory) |
| Modify | `Cargo.toml` (workspace root — remove `"xtask"` member) |
| Modify | `.husky/pre-commit` |
| Delete | `.github/workflows/generated-check.yml` |

### Task 5 (Commit 5) — release-plz

| Action | Path |
| ------ | ---- |
| Modify | `release-plz.toml` |

---

## Quality gates (run after every commit)

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

> **Note on `cargo package --dry-run`:** Only the 3 leaf crates (macros, surfaces, shared-types) can be validated
> pre-bootstrap because `cargo package` resolves their deps from crates.io. Wire, web-api-types, service-sdk, and
> openapi-client depend on crates not yet on crates.io and will fail dry-run until after bootstrap.
> Run the leaf-crate dry-runs after Task 1 only.

---

## Task 1: Commit 1 — 5 shared crates: independent versions + publish

Give each of the 5 shared crates an explicit `version = "0.0.1"` (replacing `version.workspace = true`) and
`publish = true` (overriding workspace `publish = ["uptrakit-private"]`). Also move `rootcause` in
`uptrakit-shared-macros` from `[dependencies]` to `[dev-dependencies]` — the macros reference `rootcause::` at
the call site scope, not within the crate itself, so it should not be a runtime transitive dep.

No `.rs` source changes in this task.

- [ ] **Step 1: Edit `crates/shared/macros/Cargo.toml`**

  Replace the entire file with:

  ```toml
  [package]
  name = "uptrakit-shared-macros"
  description = "Uptrakit shared procedural macros"
  version = "0.0.1"
  publish = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  edition.workspace = true

  [dev-dependencies]
  rootcause.workspace = true
  thiserror.workspace = true

  [lints]
  workspace = true
  ```

- [ ] **Step 2: Edit `crates/shared/surfaces/Cargo.toml`**

  Change `version.workspace = true` to `version = "0.0.1"` and add `publish = true`:

  ```toml
  [package]
  name = "uptrakit-surfaces"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"
  publish = true
  description = "Shared surface contracts for built-in and provider UI integration"

  [dependencies]
  serde = { workspace = true }
  serde_json = { workspace = true }
  thiserror = { workspace = true }
  uuid = { workspace = true, features = ["serde", "v4"] }

  [dev-dependencies]
  serde_json = { workspace = true }

  [lints]
  workspace = true
  ```

- [ ] **Step 3: Edit `crates/shared/types/Cargo.toml`**

  Change `version.workspace = true` to `version = "0.0.1"` and add `publish = true`:

  ```toml
  [package]
  name = "uptrakit-shared-types"
  description = "Uptrakit shared types: cross-crate enums, ids, and value objects"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"
  publish = true

  [features]
  default = []
  sea-orm = ["dep:sea-orm"]
  openapi = ["dep:utoipa"]
  http-ssrf = ["dep:reqwest", "dep:tokio", "dep:rustls", "dep:webpki-roots"]
  test-support = []

  [dependencies]
  serde = { workspace = true }
  serde_json = { workspace = true }
  thiserror = { workspace = true }
  strum = { workspace = true }
  sea-orm = { workspace = true, optional = true }
  utoipa = { workspace = true, optional = true }
  zeroize = { workspace = true }
  reqwest = { workspace = true, optional = true }
  rustls = { workspace = true, optional = true }
  tokio = { workspace = true, features = ["net"], optional = true }
  webpki-roots = { workspace = true, optional = true }
  url = { workspace = true }

  [dev-dependencies]
  husky-rs = { workspace = true }
  tokio = { workspace = true, features = ["macros", "rt"] }

  [lints]
  workspace = true
  ```

- [ ] **Step 4: Edit `crates/shared/wire/Cargo.toml`**

  Change `version.workspace = true` to `version = "0.0.1"` and add `publish = true`:

  ```toml
  [package]
  name = "uptrakit-wire"
  description = "Uptrakit shared wire protocol: WS, NATS, and REST message types"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"
  publish = true

  [dependencies]
  async-trait = { workspace = true }
  serde = { workspace = true }
  serde_json = { workspace = true }
  strum = { workspace = true }
  thiserror = { workspace = true }
  time = { workspace = true }
  tokio = { workspace = true, features = ["time"] }
  tracing = { workspace = true }
  uuid = { workspace = true, features = ["serde", "v4"] }
  uptrakit-shared-types = { workspace = true }
  uptrakit-surfaces = { workspace = true }

  [dev-dependencies]
  serde_yaml_ng = { workspace = true }
  tokio = { workspace = true, features = ["macros", "rt", "time", "test-util"] }

  [lints]
  workspace = true
  ```

- [ ] **Step 5: Edit `crates/shared/web-api-types/Cargo.toml`**

  Change `version.workspace = true` to `version = "0.0.1"` and add `publish = true`:

  ```toml
  [package]
  name = "uptrakit-web-api-types"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"
  publish = true
  description = "Shared HTTP request/response types for the Uptrakit web API"

  [features]
  default = []
  openapi = ["dep:utoipa", "uptrakit-shared-types/openapi"]

  [dependencies]
  serde = { workspace = true }
  serde_json = { workspace = true }
  thiserror = { workspace = true }
  tracing = { workspace = true }
  time = { workspace = true }
  uptrakit-wire = { workspace = true }
  uptrakit-shared-macros = { workspace = true }
  uptrakit-shared-types = { workspace = true }
  uuid = { workspace = true, features = ["serde"] }
  utoipa = { workspace = true, optional = true }
  zeroize = { workspace = true }

  [dev-dependencies]
  serde_json = { workspace = true }
  strum = { workspace = true }

  [lints]
  workspace = true
  ```

- [ ] **Step 6: Run quality gates**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  ```

  Expected: all pass. If clippy complains about empty `[dependencies]` section in macros Cargo.toml, remove the empty section header entirely.

- [ ] **Step 7: Validate leaf crate packaging**

  ```bash
  cargo package --dry-run -p uptrakit-shared-macros
  cargo package --dry-run -p uptrakit-surfaces
  cargo package --dry-run -p uptrakit-shared-types
  ```

  Expected: each prints `Packaging uptrakit-shared-X v0.0.1` with no errors. Do NOT run dry-run on wire,
  web-api-types, service-sdk, or openapi-client — those crates have internal deps not yet on crates.io and
  will fail.

- [ ] **Step 8: Commit**

  ```bash
  git add \
    crates/shared/macros/Cargo.toml \
    crates/shared/surfaces/Cargo.toml \
    crates/shared/types/Cargo.toml \
    crates/shared/wire/Cargo.toml \
    crates/shared/web-api-types/Cargo.toml
  git commit -m "feat(publish): set independent versions and publish=true for 5 shared crates"
  ```

---

## Task 2: Commit 2 — service-sdk: remove workspace-internal + generated/

Remove the `workspace-internal` feature, convert optional wire/shared-types deps to hard deps, replace passthrough
stub features with real forwarding features, delete the vendored `src/generated/` directory, and simplify
`src/lib.rs` to direct re-exports.

- [ ] **Step 1: Edit `crates/shared/service-sdk/Cargo.toml`**

  Replace the `[package]` section — add `publish = true` (version is already `"0.0.1"`).

  Replace the `[features]` block:

  ```toml
  [features]
  default = ["zeroconf"]
  zeroconf = ["dep:mdns-sd"]
  cli = []
  test-support = []
  sensitive-params = ["dep:aws-lc-rs"]
  sea-orm = ["uptrakit-shared-types/sea-orm"]
  openapi = ["uptrakit-shared-types/openapi"]
  ```

  In `[dependencies]`, make these changes:

  - Remove: `sea-orm = { workspace = true, optional = true }`
  - Remove: `utoipa = { workspace = true, optional = true }`
  - Change: `uptrakit-wire = { workspace = true, optional = true }` → `uptrakit-wire = { workspace = true }`
  - Change: `uptrakit-shared-types = { workspace = true, optional = true }` → `uptrakit-shared-types = { workspace = true }`
  - Keep: `aws-lc-rs = { workspace = true, optional = true }` (used by `sensitive-params` feature)
  - Keep: `mdns-sd = { workspace = true, optional = true }` (used by `zeroconf` feature)

  Also add `publish = true` to `[package]`. Full `[package]` section after:

  ```toml
  [package]
  name = "uptrakit-service-sdk"
  description = "Uptrakit service SDK — lifecycle, identity, TLS, wire protocol, and surface types for Uptrakit-managed services"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"
  publish = true
  ```

- [ ] **Step 2: Edit `Cargo.toml` (workspace root) — remove `workspace-internal` from service-sdk dep**

  Find the `uptrakit-service-sdk` entry in `[workspace.dependencies]`. It currently includes
  `features = ["workspace-internal"]`. Remove that `features` line entirely.

  Before:

  ```toml
  uptrakit-service-sdk = { path = "crates/shared/service-sdk", version = "0.0.1", default-features = false, features = ["workspace-internal"] }
  ```

  After:

  ```toml
  uptrakit-service-sdk = { path = "crates/shared/service-sdk", version = "0.0.1", default-features = false }
  ```

  Keep `default-features = false` — service-sdk's default features include `zeroconf` (mdns-sd), which internal
  workspace crates (agents, controller) do not need. Only remove the `features` key.

  Without removing `features = ["workspace-internal"]`, workspace crates that depend on `uptrakit-service-sdk`
  will request a feature that no longer exists, causing a build error immediately after this commit.

- [ ] **Step 3: Edit `crates/shared/service-sdk/src/lib.rs`**

  Remove `pub mod generated;` (currently line 16).

  Remove the long doc comment and the four alternating `#[cfg]` mod blocks (currently lines 32–54, starting
  from the `/// In workspace builds:` comment through the end of the second `shared_types_api` block).
  Replace them with two unconditional re-export modules:

  ```rust
  pub(crate) mod wire_api {
      pub(crate) use uptrakit_wire::*;
  }

  pub(crate) mod shared_types_api {
      pub(crate) use uptrakit_shared_types::*;
  }
  ```

  The top of `lib.rs` after editing should look like:

  ```rust
  #[macro_use]
  mod macros;

  pub mod backoff;
  pub mod build_info;
  pub mod ca;
  pub mod cert_handler;
  pub mod cli;
  pub mod config_proxy;
  pub mod connection;
  pub mod dirs;
  #[cfg(feature = "zeroconf")]
  pub mod discovery;
  pub mod error;
  pub mod event_loop;
  pub mod identity;
  pub mod lifecycle;
  pub mod main_helper;
  #[cfg(feature = "sensitive-params")]
  pub mod sensitive_params;
  pub mod shared_types;
  pub mod shutdown;
  pub mod signal;
  pub mod surface_proxy;
  #[cfg(any(test, feature = "test-support"))]
  pub mod test_support;
  pub mod tls;
  pub mod tracing_init;
  pub(crate) mod ws;

  pub(crate) mod wire_api {
      pub(crate) use uptrakit_wire::*;
  }

  pub(crate) mod shared_types_api {
      pub(crate) use uptrakit_shared_types::*;
  }

  #[cfg(feature = "cli")]
  pub use tracing_init::init_cli_tracing;
  // ... rest of pub use lines unchanged
  ```

- [ ] **Step 4: Delete `src/generated/`**

  ```bash
  rm -rf crates/shared/service-sdk/src/generated
  ```

- [ ] **Step 5: Run quality gates**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  ```

  Expected: all pass. If you see `error[E0432]: unresolved import uptrakit_wire` or similar, verify the dep
  changed from `optional = true` to non-optional in Cargo.toml.

- [ ] **Step 6: Commit**

  ```bash
  git add \
    Cargo.toml \
    crates/shared/service-sdk/Cargo.toml \
    crates/shared/service-sdk/src/lib.rs
  git rm -r crates/shared/service-sdk/src/generated
  git commit -m "feat(service-sdk): remove workspace-internal feature and vendored generated/"
  ```

---

## Task 3: Commit 3 — openapi-client: remove workspace-internal + generated/, restore shared-macros dep

Mirror what Task 2 did for `service-sdk`, plus restore the `uptrakit-shared-macros` dependency (replacing the
inlined `src/macros.rs`). The only non-generated call site for `impl_report_conversion!` inside `openapi-client`
is `src/error.rs` — update that file to import the macro explicitly.

- [ ] **Step 1: Edit `crates/shared/openapi-client/Cargo.toml`**

  Add `publish = true` to `[package]` (version is already `"0.0.1"`):

  ```toml
  [package]
  name = "uptrakit-openapi-client"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"
  publish = true
  description = "Typed HTTP client for the Uptrakit web API"
  ```

  Replace the `[features]` block:

  ```toml
  [features]
  ## Enables `MockApiServer` and `MockEndpoint` for use in tests of downstream crates.
  ## Activate with `features = ["mock"]` in `[dev-dependencies]`.
  mock = ["dep:httpmock"]
  ## Enables `#[tracing::instrument]` spans on async methods (tracing is always available).
  tracing = []
  ## Forwarding feature: enables sea-orm derives on shared-types values.
  sea-orm = ["uptrakit-shared-types/sea-orm"]
  ## Forwarding feature: enables utoipa OpenAPI derives on web-api-types values.
  openapi = ["uptrakit-web-api-types/openapi"]
  ```

  In `[dependencies]`, make these changes:

  - Remove: `uptrakit-web-api-types = { workspace = true, optional = true }`
  - Remove: `uptrakit-shared-types = { workspace = true, optional = true }`
  - Add: `uptrakit-web-api-types = { workspace = true }`
  - Add: `uptrakit-shared-types = { workspace = true }`
  - Add: `uptrakit-shared-macros = { workspace = true }`
  - Keep: `httpmock = { workspace = true, optional = true }` (used by `mock` feature)

  Full `[dependencies]` section after:

  ```toml
  [dependencies]
  reqwest = { workspace = true, features = ["query"] }
  futures-util = { workspace = true }
  serde = { workspace = true }
  serde_json = { workspace = true }
  rootcause = { workspace = true }
  thiserror = { workspace = true }
  tokio = { workspace = true, features = ["time"] }
  tracing = { workspace = true }
  uuid = { workspace = true, features = ["serde", "v4"] }
  httpmock = { workspace = true, optional = true }
  uptrakit-web-api-types = { workspace = true }
  uptrakit-shared-types = { workspace = true }
  uptrakit-shared-macros = { workspace = true }
  strum = { workspace = true }
  zeroize = { workspace = true }
  url = { workspace = true }
  time = { workspace = true, features = ["serde"] }
  async-trait = { workspace = true }
  ```

- [ ] **Step 2: Edit `Cargo.toml` (workspace root) — remove `workspace-internal` from openapi-client dep**

  Find the `uptrakit-openapi-client` entry in `[workspace.dependencies]`. It currently includes
  `features = ["workspace-internal"]`. Remove that `features` line entirely.

  Before:

  ```toml
  uptrakit-openapi-client = { path = "crates/shared/openapi-client", version = "0.0.1", features = ["workspace-internal"] }
  ```

  After:

  ```toml
  uptrakit-openapi-client = { path = "crates/shared/openapi-client", version = "0.0.1" }
  ```

  Without this change, workspace crates that depend on `uptrakit-openapi-client` will request a feature that
  no longer exists, causing a build error immediately after this commit.

- [ ] **Step 3: Edit `crates/shared/openapi-client/src/error.rs`**

  Add `use uptrakit_shared_macros::impl_report_conversion;` after the existing `use rootcause::prelude::*;`
  line. The macro is `#[macro_export]` in `uptrakit-shared-macros`, so this `use` import brings it into scope
  with its bare name.

  Full file after:

  ```rust
  use rootcause::prelude::*;
  use uptrakit_shared_macros::impl_report_conversion;

  /// Errors that can occur when communicating with the Uptrakit API.
  #[derive(Debug, thiserror::Error)]
  pub enum ClientError {
      #[error("HTTP request failed: {0}")]
      Http(#[from] reqwest::Error),

      #[error("JSON error: {0}")]
      Json(#[from] serde_json::Error),

      #[error("API error ({status}): {message}")]
      Api {
          status: reqwest::StatusCode,
          message: String,
      },

      #[error("rate limited{}", match .retry_after_seconds {
          Some(secs) => format!(" (retry after {secs}s)"),
          None => String::new(),
      })]
      RateLimited { retry_after_seconds: Option<u64> },

      #[error("not found: {0}")]
      NotFound(String),

      #[error("not authenticated")]
      NotAuthenticated,

      #[error("invalid HTTP method: {0}")]
      InvalidMethod(String),
  }

  pub type Result<T> = std::result::Result<T, Report<ClientError>>;

  impl_report_conversion! {
      reqwest::Error  => ClientError::Http,
      serde_json::Error => ClientError::Json,
  }
  ```

- [ ] **Step 4: Edit `crates/shared/openapi-client/src/lib.rs`**

  Remove line 1–2 (`#[macro_use] mod macros;`).

  Remove line 4 (`pub mod generated;`).

  Remove lines 53–87 (the six alternating `#[cfg]` blocks for `types`, `DeviceAuthStatus`, `types_impl`, `shared_types_impl`).

  Replace those six blocks with four unconditional declarations:

  ```rust
  pub use uptrakit_web_api_types as types;
  pub use uptrakit_shared_types::DeviceAuthStatus;

  pub(crate) mod types_impl {
      pub(crate) use uptrakit_web_api_types::*;
  }

  pub(crate) mod shared_types_impl {
      #[allow(unused_imports)]
      pub(crate) use uptrakit_shared_types::*;
  }
  ```

  The top of `lib.rs` after editing (through the `Uuid` / `ReqwestError` re-exports) should look like:

  ```rust
  #[cfg(feature = "mock")]
  pub mod mock;

  pub(crate) mod paths;

  pub mod access_presets;
  pub mod api_tokens;
  pub mod audit_logs;
  pub mod auth;
  pub mod autodiscovery;
  pub mod batch_progress_stream;
  pub mod device_auth_stream;
  pub mod discovery_allowlist;
  pub mod enrollment_tokens;
  pub mod error;
  pub mod events_stream;
  pub mod health;
  pub mod host_tags;
  pub mod hosts;
  pub mod notifications;
  pub mod oidc_auth;
  pub mod oidc_providers;
  pub mod permissions;
  pub mod pki;
  pub mod plugin_configs;
  pub mod plugin_type_settings;
  pub mod roles;
  pub mod scheduler;
  pub mod services;
  pub mod settings;
  pub mod settings_nats;
  pub mod settings_provider_github;
  pub mod software_items;
  pub mod sse;
  pub mod surfaces;
  pub mod system_alerts;
  pub mod system_enrollment_tokens;
  pub mod system_services;
  pub mod update_batches;
  pub mod update_history;
  pub mod update_output_stream;
  pub mod users;

  pub use error::{ClientError, Result};

  pub use uptrakit_web_api_types as types;
  pub use uptrakit_shared_types::DeviceAuthStatus;

  pub(crate) mod types_impl {
      pub(crate) use uptrakit_web_api_types::*;
  }

  pub(crate) mod shared_types_impl {
      #[allow(unused_imports)]
      pub(crate) use uptrakit_shared_types::*;
  }

  pub use uuid::Uuid;
  pub use reqwest::Error as ReqwestError;
  pub use reqwest::StatusCode;

  use rootcause::prelude::*;
  // ... rest of file unchanged
  ```

- [ ] **Step 5: Delete `src/macros.rs` and `src/generated/`**

  ```bash
  rm crates/shared/openapi-client/src/macros.rs
  rm -rf crates/shared/openapi-client/src/generated
  ```

- [ ] **Step 6: Run quality gates**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  ```

  Expected: all pass. If you see `error[E0433]: failed to resolve: use of undeclared crate or module 'macros'`,
  verify `#[macro_use] mod macros;` was fully removed from `lib.rs`. If you see
  `cannot find macro 'impl_report_conversion'`, verify
  `use uptrakit_shared_macros::impl_report_conversion;` is present in `error.rs`.

- [ ] **Step 7: Commit**

  ```bash
  git add \
    Cargo.toml \
    crates/shared/openapi-client/Cargo.toml \
    crates/shared/openapi-client/src/lib.rs \
    crates/shared/openapi-client/src/error.rs
  git rm crates/shared/openapi-client/src/macros.rs
  git rm -r crates/shared/openapi-client/src/generated
  git commit -m "feat(openapi-client): remove workspace-internal feature, vendored generated/, restore shared-macros dep"
  ```

---

## Task 4: Commit 4 — delete sync infrastructure

`xtask/` contains only the two sync commands. With those gone, the binary has no subcommands and the crate is
empty. Delete the entire `xtask/` directory and remove it from workspace members. Also strip the pre-commit
hook and delete the generated-check CI workflow.

- [ ] **Step 1: Delete the xtask crate**

  ```bash
  rm -rf xtask
  ```

- [ ] **Step 2: Edit `Cargo.toml` (workspace root) — remove xtask from members**

  Remove `"xtask",` from the `members` array (line 11). The `members` block after:

  ```toml
  members = [
      "crates/core/*",
      "crates/shared/*",
      "crates/ui/*",
      "crates/plugins/*/*",
      "crates/plugins/hooks/*",
      "crates/plugins/notifications/*",
      "frontend",
  ]
  ```

- [ ] **Step 3: Edit `.husky/pre-commit` — remove sync blocks**

  Delete everything from the line `echo '[pre-commit] Regenerating service-sdk generated types...'` to the end
  of the file. This is 6 lines total (lines 114–119): two `echo` lines, two `cargo xtask` lines, one blank
  line between them, and a trailing blank line at EOF.

  The file should end after the frontend prettier check block. Verify the last non-blank line of the file is
  now `fi` (the closing brace of the frontend check).

- [ ] **Step 4: Delete `.github/workflows/generated-check.yml`**

  ```bash
  rm .github/workflows/generated-check.yml
  ```

- [ ] **Step 5: Run quality gates**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  ```

  Expected: all pass. If you see `error: package 'xtask' not found in workspace`, verify `"xtask"` was removed from `Cargo.toml` members.

- [ ] **Step 6: Commit**

  ```bash
  git add Cargo.toml .husky/pre-commit
  git rm -r xtask
  git rm .github/workflows/generated-check.yml
  git commit -m "feat(xtask): delete sync-sdk and sync-openapi-client infrastructure"
  ```

---

## Task 5: Commit 5 — release-plz: enable publish for 7 crates

Add 5 new `[[package]]` entries for the shared crates (no `git_only`, which defaults to crates.io publish per
the workspace-level `publish = false` default). Remove `git_only = true` from the two SDK crate entries.

- [ ] **Step 1: Edit `release-plz.toml`**

  Add the following 5 entries immediately after the `[workspace]` block (before the existing `[[package]]` entries):

  ```toml
  [[package]]
  name = "uptrakit-shared-macros"

  [[package]]
  name = "uptrakit-surfaces"

  [[package]]
  name = "uptrakit-shared-types"

  [[package]]
  name = "uptrakit-wire"

  [[package]]
  name = "uptrakit-web-api-types"
  ```

  In the existing `uptrakit-openapi-client` entry, remove the `git_only = true` line:

  ```toml
  [[package]]
  name = "uptrakit-openapi-client"
  git_release_enable = true
  git_tag_enable = true
  git_tag_name = "uptrakit-openapi-client-v{{ version }}"
  git_release_name = "uptrakit-openapi-client v{{ version }}"
  ```

  In the existing `uptrakit-service-sdk` entry, remove the `git_only = true` line:

  ```toml
  [[package]]
  name = "uptrakit-service-sdk"
  git_release_enable = true
  git_tag_enable = true
  git_tag_name = "uptrakit-service-sdk-v{{ version }}"
  git_release_name = "uptrakit-service-sdk v{{ version }}"
  ```

- [ ] **Step 2: Run quality gates**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  ```

  Expected: all pass (no Rust changes in this commit; the gates confirm nothing regressed).

- [ ] **Step 3: Commit**

  ```bash
  git add release-plz.toml
  git commit -m "feat(release-plz): enable crates.io publish for 7 shared crates"
  ```

---

## Task 6: Open PR, merge, and bootstrap

- [ ] **Step 1: Push the branch and open a PR**

  ```bash
  git push -u origin HEAD
  gh pr create \
    --title "feat: migrate service-sdk and openapi-client to crates.io publishing" \
    --body "$(cat <<'EOF'
  ## Summary

  - Publish 5 shared type crates to crates.io (macros, surfaces, types, wire, web-api-types)
  - Remove workspace-internal feature and vendored generated/ from service-sdk and openapi-client
  - Delete xtask sync infrastructure (sync-sdk, sync-openapi-client)
  - Enable crates.io publish for service-sdk and openapi-client in release-plz

  ## Note

  The `release-plz release-pr` CI job will produce a red check on the merge commit. This is expected — it runs `cargo package` for crates whose deps do not yet exist on crates.io. Bootstrap immediately after merge to resolve it.

  ## Bootstrap (post-merge)

  Run the bootstrap script in the repo root after merging.
  EOF
  )"
  ```

- [ ] **Step 2: Merge the PR**

  Merge via GitHub UI or `gh pr merge --squash`. Document in the merge commit body that the release-pr CI
  failure on this commit is expected and will self-resolve after bootstrap.

- [ ] **Step 3: Bootstrap — publish all 7 crates to crates.io**

  Save the following script to a temporary file and run it from the workspace root. It is idempotent: if a
  crate is already on crates.io it is skipped. It polls the crates.io index after each publish before
  proceeding to the next crate.

  ```bash
  #!/usr/bin/env bash
  set -euo pipefail

  publish_crate() {
    local name="$1"
    if cargo search "$name" --limit 1 2>/dev/null | grep -qE "^$name[[:space:]]*="; then
      echo "$name already on crates.io — skipping"
      return
    fi
    cargo publish -p "$name"
    echo "Waiting for $name to appear in crates.io index..."
    until cargo search "$name" --limit 1 2>/dev/null | grep -qE "^$name[[:space:]]*="; do
      sleep 10
    done
    echo "$name indexed."
  }

  publish_crate uptrakit-shared-macros
  publish_crate uptrakit-surfaces
  publish_crate uptrakit-shared-types
  publish_crate uptrakit-wire
  publish_crate uptrakit-web-api-types
  publish_crate uptrakit-service-sdk
  publish_crate uptrakit-openapi-client
  ```

  If a publish fails mid-sequence (e.g. network error), re-run the script — already-published crates are skipped automatically.

- [ ] **Step 4: Post-bootstrap verification**

  Run `cargo package --dry-run` for all 7 crates. These now succeed because deps are live on crates.io:

  ```bash
  cargo package --dry-run -p uptrakit-shared-macros
  cargo package --dry-run -p uptrakit-surfaces
  cargo package --dry-run -p uptrakit-shared-types
  cargo package --dry-run -p uptrakit-wire
  cargo package --dry-run -p uptrakit-web-api-types
  cargo package --dry-run -p uptrakit-service-sdk
  cargo package --dry-run -p uptrakit-openapi-client
  ```

  Expected: all 7 print `Packaging uptrakit-X v0.0.1` with no errors.

- [ ] **Step 5: Verify release-plz now works**

  Push any trivial change to `main` (e.g. a whitespace-only doc fix) to trigger a `release-plz release-pr`
  CI run. Confirm the job succeeds and does NOT produce a new release PR (because nothing has changed since
  0.0.1). Subsequent version-bumping commits will trigger release PRs normally.

---

## Self-review checklist

**Spec coverage:**

| Spec requirement | Task covering it |
| --- | --- |
| 5 crates: `version = "0.0.1"` explicit + `publish = true` | Task 1 |
| `rootcause` moved to dev-deps in macros | Task 1 step 1 |
| service-sdk: remove `workspace-internal`, hard deps, forward features | Task 2 step 1 |
| workspace Cargo.toml: remove `features = ["workspace-internal"]` for service-sdk | Task 2 step 2 |
| service-sdk: replace alternating cfg blocks in `lib.rs` | Task 2 step 3 |
| service-sdk: delete `src/generated/` | Task 2 step 4 |
| openapi-client: remove `workspace-internal`, hard deps, forward features | Task 3 step 1 |
| openapi-client: add `uptrakit-shared-macros` dep | Task 3 step 1 |
| workspace Cargo.toml: remove `features = ["workspace-internal"]` for openapi-client | Task 3 step 2 |
| openapi-client: update `error.rs` call site for `impl_report_conversion!` | Task 3 step 3 |
| openapi-client: replace alternating cfg blocks in `lib.rs` | Task 3 step 4 |
| openapi-client: delete `src/macros.rs` and `src/generated/` | Task 3 step 5 |
| Delete xtask sync files + remove xtask from workspace | Task 4 step 1–2 |
| Remove sync blocks from `.husky/pre-commit` | Task 4 step 3 |
| Delete `.github/workflows/generated-check.yml` | Task 4 step 4 |
| Add 5 `[[package]]` entries to `release-plz.toml` | Task 5 step 1 |
| Remove `git_only = true` from openapi-client + service-sdk entries | Task 5 step 1 |
| Bootstrap script (idempotent, polling) | Task 6 step 3 |
| Post-bootstrap `cargo package --dry-run` for all 7 | Task 6 step 4 |
| `http-ssrf` and `test-support` stubs removed | Task 2 step 1 / Task 3 step 1 |
| `sea-orm` and `openapi` become real forwarding features | Task 2 step 1 / Task 3 step 1 |
