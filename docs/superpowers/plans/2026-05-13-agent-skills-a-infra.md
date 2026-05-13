# Agent Skills Plugin — Plan A: Cross-Cutting Infra

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thread the git tree SHA through the GitHub client types, add the `PACKAGE_MANAGER_SKILLS`
consumer constant, introduce `ReleaseFetchContext`, and wire the global GitHub provider lookup from
`AppState` down to `FetchReleasesExecutor` — with zero behaviour change for all existing plugins.

**Architecture:** Three additive type changes (sha field, consumer constant, context struct) + one
new factory-function type for `ReleaseFetcher` + a 5-layer wire from `AppState.global_providers` →
`EmbeddedSchedulerConfig` → `SchedulerRunConfig` → `FetchReleasesExecutor` → `ReleaseFetchContext`.
All existing plugin `new()` signatures and runtime paths are unchanged.

**Tech Stack:** Rust 2021 · `plugin-infrastructure-core` (descriptor.rs, roles.rs, macros.rs) ·
`scheduler-runtime` · `controller-runtime` · `github-client` · `global-github-provider` ·
`dashboard-icons` cache.

---

## File Map

| File                                                            | Action | What changes                                                                                                                                                                                    |
| --------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/shared/github-client/src/lib.rs`                        | Modify | Add `sha: String` to `RepositoryTreeEntry` and `RepositoryTreeEntryDto`; populate in `into_model`; update test JSON mocks                                                                       |
| `crates/shared/global-github-provider/src/lib.rs`               | Modify | Add `sha: String` to `GitHubTreeEntry`; add `PACKAGE_MANAGER_SKILLS` consumer constant                                                                                                          |
| `crates/ui/web-api/src/global_providers/github.rs`              | Modify | Thread `entry.sha` in `map_repository_tree_response`; add `sha` to test `GitHubTreeEntry` literals                                                                                              |
| `crates/plugins/enhancements/dashboard-icons/src/cache.rs`      | Modify | Add `sha` to test `GitHubTreeEntry` literals                                                                                                                                                    |
| `crates/plugins/infrastructure/core/src/roles.rs`               | Modify | Add `ReleaseFetchContext` struct                                                                                                                                                                |
| `crates/plugins/infrastructure/core/src/descriptor.rs`          | Modify | Add `CreateReleaseFetcherFn` type alias; add `ReleaseFetcherSlot` struct; change `RoleCreators.release_fetcher` to `Option<ReleaseFetcherSlot>`                                                 |
| `crates/plugins/infrastructure/core/src/macros.rs`              | Modify | Update `__define_role_creator!(ReleaseFetcher)` to 3-arg fn; update `__set_role_field!(ReleaseFetcher)` to use `ReleaseFetcherSlot`; add `release_fetcher_create` optional in `declare_plugin!` |
| `crates/core/scheduler-runtime/src/executors/fetch_releases.rs` | Modify | Add `provider_lookup: Option<Arc<dyn GlobalProviderLookup>>` field; pass `ReleaseFetchContext` to factory                                                                                       |
| `crates/core/scheduler-runtime/src/runtime.rs`                  | Modify | Add `global_provider_lookup` to `SchedulerRunConfig`; pass to `FetchReleasesExecutor`                                                                                                           |
| `crates/core/controller-runtime/src/scheduler/mod.rs`           | Modify | Add `global_provider_lookup` to `EmbeddedSchedulerConfig`; thread to `SchedulerRunConfig`                                                                                                       |
| `crates/core/controller-runtime/src/service_host/builtins.rs`   | Modify | Pass `app_state.global_providers()` as the lookup                                                                                                                                               |

---

### Task 1: Add `sha` to `RepositoryTreeEntry` in github-client

**Files:**

- Modify: `crates/shared/github-client/src/lib.rs`

- [ ] **Step 1: Find the two tests that mock tree JSON and update them to include `"sha"`**

  In `lib.rs` the test `fetch_repository_tree_decodes_blob_and_tree_entries` mocks:

  ```json
  "tree": [
    { "path": "svg/nginx.svg", "type": "blob" },
    { "path": "svg", "type": "tree" }
  ]
  ```

  Change to:

  ```json
  "tree": [
    { "path": "svg/nginx.svg", "type": "blob", "sha": "aabbcc1122334455aabbcc1122334455aabbcc11" },
    { "path": "svg", "type": "tree", "sha": "bbccdd2233445566bbccdd2233445566bbccdd22" }
  ]
  ```

  The test `fetch_repository_tree_wraps_invalid_success_payloads_as_do_not_retry_failures` mocks:

  ```json
  "tree": [
    { "path": "svg/nginx.svg", "type": "symlink" }
  ]
  ```

  Add `"sha": "aabbcc1122334455aabbcc1122334455aabbcc11"` here too (sha doesn't matter for the symlink-rejection test):

  ```json
  "tree": [
    { "path": "svg/nginx.svg", "type": "symlink", "sha": "aabbcc1122334455aabbcc1122334455aabbcc11" }
  ]
  ```

- [ ] **Step 2: Write a failing test asserting `sha` is populated**

  Add to the test module:

  ```rust
  #[tokio::test]
  async fn fetch_repository_tree_populates_sha_from_response() {
      use httpmock::Method::GET;
      use httpmock::MockServer;

      let server = MockServer::start();
      server.mock(|when, then| {
          when.method(GET)
              .path("/repos/owner/repo/git/trees/main")
              .query_param("recursive", "1");
          then.status(200).json_body(serde_json::json!({
              "truncated": false,
              "tree": [
                  {
                      "path": "skills/brainstorming",
                      "type": "tree",
                      "sha": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                  }
              ]
          }));
      });

      let client = GitHubClient::new(GitHubClientConfig::new(
          reqwest::Client::new(),
          url::Url::parse(&server.base_url()).unwrap(),
          GitHubAuth::Anonymous,
          "uptrakit-test",
      ));

      let outcome = client
          .fetch_repository_tree("owner", "repo", "main", true)
          .await
          .unwrap();
      let AttemptOutcome::Success(tree, _) = outcome else {
          panic!("expected success");
      };
      assert_eq!(tree.entries.len(), 1);
      assert_eq!(tree.entries[0].sha, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
  }
  ```

- [ ] **Step 3: Run the test to verify it fails**

  ```bash
  cargo test -p uptrakit-github-client fetch_repository_tree_populates_sha -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile error (field `sha` does not exist).

- [ ] **Step 4: Add `sha: String` to `RepositoryTreeEntryDto` and `RepositoryTreeEntry`**

  In `crates/shared/github-client/src/lib.rs`, change:

  ```rust
  #[derive(Debug, serde::Deserialize)]
  struct RepositoryTreeEntryDto {
      path: String,
      #[serde(rename = "type")]
      kind: String,
  }
  ```

  to:

  ```rust
  #[derive(Debug, serde::Deserialize)]
  struct RepositoryTreeEntryDto {
      path: String,
      #[serde(rename = "type")]
      kind: String,
      sha: String,
  }
  ```

  And change:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct RepositoryTreeEntry {
      pub path: String,
      pub kind: RepositoryTreeEntryKind,
  }
  ```

  to:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct RepositoryTreeEntry {
      pub path: String,
      pub kind: RepositoryTreeEntryKind,
      pub sha: String,
  }
  ```

- [ ] **Step 5: Thread `sha` through `into_model`**

  In `RepositoryTreeDto::into_model`, change the `Ok(RepositoryTreeEntry { ... })` line:

  ```rust
  Ok(RepositoryTreeEntry {
      path: entry.path,
      kind,
  })
  ```

  to:

  ```rust
  Ok(RepositoryTreeEntry {
      path: entry.path,
      kind,
      sha: entry.sha,
  })
  ```

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p uptrakit-github-client --all-features 2>&1 | tail -20
  ```

  Expected: all pass including the new `fetch_repository_tree_populates_sha` test.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/shared/github-client/src/lib.rs
  git commit -m "feat(github-client): add sha field to RepositoryTreeEntry"
  ```

---

### Task 2: Add `sha` to `GitHubTreeEntry` + consumer constant + bridge update

**Files:**

- Modify: `crates/shared/global-github-provider/src/lib.rs`
- Modify: `crates/ui/web-api/src/global_providers/github.rs`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/cache.rs`

- [ ] **Step 1: Add `sha: String` to `GitHubTreeEntry` and `PACKAGE_MANAGER_SKILLS` constant**

  In `crates/shared/global-github-provider/src/lib.rs`, change:

  ```rust
  pub const DASHBOARD_ICONS: GlobalProviderConsumerId =
      GlobalProviderConsumerId::new("dashboard-icons");
  ```

  to:

  ```rust
  pub const DASHBOARD_ICONS: GlobalProviderConsumerId =
      GlobalProviderConsumerId::new("dashboard-icons");

  /// Global GitHub provider consumer used by the package-manager-skills plugin.
  pub const PACKAGE_MANAGER_SKILLS: GlobalProviderConsumerId =
      GlobalProviderConsumerId::new("package-manager-skills");
  ```

  Change:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct GitHubTreeEntry {
      pub path: String,
      pub kind: GitHubTreeEntryKind,
  }
  ```

  to:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct GitHubTreeEntry {
      pub path: String,
      pub kind: GitHubTreeEntryKind,
      pub sha: String,
  }
  ```

- [ ] **Step 2: Add a test asserting the new constant value**

  In the `#[cfg(test)]` block in `lib.rs`, add:

  ```rust
  #[test]
  fn package_manager_skills_consumer_id_is_stable() {
      assert_eq!(PACKAGE_MANAGER_SKILLS.as_str(), "package-manager-skills");
  }
  ```

- [ ] **Step 3: Update `map_repository_tree_response` in `global_providers/github.rs`**

  In `crates/ui/web-api/src/global_providers/github.rs`, find `map_repository_tree_response` and change:

  ```rust
  Ok(GitHubTreeEntry {
      path: entry.path,
      kind,
  })
  ```

  to:

  ```rust
  Ok(GitHubTreeEntry {
      path: entry.path,
      kind,
      sha: entry.sha,
  })
  ```

- [ ] **Step 4: Fix `GitHubTreeEntry` literals in test code in `github.rs`**

  Search for `GitHubTreeEntry {` in `crates/ui/web-api/src/global_providers/github.rs`. For each literal, add
  `sha: "<placeholder>".into()` (or a realistic 40-char hex). For example, at the test helper around line 986:

  ```rust
  GitHubTreeEntry {
      path: "svg/nginx.svg".to_string(),
      kind: GitHubTreeEntryKind::Blob,
      sha: "aabbcc1122334455667788aabbcc1122334455aa".to_string(),
  }
  ```

  Do this for ALL `GitHubTreeEntry { ... }` struct literals in that file.

- [ ] **Step 5: Fix `GitHubTreeEntry` literals in `dashboard-icons/src/cache.rs`**

  Search for `GitHubTreeEntry {` in `crates/plugins/enhancements/dashboard-icons/src/cache.rs`.
  Add `sha: "aabbcc1122334455667788aabbcc1122334455aa".to_string()` to every literal.
  There are roughly 8–10 occurrences in the test helpers.

  For example, change:

  ```rust
  GitHubTreeEntry {
      path: "svg/actual-budget.svg".to_string(),
      kind: GitHubTreeEntryKind::Blob,
  },
  ```

  to:

  ```rust
  GitHubTreeEntry {
      path: "svg/actual-budget.svg".to_string(),
      kind: GitHubTreeEntryKind::Blob,
      sha: "aabbcc1122334455667788aabbcc1122334455aa".to_string(),
  },
  ```

- [ ] **Step 6: Compile-check affected crates**

  ```bash
  cargo check -p uptrakit-global-github-provider --all-features 2>&1 | tail -20
  cargo check -p uptrakit-plugin-enhancement-dashboard-icons --all-features 2>&1 | tail -20
  cargo check -p uptrakit-web-api --all-features 2>&1 | tail -20
  ```

  Expected: no errors.

- [ ] **Step 7: Run tests**

  ```bash
  cargo test -p uptrakit-global-github-provider --all-features 2>&1 | tail -20
  cargo test -p uptrakit-plugin-enhancement-dashboard-icons --all-features 2>&1 | tail -20
  ```

  Expected: all pass.

- [ ] **Step 8: Commit**

  ```bash
  git add crates/shared/global-github-provider/src/lib.rs \
          crates/ui/web-api/src/global_providers/github.rs \
          crates/plugins/enhancements/dashboard-icons/src/cache.rs
  git commit -m "feat(global-github-provider): add sha to GitHubTreeEntry and PACKAGE_MANAGER_SKILLS consumer"
  ```

---

### Task 3: Add `ReleaseFetchContext`, `CreateReleaseFetcherFn`, and `ReleaseFetcherSlot`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`

- [ ] **Step 1: Add `ReleaseFetchContext` to `roles.rs`**

  After the `use crate::batch_fetch::{...}` imports in `roles.rs`, add:

  ```rust
  #[cfg(feature = "catalog")]
  use crate::descriptor::GlobalProviderLookup;
  ```

  Then, before the `ReleaseFetcher` trait definition, add:

  ```rust
  /// Context provided to [`ReleaseFetcher`] factory functions at construction time.
  ///
  /// Passed as the third argument alongside the config JSON and `HostRuntime`
  /// when the scheduler creates a fetcher instance. Existing plugins ignore this
  /// context; the `package_manager_skills` plugin reads `global_provider_lookup`
  /// to reach the GitHub provider.
  #[non_exhaustive]
  pub struct ReleaseFetchContext {
      /// Global GitHub provider lookup, available when the embedded scheduler
      /// runs inside the controller. `None` in standalone-scheduler deployments.
      #[cfg(feature = "catalog")]
      pub global_provider_lookup: Option<std::sync::Arc<dyn GlobalProviderLookup>>,
  }

  impl ReleaseFetchContext {
      /// Construct a context with no provider lookup (standalone / test path).
      pub fn none() -> Self {
          Self {
              #[cfg(feature = "catalog")]
              global_provider_lookup: None,
          }
      }

      /// Construct from an `Option<Arc<dyn GlobalProviderLookup>>`.
      ///
      /// `None` → standalone / test path; `Some(lookup)` → controller path.
      #[cfg(feature = "catalog")]
      pub fn with_lookup_opt(
          lookup: Option<std::sync::Arc<dyn GlobalProviderLookup>>,
      ) -> Self {
          Self {
              global_provider_lookup: lookup,
          }
      }
  }
  ```

- [ ] **Step 2: Write a test for `ReleaseFetchContext`**

  Add to the `#[cfg(test)]` section at the bottom of `roles.rs`:

  ```rust
  #[test]
  fn release_fetch_context_none_has_no_lookup() {
      let ctx = ReleaseFetchContext::none();
      #[cfg(feature = "catalog")]
      assert!(ctx.global_provider_lookup.is_none());
      #[cfg(not(feature = "catalog"))]
      let _ = ctx;
  }
  ```

- [ ] **Step 3: Run the test**

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-core --all-features release_fetch_context -- --nocapture 2>&1 | tail -20
  ```

  Expected: PASS.

- [ ] **Step 4: Add `CreateReleaseFetcherFn` and `ReleaseFetcherSlot` to `descriptor.rs`**

  In `crates/plugins/infrastructure/core/src/descriptor.rs`, after the existing `CreateRoleFn<R>` type alias, add:

  ```rust
  /// Creation function for a `ReleaseFetcher` role — 3-arg variant that receives
  /// a `ReleaseFetchContext` so controller-side fetchers can reach the global
  /// GitHub provider. All other role factories keep the 2-arg `CreateRoleFn<R>`.
  pub type CreateReleaseFetcherFn =
      fn(
          &serde_json::Value,
          Arc<dyn HostRuntime>,
          &roles::ReleaseFetchContext,
      ) -> crate::error::Result<Box<dyn roles::ReleaseFetcher>>;

  /// A `ReleaseFetcher` creation function paired with its host requirements.
  ///
  /// Separate from `RoleSlot` because the factory takes 3 arguments (config,
  /// runtime, context) rather than the standard 2.
  pub struct ReleaseFetcherSlot {
      pub create: CreateReleaseFetcherFn,
      pub host_requirements: HostRequirements,
  }
  ```

- [ ] **Step 5: Change `RoleCreators.release_fetcher` to use `ReleaseFetcherSlot`**

  In `descriptor.rs`, find:

  ```rust
  pub release_fetcher: Option<RoleSlot<dyn roles::ReleaseFetcher>>,
  ```

  Change to:

  ```rust
  pub release_fetcher: Option<ReleaseFetcherSlot>,
  ```

  Update the `RoleCreators` `Default` impl or any struct literal that initialises `release_fetcher: None` —
  there should be just the macro-initialised one (no explicit `impl Default` needed since all fields are
  `Option<...>` that default to `None`).

- [ ] **Step 6: Update `test_support.rs` factory functions to 3-arg signature**

  In `crates/plugins/infrastructure/registry/src/test_support.rs`, two internal factory functions
  currently match the 2-arg `CreateRoleFn<dyn ReleaseFetcher>` signature. They must become 3-arg
  to satisfy `CreateReleaseFetcherFn`, and the `RoleSlot` construction must become `ReleaseFetcherSlot`.

  First, update the `use` statement. Find the line that imports `RoleSlot`
  (e.g. `use uptrakit_plugin_infrastructure_core::{..., RoleSlot, ...}`) and replace
  `RoleSlot` with `ReleaseFetcherSlot`. Keeping the unused `RoleSlot` import will trigger
  `unused_imports` in clippy.

  Find each function with this pattern:

  ```rust
  fn create_release_fetcher(
      _config: &serde_json::Value,
      _runtime: std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::HostRuntime>,
  ) -> uptrakit_plugin_infrastructure_core::error::Result<
      Box<dyn uptrakit_plugin_infrastructure_core::ReleaseFetcher>,
  > {
  ```

  Add the third parameter:

  ```rust
  fn create_release_fetcher(
      _config: &serde_json::Value,
      _runtime: std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::HostRuntime>,
      _ctx: &uptrakit_plugin_infrastructure_core::roles::ReleaseFetchContext,
  ) -> uptrakit_plugin_infrastructure_core::error::Result<
      Box<dyn uptrakit_plugin_infrastructure_core::ReleaseFetcher>,
  > {
  ```

  Do this for ALL functions matching that pattern in the file (there are two: `create_release_fetcher`
  and `create_per_item_fail_release_fetcher` or similar). Also change:

  ```rust
  release_fetcher: Some(RoleSlot {
      create: create_release_fetcher,
      host_requirements: HostRequirements::CONTROLLER_ONLY,
  }),
  ```

  to:

  ```rust
  release_fetcher: Some(ReleaseFetcherSlot {
      create: create_release_fetcher,
      host_requirements: HostRequirements::CONTROLLER_ONLY,
  }),
  ```

  Apply to all occurrences.

- [ ] **Step 7: Compile-check the core crate and registry**

  ```bash
  cargo check -p uptrakit-plugin-infrastructure-core --all-features 2>&1 | tail -20
  cargo check -p uptrakit-plugin-infrastructure-registry --all-features 2>&1 | tail -20
  ```

  Expected: `uptrakit-plugin-infrastructure-core` compiles cleanly (the type changes are in
  `descriptor.rs` and `roles.rs`, which have no dependency on `macros.rs`). `uptrakit-plugin-infrastructure-registry`
  produces errors in `macros.rs` where `RoleSlot<dyn roles::ReleaseFetcher>` is still used — those
  are fixed in Task 4. `test_support.rs` must compile cleanly after Step 6 is applied.

- [ ] **Step 8: Commit**

  ```bash
  git add crates/plugins/infrastructure/core/src/roles.rs \
          crates/plugins/infrastructure/core/src/descriptor.rs \
          crates/plugins/infrastructure/registry/src/test_support.rs
  git commit -m "feat(plugin-infra-core): add ReleaseFetchContext, CreateReleaseFetcherFn, ReleaseFetcherSlot"
  ```

---

### Task 4: Update `declare_plugin!` macro for 3-arg ReleaseFetcher factory

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/macros.rs`

This is the most intricate task. Work through it top-down: generated function, set-field helper, declare_plugin! top-level.

**Pre-flight:** enumerate all existing `.release_fetcher` field accesses so nothing is missed:

```bash
grep -rn "\.release_fetcher\b" crates/ | grep -v "target/"
```

Expected sites: `descriptor.rs` field definition, `macros.rs` macro arms, `test_support.rs` literals,
`fetch_releases.rs` usage, `plugin_ops.rs` host-requirements lookup (reads `.host_requirements` —
still compiles after type change because `ReleaseFetcherSlot` has the same field name). Review any
unexpected results before proceeding.

- [ ] **Step 1: Update `__define_role_creator!(ReleaseFetcher)` to the 3-arg signature**

  Find the arm:

  ```rust
  ($plugin:ty, $config:ty, ReleaseFetcher) => {
      pub(super) fn create_release_fetcher(
          config: &serde_json::Value,
          runtime: std::sync::Arc<dyn $crate::HostRuntime>,
      ) -> $crate::error::Result<Box<dyn $crate::ReleaseFetcher>> {
          let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
              rootcause::report!($crate::PluginError::Configuration(format!(
                  "failed to parse config: {e}"
              )))
          })?;
          let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
              rootcause::report!($crate::PluginError::Configuration(format!(
                  "plugin construction failed: {e}"
              )))
          })?;
          Ok(Box::new(plugin))
      }
  };
  ```

  Change to:

  ```rust
  ($plugin:ty, $config:ty, ReleaseFetcher) => {
      // NOTE: `_ctx` is passed but ignored in this auto-generated factory.
      // Plugins that need `ReleaseFetchContext` (e.g. to read a global provider)
      // must use the `release_fetcher_create:` override in `declare_plugin!` instead
      // of listing `ReleaseFetcher` in `roles: [...]` alone.
      pub(super) fn create_release_fetcher(
          config: &serde_json::Value,
          runtime: std::sync::Arc<dyn $crate::HostRuntime>,
          _ctx: &$crate::roles::ReleaseFetchContext,
      ) -> $crate::error::Result<Box<dyn $crate::ReleaseFetcher>> {
          let cfg: $config = serde_json::from_value(config.clone()).map_err(|e| {
              rootcause::report!($crate::PluginError::Configuration(format!(
                  "failed to parse config: {e}"
              )))
          })?;
          let plugin = <$plugin>::new(cfg, runtime).map_err(|e| {
              rootcause::report!($crate::PluginError::Configuration(format!(
                  "plugin construction failed: {e}"
              )))
          })?;
          Ok(Box::new(plugin))
      }
  };
  ```

- [ ] **Step 2: Update `__set_role_field!(ReleaseFetcher)` to use `ReleaseFetcherSlot`**

  Find:

  ```rust
  ($rc:ident, ReleaseFetcher, $hr:expr) => {
      $rc.release_fetcher = Some($crate::RoleSlot {
          create: __descriptor_impl::create_release_fetcher,
          host_requirements: $hr,
      });
  };
  ```

  Change to:

  ```rust
  ($rc:ident, ReleaseFetcher, $hr:expr) => {
      $rc.release_fetcher = Some($crate::ReleaseFetcherSlot {
          create: __descriptor_impl::create_release_fetcher,
          host_requirements: $hr,
      });
  };
  ```

- [ ] **Step 3: Add `release_fetcher_create` optional override to `declare_plugin!`**

  In the `declare_plugin!` pattern, after the existing `$(, infra: { ... })?` block, add a new optional:

  ```rust
  $(, release_fetcher_create: {
      create: $rf_create_fn:expr,
      host_requirements: $rf_hr:expr $(,)?
  } )?
  ```

  In the macro body, after the role loop and the other singleton setters (`notification_transport`, `software_item_lifecycle`, etc.), add:

  ```rust
  $(
      rc.release_fetcher = Some($crate::ReleaseFetcherSlot {
          create: $rf_create_fn,
          host_requirements: $rf_hr,
      });
  )?
  ```

  This overrides whatever the role loop set (if `ReleaseFetcher` was also in `roles: [...]`).
  For the Skills plugin, `ReleaseFetcher` is NOT listed in `roles: [...]` and instead only
  `release_fetcher_create` is used (see Plan B).

- [ ] **Step 4: Compile all crates that use `declare_plugin!`**

  ```bash
  cargo check --all-features 2>&1 | grep "^error" | head -30
  ```

  If any plugin crate fails because `create_release_fetcher` signature changed (i.e. some plugin provides
  a manual override), fix it. All auto-generated ones are handled by the macro change — only manually
  specified factories need updating (there should be none for existing plugins).

- [ ] **Step 5: Run the full test suite**

  ```bash
  cargo test --all-features 2>&1 | grep -E "^(test|FAILED|error)" | tail -40
  ```

  Expected: all pass.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/plugins/infrastructure/core/src/macros.rs
  git commit -m "feat(plugin-infra-core): update declare_plugin! for 3-arg ReleaseFetcher factory"
  ```

---

### Task 5: Wire `global_provider_lookup` through scheduler layers

**Files:**

- Modify: `crates/core/scheduler-runtime/src/executors/fetch_releases.rs`
- Modify: `crates/core/scheduler-runtime/src/runtime.rs`
- Modify: `crates/core/controller-runtime/src/scheduler/mod.rs`
- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`

- [ ] **Step 1: Add `provider_lookup` to `FetchReleasesExecutor` and wire to factory call**

  In `fetch_releases.rs`, add the import at the top:

  ```rust
  use uptrakit_plugin_infrastructure_core::{GlobalProviderLookup, roles::ReleaseFetchContext};
  ```

  (Both types are in `plugin-infrastructure-core`; the registry re-exports them but core is the
  canonical import path for `fetch_releases.rs` which depends on core directly.)

  Change the struct:

  ```rust
  pub struct FetchReleasesExecutor {
      db: DatabaseConnection,
      notifier: Arc<dyn SchedulerNotifier>,
  }
  ```

  to:

  ```rust
  pub struct FetchReleasesExecutor {
      db: DatabaseConnection,
      notifier: Arc<dyn SchedulerNotifier>,
      provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
  }
  ```

  Change `new`:

  ```rust
  impl FetchReleasesExecutor {
      pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
          Self { db, notifier }
      }
  }
  ```

  to:

  ```rust
  impl FetchReleasesExecutor {
      pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
          Self { db, notifier, provider_lookup: None }
      }

      pub fn with_global_provider_lookup(
          mut self,
          lookup: Option<Arc<dyn GlobalProviderLookup>>,
      ) -> Self {
          self.provider_lookup = lookup;
          self
      }
  }
  ```

  In `run_controller_side_fetch_releases`, find the factory call (around line 288):

  ```rust
  let runtime =
      construct_host_runtime(noop_executor.clone(), HostCapabilities::default());
  let fetcher = (slot.create)(&group.merged_config, runtime).map_err(|e| {
  ```

  Change to:

  ```rust
  let runtime =
      construct_host_runtime(noop_executor.clone(), HostCapabilities::default());
  // scheduler-runtime always depends on registry with `catalog` feature enabled,
  // so `with_lookup_opt` is always available here.
  let ctx = ReleaseFetchContext::with_lookup_opt(self.provider_lookup.clone());
  let fetcher = (slot.create)(&group.merged_config, runtime, &ctx).map_err(|e| {
  ```

- [ ] **Step 2: Add `global_provider_lookup` to `SchedulerRunConfig` in `runtime.rs`**

  In `crates/core/scheduler-runtime/src/runtime.rs`:

  Add import:

  ```rust
  use uptrakit_plugin_infrastructure_registry::GlobalProviderLookup;
  ```

  Change `SchedulerRunConfig`:

  ```rust
  pub struct SchedulerRunConfig {
      pub db: DatabaseConnection,
      pub controller_id: Uuid,
      pub notifier: Arc<dyn SchedulerNotifier>,
      pub should_yield: Box<dyn Fn() -> bool + Send + Sync>,
      pub poll_interval: Duration,
  }
  ```

  to:

  ```rust
  pub struct SchedulerRunConfig {
      pub db: DatabaseConnection,
      pub controller_id: Uuid,
      pub notifier: Arc<dyn SchedulerNotifier>,
      pub should_yield: Box<dyn Fn() -> bool + Send + Sync>,
      pub poll_interval: Duration,
      pub global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
  }
  ```

  Update `SchedulerRunConfig::new` to initialise the field to `None`:

  ```rust
  pub fn new(
      db: DatabaseConnection,
      controller_id: Uuid,
      notifier: Arc<dyn SchedulerNotifier>,
      should_yield: Box<dyn Fn() -> bool + Send + Sync>,
  ) -> Self {
      Self {
          db,
          controller_id,
          notifier,
          should_yield,
          poll_interval: DEFAULT_POLL_INTERVAL,
          global_provider_lookup: None,
      }
  }
  ```

  Add a builder method after `with_poll_interval`:

  ```rust
  pub fn with_global_provider_lookup(
      mut self,
      lookup: Option<Arc<dyn GlobalProviderLookup>>,
  ) -> Self {
      self.global_provider_lookup = lookup;
      self
  }
  ```

  In `build_scheduler`, destructure the new field and pass it to `FetchReleasesExecutor`:

  ```rust
  fn build_scheduler<F>(config: SchedulerRunConfig, register_extras: F) -> Scheduler
  where
      F: FnOnce(&mut Scheduler),
  {
      let SchedulerRunConfig {
          db,
          controller_id,
          notifier,
          should_yield,
          poll_interval,
          global_provider_lookup,
      } = config;

      // ... existing scheduler setup ...

      scheduler.register(
          ScheduledTaskType::FetchReleases,
          Box::new(
              FetchReleasesExecutor::new(db.clone(), Arc::clone(&notifier))
                  .with_global_provider_lookup(global_provider_lookup),
          ),
      );
      // rest unchanged
  ```

- [ ] **Step 3: Add `global_provider_lookup` to `EmbeddedSchedulerConfig`**

  In `crates/core/controller-runtime/src/scheduler/mod.rs`:

  Add import at the top:

  ```rust
  use uptrakit_plugin_infrastructure_registry::GlobalProviderLookup;
  ```

  Add the field to `EmbeddedSchedulerConfig`. The struct is `pub(crate)` and not feature-gated at
  field level (confirmed in `scheduler/mod.rs`). Find the struct definition and add the new field
  after `revocation_notify`:

  ```rust
  pub revocation_notify: Arc<Notify>,
  pub global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
  pub controller_update_hook:
      Option<std::sync::Arc<dyn uptrakit_plugin_infrastructure_registry::ControllerUpdateHook>>,
  ```

  (Both the existing `controller_update_hook` field and the new `global_provider_lookup` field are
  plain `Option` fields with no `#[cfg]` gate — the struct in the codebase does not use conditional
  compilation on individual fields.)

  In `run_embedded_scheduler`, thread the field into `SchedulerRunConfig`:

  ```rust
  uptrakit_scheduler_runtime::SchedulerRunConfig::new(
      config.db,
      config.controller_id,
      notifier,
      config.should_yield,
  )
  .with_global_provider_lookup(config.global_provider_lookup)
  ```

  Find the `uptrakit_scheduler_runtime::run_scheduler(...)` call and update the config passed to it
  (the `SchedulerRunConfig::new(...)` call — add `.with_global_provider_lookup(config.global_provider_lookup)`
  after constructing it).

- [ ] **Step 4: Pass the lookup from `AppState` in `builtins.rs`**

  In `crates/core/controller-runtime/src/service_host/builtins.rs`, find the `EmbeddedSchedulerConfig { ... }`
  literal (around line 225). Add the new field:

  ```rust
  crate::scheduler::EmbeddedSchedulerConfig {
      db,
      notification_service,
      controller_id,
      should_yield: yield_check,
      ca_managed,
      ca_snapshot: ca_tx_sub,
      ca_rotation_trigger,
      revocation_notify,
      global_provider_lookup: Some(app_state.global_providers()),
      #[cfg(feature = "plugin-ops")]
      controller_update_hook,
  },
  ```

  `app_state.global_providers()` returns `Arc<GlobalProviders>` which implements `GlobalProviderLookup`.
  Rust requires an explicit unsized coercion here — write the cast explicitly:

  ```rust
  global_provider_lookup: Some(
      app_state.global_providers() as Arc<dyn GlobalProviderLookup>
  ),
  ```

  Also add the import at the top of `builtins.rs` (the trait must be in scope for the `as` cast):

  ```rust
  use uptrakit_plugin_infrastructure_core::descriptor::GlobalProviderLookup;
  ```

- [ ] **Step 5: Confirm standalone scheduler gets `None` by default**

  ```bash
  grep -n "SchedulerRunConfig\|with_global_provider_lookup" \
    crates/core/scheduler-runtime/src/standalone.rs
  ```

  Expected: `SchedulerRunConfig::new(...)` is called without `.with_global_provider_lookup(...)`.
  Since `SchedulerRunConfig::new` initialises `global_provider_lookup: None` by default, no
  change is needed. If the standalone path uses a struct literal instead of `::new()`, add
  `global_provider_lookup: None` to the literal.

- [ ] **Step 6: Full compile check**

  ```bash
  cargo check --all-features 2>&1 | grep "^error" | head -30
  ```

  Expected: no errors.

- [ ] **Step 7: Run tests**

  ```bash
  cargo test --all-features 2>&1 | grep -E "^(FAILED|error\[)" | head -20
  ```

  Expected: no failures.

- [ ] **Step 8: Commit**

  ```bash
  git add \
    crates/plugins/infrastructure/core/src/roles.rs \
    crates/core/scheduler-runtime/src/executors/fetch_releases.rs \
    crates/core/scheduler-runtime/src/runtime.rs \
    crates/core/controller-runtime/src/scheduler/mod.rs \
    crates/core/controller-runtime/src/service_host/builtins.rs
  git commit -m "feat(scheduler): wire global_provider_lookup to FetchReleasesExecutor via ReleaseFetchContext"
  ```

---

### Task 6: Quality gate

- [ ] **Step 1: fmt + clippy**

  ```bash
  cargo fmt --all
  cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -30
  ```

  Expected: zero errors.

- [ ] **Step 2: Full test run**

  ```bash
  cargo test --all-features 2>&1 | tail -10
  ```

  Expected: all tests pass.

- [ ] **Step 3: markdownlint**

  No markdown files changed in this plan — skip.

- [ ] **Step 4: Final commit if any fmt changes**

  ```bash
  git add -p  # stage only fmt diffs
  git commit -m "style: cargo fmt after Plan A infra changes"
  ```

---

## Self-Review Checklist

- [ ] `sha` field threaded from DTO → `RepositoryTreeEntry` → `GitHubTreeEntry`
- [ ] `PACKAGE_MANAGER_SKILLS` constant has the correct string value `"package-manager-skills"`
- [ ] `ReleaseFetchContext` is `#[non_exhaustive]`
- [ ] `CreateReleaseFetcherFn` takes 3 args (config, runtime, ctx)
- [ ] `RoleCreators.release_fetcher` is `Option<ReleaseFetcherSlot>` not `Option<RoleSlot<...>>`
- [ ] `__define_role_creator!(ReleaseFetcher)` generates 3-arg fn
- [ ] `__set_role_field!(ReleaseFetcher)` uses `ReleaseFetcherSlot`
- [ ] `release_fetcher_create` override works in const context (direct assignment, no closures)
- [ ] `FetchReleasesExecutor::new` still takes 2 args; lookup is set via builder
- [ ] `SchedulerRunConfig::new` still takes same 4 args; lookup is set via builder
- [ ] `EmbeddedSchedulerConfig.global_provider_lookup` is populated from `app_state.global_providers()`
- [ ] Standalone scheduler passes `None` (no code change needed, default)
- [ ] All existing tests still pass
