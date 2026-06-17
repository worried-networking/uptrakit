# Skills Version Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the raw 40-character git tree SHA shown for LLM Skills in the Software Items list with the corresponding git commit date
(ISO 8601 UTC, rendered by the frontend as `"DD MMM YYYY at HH:MM"`), matching the Docker row presentation, on both `installed_version` and
`latest_version` columns.

**Architecture:** Add a controller-only typed plugin role `InstalledVersionEnricher` (capability `EnrichInstalledVersion`) that the web-api
dispatches generically via descriptor slot lookup — no plugin-type strings (ADR-0018 compliance). A new
`GitHubProviderClient::list_recent_commit_dates_for_path` primitive (oldest→newest walk, cross-batch root-tree-SHA cache, non-recursive tree
fetches, short-circuit on `expected_shas`) feeds both Skills's existing `ReleaseFetcher::batch_fetch` (top entry → `display_version`) and
the new `InstalledVersionEnricher` impl (lookup-by-tree-SHA → `installed_display_version`). Errors / misses collapse to `None` and frontend
short-SHA fallback renders. New ADR-0021 documents the role.

**Tech Stack:** Rust 2024 workspace (sea-orm, async-trait, rootcause, tokio, time, parking_lot), Svelte 5 + TypeScript frontend (vitest). No
new external crates.

**Spec:** `docs/superpowers/specs/2026-06-17-skills-version-display-design.md`

**Standards Snapshot:** `.superpowers/standards-snapshot.md` — binding rules invoked throughout this plan: `#[non_exhaustive]` on extensible
structs/enums; `rootcause::Report` for errors; no `unwrap`/`expect`/`panic!` in production; async-trait via `#[async_trait]`; tenant
isolation via `TenantDb::find_via_tenant_join`; date formatting via `time::macros::format_description!` (NOT `Rfc3339`); typed dispatch (no
plugin-type strings); atomic single-UPDATE write semantics for paired columns.

---

## File Structure

### Provider primitive (Phase 1)

- Modify `crates/shared/global-github-provider/src/lib.rs` — add `TreeCommit`, `list_recent_commit_dates_for_path` trait method with default
  impl returning `Misconfigured`.
- Modify `crates/ui/web-api/src/global_providers/github.rs` — implement the method on `GitHubProviderRuntime` (line 445).

### Role infrastructure (Phase 2)

- Modify `crates/shared/types/src/plugin_capability.rs` — add `EnrichInstalledVersion` variant + serde + tests.
- Modify `crates/plugins/infrastructure/core/src/roles.rs` — `InstalledVersionEnricher` trait, `InstalledVersionEnrichmentContext`.
- Modify `crates/plugins/infrastructure/core/src/descriptor.rs` — `InstalledVersionEnricherSlot` (bespoke, mirror
  `ReleaseFetcherSlot:260-273`) + field on `RoleCreators:499`.
- Modify `crates/plugins/infrastructure/core/src/macros.rs` — `installed_version_enricher_create:` factory field parallel to
  `release_fetcher_create:` (line 52), role-arm in `__accumulate_role_caps!`, default `None` in `RoleCreators` constructor (line 227-ish).
- Modify `crates/plugins/infrastructure/core/src/lib.rs` — re-exports.

### Skills plugin impl (Phases 3–4)

- Modify `crates/plugins/package-managers/skills/src/releases.rs` — `DISPLAY_FMT` constant, `batch_fetch` switches to the new provider
  primitive and emits `display_version`, new `InstalledVersionEnricher` impl block, expand `FakeProvider`/`CountingProvider` test doubles.
- Modify `crates/plugins/package-managers/skills/src/plugin.rs` — factory `create_installed_version_enricher_skills`, declare role +
  capability + factory key.

### Web-api dispatch (Phase 5)

- Create `crates/ui/web-api-queries/src/queries/host_software_item_plugins/mod.rs` — `plugin_types_for_role` tenant-scoped query.
- Modify `crates/ui/web-api-queries/src/queries/mod.rs` — module declaration.
- Modify `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` — `apply_version_update_to_db` signature gains override;
  `handle_version_check_results` performs typed-slot dispatch.

### Skills wire-up (Phase 6)

- Modify `crates/plugins/package-managers/skills/src/plugin.rs` — descriptor capability test extension; module doc-comment.

### Frontend (Phase 7)

- Modify `frontend/src/lib/utils.ts` — 40-hex SHA short-SHA branch in `formatVersion`.
- Modify `frontend/src/lib/utils.test.ts` (already exists) — extend with the new vitest case.

### Docs (Phase 8)

- Create `docs/adr/0021-installed-version-enrichment-role.md`.
- Modify `docs/development/coding-standards.md` — short subsection on the role.

---

## Phase 1: Provider primitive

### Task 1: `TreeCommit` struct + trait method signature

**Files:**

- Modify: `crates/shared/global-github-provider/src/lib.rs`

**Snapshot rules invoked:** `#[non_exhaustive]` on extensible public structs (coding-standards.md:383-434). Trait method gets a default impl
so the eight existing test doubles and production `GitHubProviderRuntime` compile unchanged.

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests { ... }` block in `crates/shared/global-github-provider/src/lib.rs` (or, if none exists, add at
the bottom of the file):

```rust
#[cfg(test)]
mod tree_commit_tests {
    use super::*;
    use std::collections::HashSet;

    struct UnimplementedProvider;
    #[async_trait::async_trait]
    impl GitHubProviderClient for UnimplementedProvider {
        async fn fetch_repository_tree(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _git_ref: &str,
            _recursive: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn list_recent_commit_dates_for_path_default_returns_misconfigured() {
        let p = UnimplementedProvider;
        let expected: HashSet<String> = HashSet::new();
        let err = p
            .list_recent_commit_dates_for_path(
                PACKAGE_MANAGER_SKILLS,
                "owner",
                "repo",
                "skills/x",
                90,
                &expected,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubProviderError::Misconfigured(_)),
            "expected Misconfigured, got: {err:?}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p uptrakit-global-github-provider list_recent_commit_dates_for_path_default_returns_misconfigured` Expected: FAIL with "no
method named `list_recent_commit_dates_for_path` found".

- [ ] **Step 3: Add `TreeCommit` struct + default trait method**

In `crates/shared/global-github-provider/src/lib.rs`, near the existing `GitHubRepositoryTree` definition:

```rust
/// A commit returned by [`GitHubProviderClient::list_recent_commit_dates_for_path`].
///
/// `tree_sha_at_path` is the SHA of the **subtree at the queried `path`** as of this
/// commit — not the commit's root tree SHA. `committed_at` is the committer date
/// (`commit.committer.date`), chosen over author date for rebase-stability.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeCommit {
    pub tree_sha_at_path: String,
    pub committed_at: time::OffsetDateTime,
}
```

In the `pub trait GitHubProviderClient` block (around line 104), add after the existing `fetch_repository_tree` method:

```rust
    /// Return up to `min(limit, 90)` recent commits that touched `path`, oldest-first,
    /// each annotated with the subtree SHA at `path` as of that commit.
    ///
    /// `expected_shas` lets the caller short-circuit the walk when every target SHA
    /// has been bound — pass `&HashSet::new()` to force the full walk.
    ///
    /// Default impl returns `Misconfigured("...not implemented")` so existing
    /// implementors compile unchanged. The Skills enricher treats this error like
    /// any other provider failure: log + write `None`.
    async fn list_recent_commit_dates_for_path(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        limit: usize,
        expected_shas: &std::collections::HashSet<String>,
    ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
        let _ = (consumer, owner, repo, path, limit, expected_shas);
        Err(GitHubProviderError::Misconfigured(
            "list_recent_commit_dates_for_path not implemented".to_string(),
        ))
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p uptrakit-global-github-provider list_recent_commit_dates_for_path_default_returns_misconfigured` Expected: PASS.

- [ ] **Step 5: Verify the 8 existing impls still compile**

Run:

```bash
cargo check --all-features \
  -p uptrakit-global-github-provider \
  -p uptrakit-plugin-package-manager-skills \
  -p uptrakit-plugin-enhancement-dashboard-icons \
  -p uptrakit-web-api
```

Expected: clean (the default impl protects every existing implementor).

- [ ] **Step 6: Commit**

```bash
git add crates/shared/global-github-provider/src/lib.rs
git commit -m "feat(global-github-provider): add TreeCommit + default list_recent_commit_dates_for_path"
```

---

### Task 2: Real `list_recent_commit_dates_for_path` impl on `GitHubProviderRuntime`

**Files:**

- Modify: `crates/ui/web-api/src/global_providers/github.rs:445` (the `impl GitHubProviderClient for GitHubProviderRuntime` block).
- Modify: `crates/ui/web-api/src/global_providers/github.rs` (tests).

**Snapshot rules invoked:** No `unwrap` / `expect` / `panic!` (coding-standards.md:23-38); HTTP client timeout/SSRF (AGENTS.md:283 — already
satisfied by existing `octocrab`/`reqwest::Client` config in the runtime, not in scope here since host is fixed `api.github.com`).

- [ ] **Step 1: Write the failing test (HTTP-mocked walk)**

`github.rs` has no existing `httpmock`-based test harness for `fetch_repository_tree` — only test-double impls of the trait. Adopt the
fresh-`httpmock::MockServer` pattern already used in `crates/ui/web-api/src/oauth/cimd.rs:587-1083` (search there for
`MockServer::start_async`). `httpmock = "0.8"` is already a workspace dev-dependency (`Cargo.toml:205`); ensure the test module imports
`use httpmock::prelude::*;`. Create or extend the test module at the bottom of `github.rs` with:

```rust
#[tokio::test]
async fn list_recent_commit_dates_for_path_walks_commits_then_trees() {
    use std::collections::HashSet;
    let server = httpmock::MockServer::start_async().await;

    // 1 page of 2 commits touching skills/foo.
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/repos/o/r/commits")
            .query_param("path", "skills/foo")
            .query_param("per_page", "30")
            .query_param("page", "1");
        then.status(200).json_body(serde_json::json!([
            {
                "sha": "c2",
                "commit": {
                    "committer": {"date": "2026-06-11T01:15:00Z"},
                    "tree": {"sha": "root_c2"}
                }
            },
            {
                "sha": "c1",
                "commit": {
                    "committer": {"date": "2026-05-01T08:00:00Z"},
                    "tree": {"sha": "root_c1"}
                }
            }
        ]));
    }).await;
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/repos/o/r/commits")
            .query_param("page", "2");
        then.status(200).json_body(serde_json::json!([]));
    }).await;

    // Non-recursive tree calls (per commit) — each returns one level.
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/git/trees/root_c1");
        then.status(200).json_body(serde_json::json!({
            "tree": [{"path": "skills", "type": "tree", "sha": "skills_c1"}]
        }));
    }).await;
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/git/trees/skills_c1");
        then.status(200).json_body(serde_json::json!({
            "tree": [{"path": "foo", "type": "tree", "sha": "foo_c1"}]
        }));
    }).await;
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/git/trees/root_c2");
        then.status(200).json_body(serde_json::json!({
            "tree": [{"path": "skills", "type": "tree", "sha": "skills_c2"}]
        }));
    }).await;
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/git/trees/skills_c2");
        then.status(200).json_body(serde_json::json!({
            "tree": [{"path": "foo", "type": "tree", "sha": "foo_c2"}]
        }));
    }).await;

    let runtime = make_runtime_for_test(&server).await;
    let out = runtime
        .list_recent_commit_dates_for_path(
            uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS,
            "o",
            "r",
            "skills/foo",
            90,
            &HashSet::new(),
        )
        .await
        .expect("ok");

    // Oldest-first ordering.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].tree_sha_at_path, "foo_c1");
    assert_eq!(out[1].tree_sha_at_path, "foo_c2");
}

#[tokio::test]
async fn list_recent_commit_dates_for_path_short_circuits_on_expected_shas() {
    use std::collections::HashSet;
    let server = httpmock::MockServer::start_async().await;

    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/repos/o/r/commits")
            .query_param("page", "1");
        then.status(200).json_body(serde_json::json!([
            {"sha":"c2","commit":{"committer":{"date":"2026-06-11T01:15:00Z"},"tree":{"sha":"root_c2"}}},
            {"sha":"c1","commit":{"committer":{"date":"2026-05-01T08:00:00Z"},"tree":{"sha":"root_c1"}}}
        ]));
    }).await;
    // ONLY define tree mocks for c1 (oldest). If the walker hits c2's trees the test fails.
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/git/trees/root_c1");
        then.status(200).json_body(serde_json::json!({
            "tree": [{"path":"skills","type":"tree","sha":"skills_c1"}]
        }));
    }).await;
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/git/trees/skills_c1");
        then.status(200).json_body(serde_json::json!({
            "tree": [{"path":"foo","type":"tree","sha":"foo_c1"}]
        }));
    }).await;

    let runtime = make_runtime_for_test(&server).await;
    let mut expected: HashSet<String> = HashSet::new();
    expected.insert("foo_c1".to_string());

    let out = runtime
        .list_recent_commit_dates_for_path(
            uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS,
            "o", "r", "skills/foo", 90, &expected,
        )
        .await
        .expect("ok");

    assert_eq!(out.len(), 1, "must short-circuit after binding all expected");
    assert_eq!(out[0].tree_sha_at_path, "foo_c1");
}

#[tokio::test]
async fn list_recent_commit_dates_for_path_404_surfaces_with_marker() {
    use std::collections::HashSet;
    use uptrakit_global_github_provider::GitHubProviderError;
    let server = httpmock::MockServer::start_async().await;
    server.mock_async(|when, then| {
        when.method(httpmock::Method::GET).path("/repos/o/r/commits");
        then.status(404);
    }).await;
    let runtime = make_runtime_for_test(&server).await;
    let err = runtime
        .list_recent_commit_dates_for_path(
            uptrakit_global_github_provider::PACKAGE_MANAGER_SKILLS,
            "o", "r", "skills/foo", 90, &HashSet::new(),
        )
        .await
        .unwrap_err();
    // The existing `fetch_repository_tree` ladder maps `RuntimeRequestError::NotFound`
    // to `GitHubProviderError::RequestFailed(message)` — mirror that mapping verbatim
    // here so dispatcher tagging stays uniform. Assert on the marker substring; let
    // the variant follow precedent (whichever it is — see Task 11 dispatcher comment
    // for how `upstream_gone` is derived from the message).
    let msg = err.to_string();
    assert!(msg.contains("404"), "404 marker missing from error: {msg}");
}
```

**Cross-task decision pinned here:** the dispatcher's `upstream_gone` reason tag (spec §10) is matched against the **error message content**
("404"), not against a dedicated `GitHubProviderError::UpstreamGone` variant. This keeps the wire shape stable and the existing
`fetch_repository_tree` precedent unchanged. Task 11's `tracing::warn!` line that emits `reason = "upstream_gone"` must inspect the error
message for the `"404"` substring before deciding the tag — update Task 11 prose accordingly when transcribing.

`github.rs` already exposes a `TestRuntimeBuilder` (around line 870) used by other internal tests. Use it to construct the runtime with the
mock-server base URL injected:

```rust
let runtime = TestRuntimeBuilder::new()
    .with_api_base_url(server.base_url())
    .with_auth_token("test-token")
    .build()
    .await;
```

(Exact field names may differ; cross-check `TestRuntimeBuilder` for the `api_base_url` / `auth_token` setter names. The
`make_runtime_for_test` identifier used in the snippets above is a placeholder — replace it with the actual
`TestRuntimeBuilder::new()…build()` call when transcribing into the file.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p uptrakit-web-api list_recent_commit_dates_for_path -- --nocapture` Expected: FAIL (default impl returns Misconfigured
but the message doesn't match `404`; short-circuit and ordering assertions fail).

- [ ] **Step 3: Extend the internal executor trait**

`GitHubProviderRuntime` (`github.rs:445-535`) routes every public method through `self.client: Arc<dyn GitHubRequestExecutor>` (trait
defined at line 603). The public wrapper provides the retry shell + `wait_for_cooldown` + Retry-After handling + metrics; the executor does
raw HTTP. **Do not** bypass this with an ad-hoc `http_get_json` — that would silently drop throttling, retries, and rate-limit accounting.

In `crates/ui/web-api/src/global_providers/github.rs`, extend `pub(crate) trait GitHubRequestExecutor` (line 603) with two new methods:

```rust
pub(crate) trait GitHubRequestExecutor: Send + Sync {
    // ... existing fetch_repository_tree ...

    async fn list_commits_for_path_page(
        &self,
        state: &GitHubProviderState,
        owner: &str,
        repo: &str,
        path: &str,
        per_page: usize,
        page: usize,
    ) -> Result<Vec<CommitItem>, RuntimeRequestError>;

    async fn fetch_tree_non_recursive(
        &self,
        state: &GitHubProviderState,
        owner: &str,
        repo: &str,
        tree_sha: &str,
    ) -> Result<Vec<TreeEntry>, RuntimeRequestError>;
}

/// Newest-first commit-by-path projection. Fields parsed from the GitHub
/// `/repos/{owner}/{repo}/commits` response.
pub(crate) struct CommitItem {
    pub root_tree_sha: String,
    pub committed_at: time::OffsetDateTime,
}

/// Non-recursive tree-entry projection. Filtered to `type = "tree"` rows.
pub(crate) struct TreeEntry {
    pub path: String,
    pub sha: String,
}
```

Implement on `ReqwestGitHubRequestExecutor` (line 707) using the same `uptrakit_github_client::GitHubClient` paths already used by
`fetch_repository_tree`. Parse with private serde structs scoped inside each method. Map HTTP failures to `RuntimeRequestError` exactly like
the existing impl does (search `ReqwestGitHubRequestExecutor::fetch_repository_tree` for the precedent —
`429`/`403 + X-RateLimit-Remaining: 0` → `Throttled`, `401` → `AuthFailed`, `404` → `UpstreamGone(format!("404 GET {url}"))` (add this
variant to `RuntimeRequestError` if it does not already carry one — if it does not, use the closest fit and surface the 404 marker so
`GitHubProviderError::Misconfigured("404 ...")` results downstream).

Then add the public `list_recent_commit_dates_for_path` method on `GitHubProviderRuntime` inside the existing
`impl GitHubProviderClient for GitHubProviderRuntime { ... }` block, wrapped in the **same retry shell** as `fetch_repository_tree` (lines
446-535):

```rust
    async fn list_recent_commit_dates_for_path(
        &self,
        consumer: uptrakit_global_github_provider::GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        limit: usize,
        expected_shas: &std::collections::HashSet<String>,
    ) -> std::result::Result<
        Vec<uptrakit_global_github_provider::TreeCommit>,
        uptrakit_global_github_provider::GitHubProviderError,
    > {
        use uptrakit_global_github_provider::TreeCommit;
        let _ = consumer; // reserved for per-consumer accounting.

        const PER_PAGE: usize = 30;
        const HARD_CAP: usize = 90;
        let effective_limit = limit.min(HARD_CAP);
        if effective_limit == 0 {
            return Ok(Vec::new());
        }
        let max_pages = effective_limit.div_ceil(PER_PAGE);

        // Walk skeleton — the per-call retry/cooldown/Retry-After ladder is duplicated
        // from `fetch_repository_tree` (github.rs:446-535), because that ladder is a
        // per-variant `match` on `RuntimeRequestError` with side effects (metrics
        // labels, cooldown updates) that don't fit a single trait-bound helper. The
        // duplication is deliberate: keep the per-variant arms identical between the
        // two methods to preserve observability parity. **Before transcribing, read
        // `fetch_repository_tree` end-to-end and copy its retry-shell match arms
        // verbatim for both executor calls below.**

        // 1. Paged commits walk (newest-first per call; reverse after collection).
        let mut commits: Vec<CommitItem> = Vec::new();
        for page in 1..=max_pages {
            // ── BEGIN retry-shell #1 (mirror fetch_repository_tree:446-535 verbatim) ──
            //   for attempt in 0..=self.retry_policy.max_retries {
            //       self.wait_for_cooldown(state.key_kind).await?;
            //       match self.client.list_commits_for_path_page(
            //           &state, owner, repo, path, PER_PAGE, page,
            //       ).await {
            //           Ok(v) => break Ok(v),
            //           Err(RuntimeRequestError::Throttled { retry_after, .. }) if attempt < max => {
            //               self.set_cooldown(state.key_kind, retry_after).await;
            //               self.sleeper.sleep(self.backoff_for_attempt(attempt)).await;
            //               continue;
            //           }
            //           Err(RuntimeRequestError::AuthFailed(m)) => break Err(GitHubProviderError::AuthFailed(m)),
            //           Err(RuntimeRequestError::NotFound(m))   => break Err(GitHubProviderError::RequestFailed(m)),
            //           Err(RuntimeRequestError::UpstreamUnavailable { message, retry_after: _ }) if attempt < max => {
            //               self.sleeper.sleep(self.backoff_for_attempt(attempt)).await;
            //               continue;
            //           }
            //           Err(other) => break Err(map_runtime_to_provider(other)), // same private mapper fetch_repository_tree uses
            //       }
            //   }
            // ── END retry-shell #1 ──
            let page_resp: Vec<CommitItem> = /* result of retry-shell #1 */;
            if page_resp.is_empty() {
                break;
            }
            for c in page_resp {
                commits.push(c);
                if commits.len() >= effective_limit { break; }
            }
            if commits.len() >= effective_limit { break; }
        }
        if commits.is_empty() {
            return Ok(Vec::new());
        }
        commits.reverse(); // oldest-first

        // 2. Resolve subtree-at-path per commit. Cross-batch cache keyed by
        //    (tree_sha, path_segment). Each cache miss → 1 non-recursive tree call
        //    routed through retry-shell #2 (same ladder as #1).
        let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut subtree_cache: std::collections::HashMap<(String, String), String> =
            std::collections::HashMap::new();

        let mut out: Vec<TreeCommit> = Vec::new();
        for commit in &commits {
            let mut current = commit.root_tree_sha.clone();
            let mut walk_ok = true;
            for seg in &path_segments {
                let cache_key = (current.clone(), (*seg).to_string());
                if let Some(child) = subtree_cache.get(&cache_key) {
                    current = child.clone();
                    continue;
                }
                // ── BEGIN retry-shell #2 (copy fetch_repository_tree ladder verbatim) ──
                // executor call: self.client.fetch_tree_non_recursive(&state, owner, repo, &current).await
                // ── END retry-shell #2 ──
                let tree_entries: Vec<TreeEntry> = /* result of retry-shell #2 */;
                let next = tree_entries.iter().find(|e| e.path == *seg);
                let Some(entry) = next else {
                    walk_ok = false;
                    break;
                };
                subtree_cache.insert(cache_key, entry.sha.clone());
                current = entry.sha.clone();
            }
            if !walk_ok {
                continue; // path renamed at this commit; skip
            }
            out.push(TreeCommit {
                tree_sha_at_path: current.clone(),
                committed_at: commit.committed_at,
            });
            // Short-circuit when every expected SHA is bound.
            if !expected_shas.is_empty()
                && expected_shas
                    .iter()
                    .all(|want| out.iter().any(|tc| &tc.tree_sha_at_path == want))
            {
                break;
            }
        }
        Ok(out)
    }
```

**Concrete identifier list — verify against `github.rs` before writing:**

| Identifier                                 | Where it lives                                                                                                                                                  | Use as                 |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| `self.retry_policy.max_retries`            | field on `GitHubProviderRuntime` (line ~170)                                                                                                                    | retry bound            |
| `self.wait_for_cooldown(key_kind)`         | method (line 315)                                                                                                                                               | pre-call cooldown      |
| `self.set_cooldown(key_kind, retry_after)` | method (search alongside `wait_for_cooldown`)                                                                                                                   | apply Retry-After      |
| `self.sleeper.sleep(backoff)`              | field `sleeper` + trait method                                                                                                                                  | testable sleep         |
| `self.backoff_for_attempt(attempt)`        | method on `GitHubProviderRuntime` (search around `retry_policy`)                                                                                                | exponential backoff    |
| `state.key_kind`                           | field on `GitHubProviderState`                                                                                                                                  | metrics / cooldown key |
| `map_runtime_to_provider`                  | the private fn used by `fetch_repository_tree` for the terminal mapping (grep `RuntimeRequestError` → `GitHubProviderError` translations, around lines 769-792) | terminal error mapping |

If any of these accessors/methods are spelled differently in current code, grep first; use the actual identifier — do **not** invent helpers
like `is_retryable()` or `into_provider_error()` (they do not exist in this codebase).

**Why duplication is acceptable here:**

- The per-variant retry-shell match ladder in `fetch_repository_tree` carries per-variant side effects (metrics labels, cooldown updates,
  Retry-After parsing) that cannot be expressed as a single closure-bound helper without recreating the same side effects in the closure.
- Both new executor calls need identical observability parity with `fetch_repository_tree`, so an honest copy keeps the surfaces aligned and
  detectable on future grep audits.
- A follow-up refactor that extracts the ladder into a shared private method (taking a
  `FnMut() -> impl Future<Output = Result<T, RuntimeRequestError>>`) is fine to do **after** Task 2 lands — but is out of scope here.

### Step 3a — Update both `GitHubRequestExecutor` test doubles (mandatory)

`github.rs` defines two distinct `FakeGitHubRequestExecutor` test doubles (around **line 997** and **line 1027**). Both must implement the
two new methods because `GitHubRequestExecutor` has no default impl (it is an internal trait). For each:

- If the test does not exercise the new methods: return
  `Err(RuntimeRequestError::RequestFailed("test fake: list_commits_for_path_page not configured".into()))` (loud but non-panicking —
  satisfies the "no `unwrap`/`panic!` in production-shape code" snapshot rule even in test code).
- If the test does exercise them: add a tiny outcome queue/builder in the same style as the existing `fetch_repository_tree` test override.

Mention these two line numbers explicitly to the implementer; missing the second fake will fail `cargo check --all-features`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-web-api list_recent_commit_dates_for_path` Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/global_providers/github.rs
git commit -m "feat(web-api/providers): implement list_recent_commit_dates_for_path"
```

---

## Phase 2: Role infrastructure

### Task 3: New `EnrichInstalledVersion` capability variant

**Files:**

- Modify: `crates/shared/types/src/plugin_capability.rs`

**Snapshot rules invoked:** `PluginCapability` is already `#[non_exhaustive]` (line 13) per coding-standards.md:184-228 — append-only.

- [ ] **Step 1: Extend `plugin_capability_all_variants_snake_case`**

In `crates/shared/types/src/plugin_capability.rs`, in the `plugin_capability_all_variants_snake_case` test (line 77), append before the
closing `];`:

```rust
            (
                PluginCapability::EnrichInstalledVersion,
                "enrich_installed_version",
            ),
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p uptrakit-shared-types plugin_capability_all_variants_snake_case` Expected: FAIL — `EnrichInstalledVersion` variant does
not exist.

- [ ] **Step 3: Add the variant**

Add to the `enum PluginCapability` block (line 16+) just below `ConfigTest`:

```rust
    /// Plugin can enrich agent-reported `installed_version` strings with a
    /// human-friendly `installed_display_version` (e.g. a git commit date for
    /// a Skills tree-SHA). Implementations live in `InstalledVersionEnricher`.
    /// Dispatched controller-side from `handle_version_check_results`.
    EnrichInstalledVersion,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p uptrakit-shared-types plugin_capability_all_variants_snake_case` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/types/src/plugin_capability.rs
git commit -m "feat(plugin-types): add EnrichInstalledVersion capability"
```

---

### Task 4: `InstalledVersionEnricher` trait + context type

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs` (re-exports)

**Snapshot rules invoked:** `#[non_exhaustive]` on public structs (coding-standards.md:383-434); `async_trait` per workspace convention;
`rootcause::Report` errors (error-handling.md); feature-gate via `#[cfg(feature = "catalog")]`, never `not(...)`
(coding-standards.md:526-554) — matches existing `ReleaseFetchContext` precedent.

- [ ] **Step 1: Write the failing test**

In `crates/plugins/infrastructure/core/src/roles.rs` tests module (search for `#[cfg(test)] mod tests { ... }`), add:

```rust
#[test]
fn installed_version_enrichment_context_empty() {
    let ctx = crate::InstalledVersionEnrichmentContext::empty();
    #[cfg(feature = "catalog")]
    assert!(ctx.global_provider_lookup.is_none());
    let _ = ctx;
}

#[tokio::test]
async fn installed_version_enricher_trait_is_object_safe() {
    use std::sync::Arc;
    struct Noop;
    #[async_trait::async_trait]
    impl crate::InstalledVersionEnricher for Noop {
        async fn enrich_installed_versions(
            &self,
            items: &[crate::InstalledVersionItem],
        ) -> crate::Result<Vec<crate::InstalledVersionDisplay>> {
            Ok(items
                .iter()
                .map(|i| crate::InstalledVersionDisplay {
                    package_identifier: i.package_identifier.clone(),
                    installed_version_echo: i.installed_version.clone(),
                    display_version: None,
                })
                .collect())
        }
    }
    let arc: Arc<dyn crate::InstalledVersionEnricher> = Arc::new(Noop);
    let items = vec![crate::InstalledVersionItem {
        package_identifier: "x".to_string(),
        installed_version: Some("sha".to_string()),
    }];
    let out = arc.enrich_installed_versions(&items).await.expect("ok");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].installed_version_echo.as_deref(), Some("sha"));
    assert!(out[0].display_version.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p uptrakit-plugin-infrastructure-core installed_version --all-features` Expected: FAIL — `InstalledVersionEnricher` /
`InstalledVersionItem` / `InstalledVersionDisplay` / `InstalledVersionEnrichmentContext` not in scope.

- [ ] **Step 3: Add trait + structs + context**

In `crates/plugins/infrastructure/core/src/roles.rs`, near the existing `ReleaseFetcher` trait + `ReleaseFetchContext`, add:

````rust
use crate::{HostRuntime, Result};

/// Per-item input to an [`InstalledVersionEnricher`].
#[non_exhaustive]
pub struct InstalledVersionItem {
    pub package_identifier: String,
    pub installed_version: Option<String>,
}

impl InstalledVersionItem {
    pub fn new(package_identifier: String, installed_version: Option<String>) -> Self {
        Self {
            package_identifier,
            installed_version,
        }
    }
}

/// Per-item output from an [`InstalledVersionEnricher`].
///
/// The dispatcher zips inputs ↔ outputs **by index** (the returned `Vec` MUST
/// be the same length and order as the input slice). `installed_version_echo`
/// doubles as a contract sanity-check — see the trait doc.
#[non_exhaustive]
pub struct InstalledVersionDisplay {
    pub package_identifier: String,
    pub installed_version_echo: Option<String>,
    pub display_version: Option<String>,
}

impl InstalledVersionDisplay {
    pub fn new(
        package_identifier: String,
        installed_version_echo: Option<String>,
        display_version: Option<String>,
    ) -> Self {
        Self {
            package_identifier,
            installed_version_echo,
            display_version,
        }
    }
}

/// Controller-only role: derive a human-friendly `installed_display_version`
/// from the raw `installed_version` an agent reported. Used when the raw value
/// is opaque (e.g. a git tree SHA) and the display string must come from
/// upstream metadata only the controller can reach.
#[async_trait::async_trait]
pub trait InstalledVersionEnricher: Send + Sync {
    /// Returns a `Vec` of the same length and order as `items`. The
    /// dispatcher zips by index, not by `package_identifier`, so two items
    /// sharing a `package_identifier` (e.g. the same Skill installed on two
    /// hosts with different SHAs) stay distinct. Implementors MUST preserve
    /// order; the dispatcher checks length and treats a mismatch as a fatal
    /// contract violation (warn + drop all display values to `None`).
    ///
    /// **`None`-input contract**: if `items[i].installed_version` is `None`,
    /// implementors MUST return `display_version = None` for that index.
    /// Returning a phantom display for an unknown installed SHA is a
    /// trait-contract violation.
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>>;
}

/// Context object passed to [`InstalledVersionEnricher`] factories at construction
/// time. Mirrors [`ReleaseFetchContext`] (ADR-0015). Carries the optional
/// `GlobalProviderLookup` so Skills can reach the GitHub provider client.
#[non_exhaustive]
pub struct InstalledVersionEnrichmentContext {
    #[cfg(feature = "catalog")]
    pub global_provider_lookup:
        Option<std::sync::Arc<dyn crate::descriptor::GlobalProviderLookup>>,
}

impl InstalledVersionEnrichmentContext {
    /// Construct an empty context (no provider lookup). Available under all feature
    /// combinations; non-catalog builds use this exclusively.
    pub const fn empty() -> Self {
        Self {
            #[cfg(feature = "catalog")]
            global_provider_lookup: None,
        }
    }

    /// Attach a `GlobalProviderLookup` (builder method). Available only with `catalog`.
    /// Call sites that need conditional attachment wrap this in a **single positive**
    /// `#[cfg(feature = "catalog")]` block — never `#[cfg(not(...))]`. Example:
    ///
    /// ```ignore
    /// let mut ctx = InstalledVersionEnrichmentContext::empty();
    /// #[cfg(feature = "catalog")]
    /// {
    ///     ctx = ctx.with_lookup(provider_lookup);
    /// }
    /// ```
    #[cfg(feature = "catalog")]
    pub fn with_lookup(
        mut self,
        lookup: std::sync::Arc<dyn crate::descriptor::GlobalProviderLookup>,
    ) -> Self {
        self.global_provider_lookup = Some(lookup);
        self
    }
}
````

Note: this constructor split satisfies the standards-snapshot binding rule **Feature flags must be additive-only**
(coding-standards.md:526-554) — no `#[cfg(not(feature = "X"))]` attribute on the new code. Non-catalog callers use `empty()`; catalog
callers use `with_lookup_opt`. The existing `ReleaseFetchContext::with_lookup_opt` predates this rule and is grandfathered; do **not**
mirror its `#[cfg(not(...))]` parameter pattern in new code.

In `crates/plugins/infrastructure/core/src/lib.rs`, re-export:

```rust
pub use roles::{
    // ...existing re-exports...
    InstalledVersionDisplay, InstalledVersionEnricher, InstalledVersionEnrichmentContext,
    InstalledVersionItem,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-plugin-infrastructure-core installed_version --all-features` Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/roles.rs crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugin-infra): add InstalledVersionEnricher trait + context"
```

---

### Task 5: `InstalledVersionEnricherSlot` + `RoleCreators` field

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`

**Snapshot rules invoked:** `#[non_exhaustive]` on extensible public structs (coding-standards.md:383-434). The bespoke slot is required
because the factory takes 3 args (config + runtime + context), exactly like `ReleaseFetcherSlot:260-273`.

- [ ] **Step 1: Write the failing test**

In `crates/plugins/infrastructure/core/src/descriptor.rs` tests module, add:

```rust
#[test]
fn installed_version_enricher_slot_const_constructable() {
    use crate::{
        HostRequirements, HostRuntime, InstalledVersionEnricher,
        InstalledVersionEnrichmentContext, InstalledVersionEnricherSlot, Result,
    };
    use std::sync::Arc;

    fn make_dummy(
        _cfg: &serde_json::Value,
        _runtime: Arc<dyn HostRuntime>,
        _ctx: &InstalledVersionEnrichmentContext,
    ) -> Result<Box<dyn InstalledVersionEnricher>> {
        panic!("not invoked in this test");
    }

    const SLOT: InstalledVersionEnricherSlot =
        InstalledVersionEnricherSlot::new(make_dummy, HostRequirements::CONTROLLER_ONLY);
    assert!(SLOT.host_requirements.controller_only);
}

#[test]
fn role_creators_default_installed_version_enricher_is_none() {
    let rc = crate::descriptor::RoleCreators::default();
    assert!(rc.installed_version_enricher.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
`cargo test -p uptrakit-plugin-infrastructure-core installed_version_enricher_slot --all-features role_creators_default_installed_version_enricher_is_none`
Expected: FAIL — types do not exist.

- [ ] **Step 3: Add the slot + field**

In `crates/plugins/infrastructure/core/src/descriptor.rs`, near `ReleaseFetcherSlot` (line 260):

```rust
/// Factory fn pointer for an [`InstalledVersionEnricher`] role. Three-arg
/// shape (config + runtime + context) mirrors [`CreateReleaseFetcherFn`].
pub type CreateInstalledVersionEnricherFn = fn(
    &serde_json::Value,
    std::sync::Arc<dyn crate::HostRuntime>,
    &crate::InstalledVersionEnrichmentContext,
) -> crate::Result<Box<dyn crate::InstalledVersionEnricher>>;

/// Slot for the [`InstalledVersionEnricher`] role. Bespoke (not `RoleSlot<R>`)
/// because the factory needs the enrichment context, identical to
/// [`ReleaseFetcherSlot`].
#[non_exhaustive]
pub struct InstalledVersionEnricherSlot {
    pub create: CreateInstalledVersionEnricherFn,
    pub host_requirements: HostRequirements,
}

impl InstalledVersionEnricherSlot {
    pub const fn new(
        create: CreateInstalledVersionEnricherFn,
        host_requirements: HostRequirements,
    ) -> Self {
        Self {
            create,
            host_requirements,
        }
    }
}
```

In `pub struct RoleCreators { ... }` (line 499), add field:

```rust
    pub installed_version_enricher: Option<InstalledVersionEnricherSlot>,
```

In the `Default` impl for `RoleCreators` (or the `const fn new()` constructor — find it via `impl RoleCreators` block;
descriptor.rs:507-ish), initialize the new field to `None`.

Re-export from `crates/plugins/infrastructure/core/src/lib.rs`:

```rust
pub use descriptor::{
    // ...existing...
    CreateInstalledVersionEnricherFn, InstalledVersionEnricherSlot,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run:
`cargo test -p uptrakit-plugin-infrastructure-core installed_version_enricher_slot role_creators_default_installed_version_enricher_is_none --all-features`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugin-infra): add InstalledVersionEnricherSlot + RoleCreators field"
```

---

### Task 6: `declare_plugin!` macro — `installed_version_enricher_create:` field

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/macros.rs`

**Snapshot rules invoked:** Macro hygiene + DRY — mirror existing `release_fetcher_create:` field exactly (line 52 of macros.rs).

- [ ] **Step 1: Write the failing test (descriptor-builder smoke)**

In `crates/plugins/infrastructure/core/src/macros.rs` tests module (search for `#[cfg(test)] mod tests`), or in
`crates/plugins/infrastructure/core/tests/macro_smoke.rs` if integration-style tests live there, add:

```rust
#[test]
fn declare_plugin_accepts_installed_version_enricher_create() {
    // Compile-only smoke test: if this builds, the macro accepts the field.
    use crate::{
        ConfigModel, HostRequirements, HostRuntime, InstalledVersionEnricher,
        InstalledVersionEnrichmentContext, PluginFamily, Result,
    };
    use std::sync::Arc;

    struct DummyConfig;
    impl serde::Serialize for DummyConfig {
        fn serialize<S: serde::Serializer>(&self, _: S) -> std::result::Result<S::Ok, S::Error> {
            unreachable!()
        }
    }
    impl<'de> serde::Deserialize<'de> for DummyConfig {
        fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
            unreachable!()
        }
    }
    struct DummyPlugin;
    #[async_trait::async_trait]
    impl InstalledVersionEnricher for DummyPlugin {
        async fn enrich_installed_versions(
            &self,
            _items: &[crate::InstalledVersionItem],
        ) -> Result<Vec<crate::InstalledVersionDisplay>> {
            unreachable!()
        }
    }

    fn factory(
        _cfg: &serde_json::Value,
        _runtime: Arc<dyn HostRuntime>,
        _ctx: &InstalledVersionEnrichmentContext,
    ) -> Result<Box<dyn InstalledVersionEnricher>> {
        Ok(Box::new(DummyPlugin))
    }

    // Reuse the existing declare_plugin! macro path — pulled into scope via
    // the existing test setup. If the macro silently ignores the new key the
    // descriptor's slot will be None and the assertion fails.
    crate::declare_plugin!(DummyPlugin, DummyConfig, "test_dummy_ive", {
        display_name: "Dummy",
        family: PluginFamily::Software,
        config_model: ConfigModel::PluginConfig,
        host_requirements: HostRequirements::POSIX,
        roles: [],
        installed_version_enricher_create: {
            create: factory,
            host_requirements: HostRequirements::CONTROLLER_ONLY,
        },
    });
    assert!(
        DESCRIPTOR.roles.installed_version_enricher.is_some(),
        "macro must populate the slot"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p uptrakit-plugin-infrastructure-core declare_plugin_accepts_installed_version_enricher_create --all-features` Expected:
FAIL — macro doesn't accept the key.

- [ ] **Step 3: Extend the macro**

In `crates/plugins/infrastructure/core/src/macros.rs`, around line 52, add the new optional field parallel to `release_fetcher_create:`:

```rust
            $(, release_fetcher_create: {
                create: $rf_create:expr,
                host_requirements: $rf_hr:expr $(,)?
            } )?
            $(, installed_version_enricher_create: {
                create: $ive_create:expr,
                host_requirements: $ive_hr:expr $(,)?
            } )?
```

In the macro body that builds `RoleCreators` (around line 243 — the block that sets `rc.controller_update_protection = Some(...)`), add:

```rust
            $(
                rc.installed_version_enricher = Some(
                    $crate::InstalledVersionEnricherSlot::new($ive_create, $ive_hr),
                );
            )?
```

Also extend the **four** helper macros so the `InstalledVersionEnricher` role identifier is accepted when listed inside `roles: [...]`. The
existing macros (verified in `macros.rs`) match exhaustively on each role name — adding the identifier without these arms produces a "no
rules matched" macro error.

**`__assert_role_impl!`** (lines 305-370) — add an arm mirroring `ReleaseFetcher` (lines 322-329):

```rust
    ($plugin:ty, InstalledVersionEnricher) => {
        const _: () = {
            fn _assert<T: $crate::InstalledVersionEnricher>() {}
            fn _check() {
                _assert::<$plugin>();
            }
        };
    };
```

**`__define_role_creator!`** (lines 379+) — singleton role, **no creator**. Add an empty arm mirroring
`NotificationTransport`/`SoftwareItemLifecycle` (the per-instance creator path is unused because the slot is populated via the bespoke
`installed_version_enricher_create:` key):

```rust
    ($plugin:ty, $config:ty, InstalledVersionEnricher) => {};
```

**`__set_role_field!`** (lines 500-540) — singleton role, **no field set**. Add an empty arm mirroring
`NotificationTransport`/`SoftwareItemLifecycle` (lines 538-539):

```rust
    ($rc:ident, InstalledVersionEnricher, $hr:expr) => {};
```

**`__accumulate_role_caps!`** (lines 627+) — contribute **no implicit capability** (the gating bit `EnrichInstalledVersion` is declared
explicitly via `extra_capabilities:`). Add a single arm matching the role identifier; trailing tokens (including any optional
`{ host_requirements: ... }` block) are consumed by `$($rest:tt)*` exactly like the existing `NotificationTransport` arm at lines 684-689:

```rust
    ( [ $($acc:expr),* ], InstalledVersionEnricher $($rest:tt)* ) => {
        $crate::__accumulate_role_caps!([ $($acc),* ] $($rest)*)
    };
```

Final compile-time-impl assertion via the optional field (separate from `__assert_role_impl!`, because the singleton role lives outside the
`roles: [...]` list):

```rust
            $(
                const _: fn() = || {
                    fn _assert<T: $crate::InstalledVersionEnricher>() {}
                    let _ = $ive_create; // ensures the fn type matches
                };
            )?
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p uptrakit-plugin-infrastructure-core declare_plugin_accepts_installed_version_enricher_create --all-features` Expected:
PASS.

- [ ] **Step 5: Verify other plugins still compile**

Run: `cargo check --all-features` Expected: clean — the new field is optional.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/core/src/macros.rs
git commit -m "feat(plugin-infra): declare_plugin! supports installed_version_enricher_create"
```

---

## Phase 3: Skills `ReleaseFetcher::batch_fetch` emits commit-date display

### Task 7: `DISPLAY_FMT` constant + `batch_fetch` switches to new primitive

**Files:**

- Modify: `crates/plugins/package-managers/skills/src/releases.rs`

**Snapshot rules invoked:** `time::macros::format_description!` (NOT `Rfc3339` — frontend regex `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$`
requires no subsecond precision); no `unwrap` in production.

- [ ] **Step 1: Extend the existing test `fetch_releases_skill_folder_found_returns_one_release`**

In `crates/plugins/package-managers/skills/src/releases.rs` tests module, modify the existing test (lines ~406-432) — the `FakeProvider`
currently returns a fixed tree. Switch the test fake to also return commit-date data. Add a new test alongside:

```rust
#[tokio::test]
async fn fetch_releases_sets_display_version_to_iso_8601_commit_date() {
    use std::collections::HashSet;
    let sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    struct DateProvider;
    #[async_trait]
    impl GitHubProviderClient for DateProvider {
        async fn fetch_repository_tree(
            &self,
            _: GlobalProviderConsumerId,
            _: &str,
            _: &str,
            _: &str,
            _: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            unreachable!("batch_fetch must not call fetch_repository_tree anymore");
        }
        async fn list_recent_commit_dates_for_path(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _path: &str,
            _limit: usize,
            _expected: &HashSet<String>,
        ) -> std::result::Result<Vec<uptrakit_global_github_provider::TreeCommit>, GitHubProviderError>
        {
            // Newest-first per provider contract.
            Ok(vec![uptrakit_global_github_provider::TreeCommit {
                tree_sha_at_path: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
                committed_at: time::macros::datetime!(2026-06-11 01:15:00 UTC),
            }])
        }
    }

    let plugin = make_plugin_with_provider(Arc::new(DateProvider));
    let id = skill_id(
        "https://github.com/obra/superpowers",
        "skills/brainstorming/SKILL.md",
    );
    let releases = plugin.fetch_releases(&id).await.expect("fetch ok");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].tag, sha, "tag still carries the SHA");
    assert_eq!(
        releases[0].display_version.as_deref(),
        Some("2026-06-11T01:15:00Z"),
        "display_version is strict ISO 8601 UTC second-precision"
    );
}
```

(`make_plugin_with_provider`, `skill_id` already exist as test helpers in the same module — see lines 325-335 of the current file.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p uptrakit-plugin-package-manager-skills fetch_releases_sets_display_version` Expected: FAIL —
`list_recent_commit_dates_for_path` not called; default impl returns Misconfigured.

- [ ] **Step 3: Add `DISPLAY_FMT` + rewrite `batch_fetch` / `fetch_releases`**

At the top of `crates/plugins/package-managers/skills/src/releases.rs`, add:

```rust
const DISPLAY_FMT: &[time::format_description::FormatItem<'_>] = time::macros::format_description!(
    "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
);
```

Replace the body of `batch_fetch` and `fetch_releases` to call
`provider.list_recent_commit_dates_for_path(consumer, owner, repo, skill_dir, 90, &HashSet::new())` instead of `fetch_repository_tree`. Use
entry `[entries.len() - 1]` (newest, since the new primitive returns oldest-first) — equivalently, the last entry. Set:

```rust
let last = entries.last(); // newest-first now lives at end; per provider contract returns oldest-first
let Some(top) = last else {
    return Ok(vec![]);
};
let display_version = top
    .committed_at
    .format(&DISPLAY_FMT)
    .ok();
let release = UpstreamRelease::new(
    Version::new(top.tree_sha_at_path.clone()),
    top.tree_sha_at_path.clone(),
    false,
    format!("https://github.com/{owner}/{repo}/tree/HEAD/{skill_dir}"),
);
let release = UpstreamRelease {
    display_version,
    ..release
};
Ok(vec![release])
```

(If `UpstreamRelease::new` already returns a builder-like struct without `display_version` as a public field, use the public setter or
struct-update pattern that exists today. Check `crates/plugins/infrastructure/core/src/types.rs:60-80` for the canonical shape.)

For `batch_fetch`, group by `(owner, repo, skill_dir)`, build `expected_shas` from `items.installed_version` if any are passed through (none
today — batch_fetch only handles latest), call once per group, map the newest entry to each `BatchFetchResult::found`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p uptrakit-plugin-package-manager-skills fetch_releases_sets_display_version_to_iso_8601_commit_date` Expected: PASS.

- [ ] **Step 5: Update existing `FakeProvider` / `CountingProvider` / error-path test doubles**

Search the tests module for every `impl GitHubProviderClient for FakeProvider` / `CountingProvider` / `ThrottledProvider` /
`AuthFailedProvider` / `MisconfiguredProvider` (lines ~359-545). Each test that depends on the new code path needs its provider to override
`list_recent_commit_dates_for_path` returning the appropriate `TreeCommit` vec / error. Test doubles whose tests still cover only
error-mapping (Throttled, AuthFailed, Misconfigured) can return the error from the new method instead.

- [ ] **Step 6: Run full skills test suite**

Run: `cargo test -p uptrakit-plugin-package-manager-skills` Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add crates/plugins/package-managers/skills/src/releases.rs
git commit -m "feat(plugins/skills): emit commit-date display_version from batch_fetch"
```

---

## Phase 4: Skills `InstalledVersionEnricher` impl

### Task 8: `enrich_installed_versions` on `SkillsPlugin`

**Files:**

- Modify: `crates/plugins/package-managers/skills/src/releases.rs` (or new `enricher.rs` if file grows too long — the team's convention
  prefers small focused files; add `enricher.rs` if `releases.rs` exceeds ~700 lines after Task 7).

**Snapshot rules invoked:** rootcause::Report errors; no unwrap in production; async-trait via `#[async_trait]`.

- [ ] **Step 1: Write the failing tests**

Add a new test module or extend the existing one in `releases.rs` (or `enricher.rs`):

```rust
#[tokio::test]
async fn enrich_installed_versions_maps_known_sha_to_commit_date() {
    use std::collections::HashSet;
    use uptrakit_global_github_provider::TreeCommit;
    use uptrakit_plugin_infrastructure_core::{
        InstalledVersionDisplay, InstalledVersionEnricher, InstalledVersionItem,
    };

    struct P;
    #[async_trait]
    impl GitHubProviderClient for P {
        async fn fetch_repository_tree(
            &self,
            _: GlobalProviderConsumerId, _: &str, _: &str, _: &str, _: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            unreachable!()
        }
        async fn list_recent_commit_dates_for_path(
            &self,
            _: GlobalProviderConsumerId, _: &str, _: &str, _: &str, _: usize,
            _expected: &HashSet<String>,
        ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
            Ok(vec![
                TreeCommit { tree_sha_at_path: "sha_old".to_string(),
                    committed_at: time::macros::datetime!(2026-04-01 00:00:00 UTC) },
                TreeCommit { tree_sha_at_path: "sha_new".to_string(),
                    committed_at: time::macros::datetime!(2026-06-11 01:15:00 UTC) },
            ])
        }
    }
    let plugin = make_plugin_with_provider(Arc::new(P));
    let items = vec![
        InstalledVersionItem {
            package_identifier: skill_id(
                "https://github.com/obra/superpowers",
                "skills/brainstorming/SKILL.md",
            ),
            installed_version: Some("sha_new".to_string()),
        },
        InstalledVersionItem {
            package_identifier: skill_id(
                "https://github.com/obra/superpowers",
                "skills/dispatching/SKILL.md",
            ),
            installed_version: Some("not_in_window".to_string()),
        },
        InstalledVersionItem {
            package_identifier: skill_id(
                "https://github.com/obra/superpowers",
                "skills/empty/SKILL.md",
            ),
            installed_version: None,
        },
    ];

    let out = plugin.enrich_installed_versions(&items).await.expect("ok");

    assert_eq!(out.len(), 3, "must preserve length");
    assert_eq!(out[0].installed_version_echo.as_deref(), Some("sha_new"));
    assert_eq!(out[0].display_version.as_deref(), Some("2026-06-11T01:15:00Z"));
    assert_eq!(out[1].installed_version_echo.as_deref(), Some("not_in_window"));
    assert_eq!(out[1].display_version, None, "miss → None");
    assert_eq!(out[2].installed_version, /* ignored */ out[2].installed_version);
    assert_eq!(out[2].installed_version_echo, None);
    assert_eq!(out[2].display_version, None, "None-input contract");
}

#[tokio::test]
async fn enrich_installed_versions_groups_by_skill_dir_to_minimize_calls() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::collections::HashSet;
    use uptrakit_global_github_provider::TreeCommit;
    use uptrakit_plugin_infrastructure_core::{
        InstalledVersionEnricher, InstalledVersionItem,
    };

    struct Counting {
        calls: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl GitHubProviderClient for Counting {
        async fn fetch_repository_tree(
            &self,
            _: GlobalProviderConsumerId, _: &str, _: &str, _: &str, _: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            unreachable!()
        }
        async fn list_recent_commit_dates_for_path(
            &self,
            _: GlobalProviderConsumerId, _: &str, _: &str, _: &str, _: usize,
            _: &HashSet<String>,
        ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let plugin = make_plugin_with_provider(Arc::new(Counting { calls: calls.clone() }));

    let id1 = skill_id("https://github.com/obra/superpowers", "skills/a/SKILL.md");
    let id2 = skill_id("https://github.com/obra/superpowers", "skills/a/SKILL.md"); // same dir
    let id3 = skill_id("https://github.com/obra/superpowers", "skills/b/SKILL.md");
    let items = vec![
        InstalledVersionItem { package_identifier: id1, installed_version: Some("x".into()) },
        InstalledVersionItem { package_identifier: id2, installed_version: Some("y".into()) },
        InstalledVersionItem { package_identifier: id3, installed_version: Some("z".into()) },
    ];
    let _ = plugin.enrich_installed_versions(&items).await.expect("ok");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one call per unique (owner, repo, skill_dir)"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p uptrakit-plugin-package-manager-skills enrich_installed_versions` Expected: FAIL — no
`InstalledVersionEnricher for SkillsPlugin` impl yet.

- [ ] **Step 3: Add the impl**

Append to `releases.rs` (or new `enricher.rs` module declared in `crates/plugins/package-managers/skills/src/lib.rs`):

```rust
use std::collections::{HashMap, HashSet};
use uptrakit_plugin_infrastructure_core::{
    InstalledVersionDisplay, InstalledVersionEnricher, InstalledVersionItem, PluginError, Result,
};

#[async_trait]
impl InstalledVersionEnricher for SkillsPlugin {
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            report!(SkillsError::ProviderUnavailable(
                "skills installed-version enrichment requires the global GitHub provider"
                    .to_string()
            ))
        }).context_to::<PluginError>()?;

        // Resolve every item to (Option<(owner, repo, skill_dir)>) up front.
        struct Resolved {
            owner: String,
            repo: String,
            skill_dir: String,
        }
        let mut resolved: Vec<Option<Resolved>> = Vec::with_capacity(items.len());
        for item in items {
            if item.installed_version.is_none() {
                resolved.push(None);
                continue;
            }
            let parsed = (|| -> Result<Resolved> {
                let (source_url, skill_path) =
                    parse_skill_identifier(&item.package_identifier).context_to()?;
                let (owner, repo) = parse_github_owner_repo(&source_url)?;
                let skill_dir = derive_skill_dir(&skill_path).to_string();
                Ok(Resolved { owner, repo, skill_dir })
            })();
            resolved.push(parsed.ok());
        }

        // Group expected SHAs by (owner, repo, skill_dir).
        let mut expected_by_key: HashMap<(String, String, String), HashSet<String>> = HashMap::new();
        for (i, r) in resolved.iter().enumerate() {
            let Some(r) = r else { continue };
            let Some(ref sha) = items[i].installed_version else { continue };
            let key = (r.owner.clone(), r.repo.clone(), r.skill_dir.clone());
            expected_by_key.entry(key).or_default().insert(sha.clone());
        }

        // One provider call per group; cache result for the duration of the batch.
        let mut dates_by_key: HashMap<(String, String, String), HashMap<String, time::OffsetDateTime>> = HashMap::new();
        for (key, expected) in &expected_by_key {
            let resp = provider
                .list_recent_commit_dates_for_path(
                    PACKAGE_MANAGER_SKILLS,
                    &key.0,
                    &key.1,
                    &key.2,
                    90,
                    expected,
                )
                .await;
            let map = match resp {
                Ok(entries) => entries
                    .into_iter()
                    .map(|tc| (tc.tree_sha_at_path, tc.committed_at))
                    .collect::<HashMap<_, _>>(),
                Err(e) => {
                    tracing::warn!(
                        owner = %key.0, repo = %key.1, path = %key.2,
                        error = %e,
                        reason = "provider_error",
                        "installed-version enrichment: provider call failed; skipping group"
                    );
                    HashMap::new()
                }
            };
            dates_by_key.insert(key.clone(), map);
        }

        // Build the per-item output preserving order.
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let display_version = match (&resolved[i], &item.installed_version) {
                (Some(r), Some(sha)) => {
                    let key = (r.owner.clone(), r.repo.clone(), r.skill_dir.clone());
                    dates_by_key
                        .get(&key)
                        .and_then(|m| m.get(sha))
                        .and_then(|dt| dt.format(&DISPLAY_FMT).ok())
                }
                _ => None,
            };
            out.push(InstalledVersionDisplay {
                package_identifier: item.package_identifier.clone(),
                installed_version_echo: item.installed_version.clone(),
                display_version,
            });
        }
        Ok(out)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-plugin-package-manager-skills enrich_installed_versions` Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/package-managers/skills/src/releases.rs crates/plugins/package-managers/skills/src/lib.rs
git commit -m "feat(plugins/skills): implement InstalledVersionEnricher"
```

---

## Phase 5: Web-api dispatch

### Task 9: `plugin_types_for_role` tenant-scoped query

**Files:**

- Create: `crates/ui/web-api-queries/src/queries/host_software_item_plugins/mod.rs`
- Modify: `crates/ui/web-api-queries/src/queries/mod.rs` (declare new module)

**Snapshot rules invoked:** Tenant-scoped join (AGENTS.md:280) — must use `TenantDb::find_via_tenant_join`.
`host_software_item_plugin::Entity` is NOT `TenantScoped`; join through `software_item::Entity` via `Relation::SoftwareItem`.
`rootcause::Report` errors.

- [ ] **Step 1: Write the failing test**

Create `crates/ui/web-api-queries/src/queries/host_software_item_plugins/tests.rs` (referenced via `#[cfg(test)] mod tests;` in `mod.rs`):

```rust
use crate::queries::host_software_item_plugins::plugin_types_for_role;
use uptrakit_shared_db::entity::*;
use sea_orm::ActiveModelTrait;

// helper: bootstrap_test_db() returns (DatabaseConnection, tenant_a_id, tenant_b_id, hsi_ids)
async fn bootstrap() -> /* (db, tenant_a, tenant_b, hsi_a_id) */ ... { /* see existing test patterns in web-api-queries */ }

#[tokio::test]
async fn plugin_types_for_role_returns_assignment_for_detect_version() {
    let (db, tenant_a, _tenant_b, hsi_a_id) = bootstrap().await;
    let tenant_db = uptrakit_web_api_auth::TenantDb::new(db.clone(), tenant_a);
    let out = plugin_types_for_role(&tenant_db, &[hsi_a_id], "detect_version")
        .await
        .expect("ok");
    let assignment = out.get(&hsi_a_id).expect("present");
    assert_eq!(assignment.plugin_type, "package_manager_skills");
    assert_eq!(
        assignment.package_identifier,
        "https://github.com/obra/superpowers#skills/brainstorming/SKILL.md"
    );
}

#[tokio::test]
async fn plugin_types_for_role_excludes_other_tenants() {
    let (db, _tenant_a, tenant_b, hsi_a_id) = bootstrap().await;
    let tenant_db = uptrakit_web_api_auth::TenantDb::new(db.clone(), tenant_b);
    let out = plugin_types_for_role(&tenant_db, &[hsi_a_id], "detect_version")
        .await
        .expect("ok");
    assert!(
        out.is_empty(),
        "tenant B must not see tenant A's host_software_item_plugin rows"
    );
}
```

Implement `bootstrap()` using the existing test-DB setup pattern in `web-api-queries` — search for `enable_plaintext_mode` +
`bootstrap_test_db` invocations in sibling query modules and copy the harness. Test fixture must insert: two `tenant` rows; two
`software_item` rows (one per tenant); a `host_software_item` row under tenant A; a `host_software_item_plugin` row pointing to that hsi
with role `detect_version` and plugin_type `package_manager_skills`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p uptrakit-web-api-queries plugin_types_for_role` Expected: FAIL — function does not exist.

- [ ] **Step 3: Implement the query**

In `crates/ui/web-api-queries/src/queries/host_software_item_plugins/mod.rs`:

```rust
//! Tenant-scoped lookups against `host_software_item_plugin`.
//!
//! `host_software_item_plugin::Entity` is NOT `TenantScoped`; this module
//! enforces tenant isolation via `TenantDb::find_via_tenant_join` through
//! `software_item::Entity` (which IS `TenantScoped`).

use std::collections::HashMap;

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, DbErr, EntityTrait, FromQueryResult, QueryFilter, QuerySelect, RelationTrait,
};
use thiserror::Error;
use uptrakit_shared_db::entity::{host_software_item_plugin, software_item};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_auth::TenantDb;
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HostSoftwareItemPluginQueryError {
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

impl_report_conversion!(
    HostSoftwareItemPluginQueryError => uptrakit_plugin_infrastructure_core::PluginError,
    |e: HostSoftwareItemPluginQueryError| {
        uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
    }
);

pub type Result<T> = std::result::Result<T, rootcause::Report<HostSoftwareItemPluginQueryError>>;

#[derive(Debug, FromQueryResult)]
struct PluginAssignmentRow {
    host_software_item_id: Uuid,
    plugin_type: String,
    package_identifier: String,
}

/// Per-host_software_item record returned by [`plugin_types_for_role`]. Includes
/// `package_identifier` so the caller can pass it into the per-plugin enrichment
/// batch without a second DB round-trip — `VersionCheckResult` from the wire does
/// not carry it (it lives on `host_software_item_plugin`).
#[non_exhaustive]
pub struct PluginAssignment {
    pub plugin_type: String,
    pub package_identifier: String,
}

/// Return `host_software_item_id → PluginAssignment` for the given hsi ids
/// restricted to `role`. Tenant-scoped via `host_software_item_plugin →
/// software_item`. Rows belonging to other tenants are silently excluded by the
/// join — they never appear in the result map.
pub async fn plugin_types_for_role(
    tenant_db: &TenantDb,
    host_software_item_ids: &[Uuid],
    role: &str,
) -> Result<HashMap<Uuid, PluginAssignment>> {
    if host_software_item_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = tenant_db
        .find_via_tenant_join::<host_software_item_plugin::Entity, software_item::Entity>(
            host_software_item_plugin::Relation::SoftwareItem.def(),
        )
        .filter(
            host_software_item_plugin::Column::HostSoftwareItemId
                .is_in(host_software_item_ids.iter().copied()),
        )
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .select_only()
        .column(host_software_item_plugin::Column::HostSoftwareItemId)
        .column(host_software_item_plugin::Column::PluginType)
        .column(host_software_item_plugin::Column::PackageIdentifier)
        .into_model::<PluginAssignmentRow>()
        .all(tenant_db.db())
        .await
        .map_err(|e| report!(HostSoftwareItemPluginQueryError::Database(e)))?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.host_software_item_id,
                PluginAssignment {
                    plugin_type: r.plugin_type,
                    package_identifier: r.package_identifier,
                },
            )
        })
        .collect())
}
```

**Why this shape:**

- **Local error type** — the workspace does not have a shared `WebApiQueriesError`; every query module defines its own
  (`SoftwareItemQueryError`, `ServiceQueryError`, etc., confirmed at `web-api-queries/src/queries/software_items/mod.rs:42` and siblings).
  Follow the precedent: `HostSoftwareItemPluginQueryError { Database(DbErr) }` keeps the typed `DbErr` per binding rule against
  `Result<T, String>` (coding-standards.md:15-18).
- **`into_model::<PluginAssignmentRow>()`** — every multi-column `select_only` in the workspace uses `#[derive(FromQueryResult)]` +
  `into_model::<...>()` (confirmed in `queries/hosts.rs:117`, `queries/update_batches/queries.rs:69`, `queries/software_states.rs:133`). The
  `into_tuple()` form is reserved for single-column extractions.
- **`tenant_db.db()`** — confirmed accessor name throughout `web-api-queries` (see `queries/update_batches/queries.rs:41,46,70,117,126`).
  Not `.connection()`.
- **Returns `PluginAssignment` (not bare `String`)** — `VersionCheckResult` (`crates/shared/wire/src/payloads.rs:409-449`) carries no
  `package_identifier`; the enricher dispatch in Task 11 needs it per item. Including `package_identifier` in the same query avoids a second
  DB hop in `handle_version_check_results`.

In `crates/ui/web-api-queries/src/queries/mod.rs`, add `pub mod host_software_item_plugins;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-web-api-queries plugin_types_for_role` Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/host_software_item_plugins/ crates/ui/web-api-queries/src/queries/mod.rs
git commit -m "feat(web-api-queries): tenant-scoped plugin_types_for_role lookup"
```

---

### Task 10: `apply_version_update_to_db` override parameter

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` (around line 747).

**Snapshot rules invoked:** Atomic single-UPDATE invariant (spec §7); rootcause::Report errors.

- [ ] **Step 1: Write the failing test**

In the existing `messages.rs` tests module (search for the existing `handle_version_check_results` tests around line 2899), add:

```rust
#[tokio::test]
async fn apply_version_update_to_db_writes_override_into_installed_display_version() {
    let (state, svc, hsi_id) = setup_with_one_hsi().await; // existing helper or build inline
    let now = time::OffsetDateTime::now_utc();
    let result = uptrakit_wire::VersionCheckResult {
        software_item_id: svc.software_item_id,
        installed_version: Some("sha_abc".to_string()),
        installed_display_version: None, // agent sent nothing
        latest_version: None,
        update_category: uptrakit_shared_types::UpdateCategory::Unknown,
        error: None,
        host_software_item_id: Some(hsi_id),
        ..Default::default()
    };
    apply_version_update_to_db(
        &state.db,
        &result,
        vec![hsi_id],
        now,
        DisplayOverride::Override(Some("2026-06-11T01:15:00Z".to_string())),
    )
    .await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.installed_version.as_deref(), Some("sha_abc"));
    assert_eq!(
        row.installed_display_version.as_deref(),
        Some("2026-06-11T01:15:00Z")
    );
}

#[tokio::test]
async fn apply_version_update_to_db_override_clear_overwrites_prior_display() {
    let (state, svc, hsi_id) = setup_with_existing_display("old_display").await;
    let now = time::OffsetDateTime::now_utc();
    let result = uptrakit_wire::VersionCheckResult {
        software_item_id: svc.software_item_id,
        installed_version: Some("sha_new".to_string()),
        installed_display_version: Some("legacy_agent_supplied".to_string()),
        latest_version: None,
        update_category: uptrakit_shared_types::UpdateCategory::Unknown,
        error: None,
        host_software_item_id: Some(hsi_id),
        ..Default::default()
    };

    // Enricher ran but returned no display for this SHA → Override(None).
    apply_version_update_to_db(&state.db, &result, vec![hsi_id], now, DisplayOverride::Override(None)).await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.installed_version.as_deref(), Some("sha_new"));
    assert_eq!(
        row.installed_display_version, None,
        "Override(None) must overwrite prior display"
    );
}

#[tokio::test]
async fn apply_version_update_to_db_use_agent_value_preserves_wire_value() {
    let (state, svc, hsi_id) = setup_with_one_hsi().await;
    let now = time::OffsetDateTime::now_utc();
    let result = uptrakit_wire::VersionCheckResult {
        software_item_id: svc.software_item_id,
        installed_version: Some("sha_zzz".to_string()),
        installed_display_version: Some("docker_supplied_date".to_string()),
        host_software_item_id: Some(hsi_id),
        ..Default::default()
    };

    // No enricher applies → UseAgentValue preserves the wire-supplied display.
    apply_version_update_to_db(&state.db, &result, vec![hsi_id], now, DisplayOverride::UseAgentValue).await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(&state.db).await.unwrap().unwrap();
    assert_eq!(row.installed_display_version.as_deref(), Some("docker_supplied_date"));
}
```

`setup_with_one_hsi` and `setup_with_existing_display` follow the existing test-harness conventions in this file — search for the pattern at
messages.rs:2870-2950.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p uptrakit-web-api apply_version_update_to_db_writes_override apply_version_update_to_db_override_none_overwrites`
Expected: FAIL — `apply_version_update_to_db` does not take the new arg.

- [ ] **Step 3: Add the override parameter**

In `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`, first add a small dedicated enum at module scope. The
`Option<Option<String>>` shape is a known footgun (the workspace memory rule on nullable update payloads — `feedback_*` — recommends typed
enums for tri-state semantics):

```rust
/// Tri-state for the installed-display-version write path.
///
/// `UseAgentValue` preserves backward-compatible behaviour: the value comes
/// straight from `result.installed_display_version` (the wire payload).
/// `Override(Some(s))` writes the supplied display string. `Override(None)`
/// explicitly clears the column. The dispatcher in
/// `handle_version_check_results` constructs the value from enricher output:
/// no enricher applies → `UseAgentValue`; enricher ran and returned a string →
/// `Override(Some(...))`; enricher ran and returned `None` (miss / throttle /
/// out-of-window) → `Override(None)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisplayOverride {
    UseAgentValue,
    Override(Option<String>),
}
```

Then change `apply_version_update_to_db` (currently `fn apply_version_update_to_db(state, result, matching_ids, now)` around line 747):

```rust
pub(super) async fn apply_version_update_to_db(
    db: &sea_orm::DatabaseConnection,
    result: &uptrakit_wire::VersionCheckResult,
    matching_ids: Vec<uuid::Uuid>,
    now: time::OffsetDateTime,
    installed_display_override: DisplayOverride,
) {
    // ... existing body ...
}
```

Where the call site at line 776 sets `InstalledDisplayVersion`, change:

```rust
.col_expr(
    host_software_item::Column::InstalledDisplayVersion,
    sea_orm::sea_query::Expr::value(result.installed_display_version.clone()),
);
```

to:

```rust
.col_expr(
    host_software_item::Column::InstalledDisplayVersion,
    sea_orm::sea_query::Expr::value(match &installed_display_override {
        DisplayOverride::UseAgentValue => result.installed_display_version.clone(),
        DisplayOverride::Override(value) => value.clone(),
    }),
);
```

Update every existing call site of `apply_version_update_to_db` in `messages.rs` to pass `DisplayOverride::UseAgentValue` for the new
parameter, preserving today's behavior.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-web-api apply_version_update_to_db` Expected: PASS.

- [ ] **Step 5: Run full messages tests to confirm no regressions**

Run: `cargo test -p uptrakit-web-api handle_version_check_results` Expected: all existing tests pass (they pass `None` for the new arg).

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/routes/service_ws/handler/messages.rs
git commit -m "feat(web-api): apply_version_update_to_db accepts display override"
```

---

### Task 11: Typed-slot dispatch in `handle_version_check_results`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` (around line 937 — `handle_version_check_results`).

**Snapshot rules invoked:** ADR-0018 (typed plugin boundary — no plugin_type strings); tenant isolation via `TenantDb`; tracing `skip_all`
(AGENTS.md:296); rootcause::Report errors; atomic-update invariant preserved through Task 10.

- [ ] **Step 1: Write the failing tests**

In `messages.rs` tests module:

```rust
#[tokio::test]
async fn handle_version_check_results_invokes_enricher_for_capable_plugin() {
    // Fixture: one host_software_item assigned to a stub plugin descriptor
    // that declares `EnrichInstalledVersion` and an InstalledVersionEnricher
    // factory returning Some("2026-06-11T01:15:00Z") for the test SHA.
    let (state, svc, hsi_id) = setup_for_enricher_test(
        /* plugin_type */ "test_enricher_plugin",
        /* installed_sha */ "abc123",
    ).await;
    let payload = uptrakit_wire::VersionCheckResultsPayload {
        host_machine_id: svc.host_machine_id.clone(),
        results: vec![uptrakit_wire::VersionCheckResult {
            software_item_id: svc.software_item_id,
            installed_version: Some("abc123".to_string()),
            installed_display_version: None,
            host_software_item_id: Some(hsi_id),
            ..Default::default()
        }],
    };

    handle_version_check_results(&state, svc.id, &payload).await;

    let row = host_software_item::Entity::find_by_id(hsi_id)
        .one(&state.db).await.unwrap().unwrap();
    assert_eq!(
        row.installed_display_version.as_deref(),
        Some("2026-06-11T01:15:00Z"),
        "enricher output must flow into the write"
    );
}

#[tokio::test]
async fn handle_version_check_results_does_not_invoke_enricher_without_capability() {
    let (state, svc, hsi_id) = setup_for_enricher_test_uncapable("test_plain_plugin", "xyz").await;
    let payload = /* same shape, installed_version Some("xyz") */ ;
    handle_version_check_results(&state, svc.id, &payload).await;
    let row = host_software_item::Entity::find_by_id(hsi_id).one(&state.db).await.unwrap().unwrap();
    assert!(row.installed_display_version.is_none(), "no enricher → no override");
}

#[tokio::test]
async fn handle_version_check_results_writes_none_when_enricher_misses() {
    // Enricher returns Vec entry with display_version=None for this SHA.
    // Fixture pre-seeds installed_display_version="old".
    let (state, svc, hsi_id) = setup_with_existing_display_and_enricher("old").await;
    let payload = /* installed_version Some("new"), display None */ ;
    handle_version_check_results(&state, svc.id, &payload).await;
    let row = host_software_item::Entity::find_by_id(hsi_id).one(&state.db).await.unwrap().unwrap();
    assert!(
        row.installed_display_version.is_none(),
        "miss must overwrite stale display with None"
    );
}

#[tokio::test]
async fn handle_version_check_results_keeps_distinct_display_per_host_for_same_skill() {
    // Two hosts, same package_identifier, different installed SHAs.
    let (state, svc, hsi_id_a, hsi_id_b) =
        setup_two_hosts_same_skill_different_shas("test_enricher_plugin").await;
    let payload = /* two results, same software_item_id different host_software_item_ids */ ;
    handle_version_check_results(&state, svc.id, &payload).await;
    let row_a = host_software_item::Entity::find_by_id(hsi_id_a).one(&state.db).await.unwrap().unwrap();
    let row_b = host_software_item::Entity::find_by_id(hsi_id_b).one(&state.db).await.unwrap().unwrap();
    assert_eq!(row_a.installed_display_version.as_deref(), Some("date_for_host_a_sha"));
    assert_eq!(row_b.installed_display_version.as_deref(), Some("date_for_host_b_sha"));
}
```

The four `setup_*` helpers register a stub plugin descriptor at test time via the existing test-only descriptor-registration path (search
for how `dashboard-icons` test-only descriptors register — there is a `test_support` module under
`crates/plugins/infrastructure/registry/src/`). Use that to inject a `test_enricher_plugin` descriptor with:
`extra_capabilities: [EnrichInstalledVersion]`, `installed_version_enricher_create` factory that returns an inline enricher matching the
test's expected behavior.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p uptrakit-web-api handle_version_check_results_invokes_enricher` Expected: FAIL — dispatch logic not yet present.

- [ ] **Step 3: Add the dispatch block**

In `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`, locate
`pub(super) async fn handle_version_check_results(state: &AppState, service_id: Uuid, payload: &VersionCheckResultsPayload)` (around line
937). Before the per-result `apply_version_update_to_db` loop, insert:

**Prerequisite — tenant resolution + Service→Tenant lookup.** `AppState` does not expose a `tenant_db_for_service` helper. The existing
pattern is to load the service model, read its `tenant_id`, and construct `TenantDb::new(state.db.clone(), tenant_id)`. The nearest
precedent is the `service_tenant_id` field threaded through `messages.rs:57,123` — copy that pattern. If a helper is missing, add a small
private fn at the bottom of the file:

```rust
async fn resolve_tenant_db_for_service(
    state: &AppState,
    service_id: uuid::Uuid,
) -> Option<uptrakit_web_api_auth::TenantDb> {
    use sea_orm::EntityTrait;
    let service = uptrakit_shared_db::entity::service::Entity::find_by_id(service_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()?;
    Some(uptrakit_web_api_auth::TenantDb::new(state.db.clone(), service.tenant_id))
}
```

(Naming + visibility: match whatever pattern other helpers in this file use. If `service_tenant_id` is already in scope from an earlier
resolution step in `handle_version_check_results`, reuse it instead of re-querying.)

Add the dispatch block before the per-result `apply_version_update_to_db` loop in
`pub(super) async fn handle_version_check_results(state: &AppState, service_id: Uuid, payload: &VersionCheckResultsPayload)` (around line
937):

```rust
    // ── Installed-version enrichment dispatch ────────────────────────────
    // Resolves a `host_software_item_id → DisplayOverride` map for every
    // result whose plugin_type declares `EnrichInstalledVersion`. Items not
    // covered by an enricher fall through to `DisplayOverride::UseAgentValue`
    // at the write site.
    //
    // Web-api stays plugin-agnostic — purely typed registry lookup. ADR-0018.
    let mut enriched: std::collections::HashMap<uuid::Uuid, DisplayOverride> =
        std::collections::HashMap::new();
    {
        use std::collections::HashMap;
        use std::sync::Arc;
        use uptrakit_plugin_infrastructure_core::{
            InstalledVersionEnrichmentContext, InstalledVersionItem, PluginCapability,
        };
        use uptrakit_plugin_infrastructure_registry::{
            GlobalProviderLookup, construct_host_runtime, get_descriptor,
        };

        // 1. Source (plugin_type, package_identifier) per host_software_item_id for role detect_version.
        let hsi_ids: Vec<uuid::Uuid> = payload
            .results
            .iter()
            .filter_map(|r| r.host_software_item_id)
            .collect();
        let assignments = if hsi_ids.is_empty() {
            HashMap::new()
        } else {
            let Some(tenant_db) = resolve_tenant_db_for_service(state, service_id).await else {
                tracing::warn!(%service_id, "enrichment: tenant resolution failed; skipping");
                HashMap::new()
            };
            match uptrakit_web_api_queries::queries::host_software_item_plugins::plugin_types_for_role(
                &tenant_db,
                &hsi_ids,
                "detect_version",
            ).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "enrichment: plugin_type lookup failed; skipping");
                    HashMap::new()
                }
            }
        };

        // 2. Group items by plugin_type. Items inherit `package_identifier` from
        //    the DB-side assignment (NOT from the wire payload, which doesn't carry it).
        #[derive(Default)]
        struct Group {
            hsi_ids: Vec<uuid::Uuid>,
            items: Vec<InstalledVersionItem>,
        }
        let mut by_plugin: HashMap<String, Group> = HashMap::new();
        for r in &payload.results {
            let Some(hsi_id) = r.host_software_item_id else { continue };
            let Some(assignment) = assignments.get(&hsi_id) else { continue };
            let g = by_plugin.entry(assignment.plugin_type.clone()).or_default();
            g.hsi_ids.push(hsi_id);
            g.items.push(InstalledVersionItem {
                package_identifier: assignment.package_identifier.clone(),
                installed_version: r.installed_version.clone(),
            });
        }

        // 3. For each plugin_type, look up descriptor + capability + slot.
        for (pt, group) in by_plugin {
            let Some(desc) = get_descriptor(&pt) else { continue };
            if !desc.capabilities.contains(&PluginCapability::EnrichInstalledVersion) {
                continue;
            }
            let Some(slot) = desc.roles.installed_version_enricher.as_ref() else { continue };

            // 4. Build the context using the empty()+builder pattern. Single positive
            //    #[cfg(feature = "catalog")] block — no `not(...)` cfg, satisfying the
            //    "feature flags must be additive-only" binding rule.
            //    Non-catalog builds (e.g. standalone-scheduler) leave the lookup as None
            //    and the enricher falls back to `display_version = None`.
            #[allow(unused_mut)]
            let mut ctx = InstalledVersionEnrichmentContext::empty();
            #[cfg(feature = "catalog")]
            {
                let lookup: Arc<dyn GlobalProviderLookup> =
                    state.plugin.global_providers.clone() as Arc<dyn GlobalProviderLookup>;
                ctx = ctx.with_lookup(lookup);
            }

            let runtime = construct_host_runtime(
                Arc::new(uptrakit_command::NoopCommandExecutor),
                uptrakit_plugin_infrastructure_core::HostCapabilities::default(),
            );
            let merged_cfg = serde_json::json!({});
            let enricher = match (slot.create)(&merged_cfg, runtime, &ctx) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        plugin_type = %pt, error = %e, reason = "provider_error",
                        "enrichment: factory failed; collapsing group"
                    );
                    for hsi in &group.hsi_ids {
                        enriched.insert(*hsi, DisplayOverride::Override(None));
                    }
                    continue;
                }
            };
            let result = enricher.enrich_installed_versions(&group.items).await;
            let out = match result {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        plugin_type = %pt, error = %e, reason = "provider_error",
                        "enrichment: enricher returned Err; collapsing group"
                    );
                    for hsi in &group.hsi_ids {
                        enriched.insert(*hsi, DisplayOverride::Override(None));
                    }
                    continue;
                }
            };
            if out.len() != group.items.len() {
                tracing::warn!(
                    plugin_type = %pt, expected = group.items.len(), got = out.len(),
                    reason = "race_skipped",
                    "enrichment: length mismatch; collapsing group"
                );
                for hsi in &group.hsi_ids {
                    enriched.insert(*hsi, DisplayOverride::Override(None));
                }
                continue;
            }
            for (i, display) in out.into_iter().enumerate() {
                let hsi = group.hsi_ids[i];
                if display.installed_version_echo != group.items[i].installed_version {
                    tracing::warn!(
                        plugin_type = %pt, %hsi, reason = "race_skipped",
                        "enrichment: installed_version_echo mismatch"
                    );
                    enriched.insert(hsi, DisplayOverride::Override(None));
                    continue;
                }
                if display.package_identifier != group.items[i].package_identifier {
                    tracing::warn!(
                        plugin_type = %pt, %hsi, reason = "race_skipped",
                        "enrichment: package_identifier echo mismatch"
                    );
                    enriched.insert(hsi, DisplayOverride::Override(None));
                    continue;
                }
                enriched.insert(hsi, DisplayOverride::Override(display.display_version));
            }
        }
    }
```

Then in the per-result write loop, change every call site of `apply_version_update_to_db(&state.db, &result, matching_ids, now)` to:

```rust
let override_for_result = result
    .host_software_item_id
    .and_then(|hsi| enriched.get(&hsi).cloned())
    .unwrap_or(DisplayOverride::UseAgentValue);
apply_version_update_to_db(
    &state.db,
    &result,
    matching_ids,
    now,
    override_for_result,
).await;
```

Wrap the dispatch block in the existing tracing span (`handle_version_check_results` is already
`#[tracing::instrument(skip_all, fields(service_id, result_count = payload.results.len()))]` — verify and keep `skip_all`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p uptrakit-web-api handle_version_check_results` Expected: 4 new tests + all prior tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/routes/service_ws/handler/messages.rs
git commit -m "feat(web-api): typed-slot dispatch for InstalledVersionEnricher"
```

---

## Phase 6: Skills wire-up

### Task 12: Declare role + capability + factory on `SkillsPlugin`

**Files:**

- Modify: `crates/plugins/package-managers/skills/src/plugin.rs`

**Snapshot rules invoked:** `#[non_exhaustive]` already on `SkillsPlugin`; rootcause::Report errors; module-doc-comment housekeeping.

- [ ] **Step 1: Extend the `descriptor_capabilities` test**

In `crates/plugins/package-managers/skills/src/plugin.rs` (test added in the prior session at lines 152-156), add:

```rust
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::EnrichInstalledVersion)
        );
        assert!(
            DESCRIPTOR.roles.installed_version_enricher.is_some(),
            "Skills must register an InstalledVersionEnricher slot"
        );
        assert!(
            DESCRIPTOR
                .roles
                .installed_version_enricher
                .as_ref()
                .unwrap()
                .host_requirements
                .controller_only
        );
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p uptrakit-plugin-package-manager-skills descriptor_capabilities` Expected: FAIL — capability bit + slot not yet declared.

- [ ] **Step 3: Add the factory + extend `declare_plugin!`**

In `crates/plugins/package-managers/skills/src/plugin.rs`, add a factory function next to `create_release_fetcher_skills`:

```rust
pub(crate) fn create_installed_version_enricher_skills(
    config: &serde_json::Value,
    runtime: Arc<dyn HostRuntime>,
    ctx: &uptrakit_plugin_infrastructure_core::InstalledVersionEnrichmentContext,
) -> uptrakit_plugin_infrastructure_core::error::Result<
    Box<dyn uptrakit_plugin_infrastructure_core::InstalledVersionEnricher>,
> {
    let cfg: SkillsConfig = serde_json::from_value(config.clone()).map_err(|e| {
        report!(PluginError::Configuration(format!(
            "failed to parse skills config: {e}"
        )))
    })?;
    let provider = lookup_github_provider_from_enrichment_ctx(ctx);
    Ok(Box::new(SkillsPlugin {
        config: cfg,
        executor: runtime.executor(),
        provider,
    }))
}

fn lookup_github_provider_from_enrichment_ctx(
    ctx: &uptrakit_plugin_infrastructure_core::InstalledVersionEnrichmentContext,
) -> Option<Arc<dyn GitHubProviderClient>> {
    // Single positive `#[cfg(feature = "catalog")]` block; the implicit "else" is
    // the function returning the trailing `None` literal. Avoids `#[cfg(not(...))]`,
    // satisfying the additive-only feature-flag binding rule.
    let _ = ctx;
    #[cfg(feature = "catalog")]
    {
        if let Some(lookup) = ctx.global_provider_lookup.as_ref() {
            if let Some(handle) = lookup.lookup("github") {
                return Arc::downcast::<GitHubProviderHandle>(handle)
                    .ok()
                    .map(|h| h.client());
            }
        }
    }
    None
}
```

(The existing `lookup_github_provider_from_ctx` for `ReleaseFetchContext` predates the additive-only rule and uses
`#[cfg(not(feature = "catalog"))]` — do **not** mirror that pattern. The new helper above uses only positive cfg attributes.)

Extend the `declare_plugin!` block (currently lines 95-110) so it reads:

```rust
declare_plugin!(SkillsPlugin, SkillsConfig, "package_manager_skills", {
    display_name: "Agent Skills",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher { host_requirements: HostRequirements::CONTROLLER_ONLY },
        InstalledVersionEnricher { host_requirements: HostRequirements::CONTROLLER_ONLY },
        UpdateExecutor,
    ],
    extra_capabilities: [
        PluginCapability::ControllerSideFetchReleases,
        PluginCapability::EnrichInstalledVersion,
    ],
    release_fetcher_create: {
        create: create_release_fetcher_skills,
        host_requirements: HostRequirements::CONTROLLER_ONLY,
    },
    installed_version_enricher_create: {
        create: create_installed_version_enricher_skills,
        host_requirements: HostRequirements::CONTROLLER_ONLY,
    },
});
```

Update the module doc-comment at the top of the file:

```rust
//! Agent Skills package-manager plugin.
//!
//! Manages LLM-agent skills installed via the `skills` CLI (`npx skills@<version>`).
//! Each skill is identified by a `<source_url>#<skill_path>` composite key.
//!
//! Controller-side roles: `ReleaseFetcher` (latest commit-date display via GitHub Trees +
//! commits-by-path) and `InstalledVersionEnricher` (installed commit-date display via the
//! same primitive — keyed by tree-at-path SHA reported by the agent).
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p uptrakit-plugin-package-manager-skills descriptor_capabilities` Expected: PASS.

- [ ] **Step 5: Verify full Skills suite**

Run: `cargo test -p uptrakit-plugin-package-manager-skills` Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/package-managers/skills/src/plugin.rs
git commit -m "feat(plugins/skills): register InstalledVersionEnricher role"
```

---

## Phase 7: Frontend short-SHA fallback

### Task 13: `formatVersion` renders 40-hex SHA as `<first 12>…`

**Files:**

- Modify: `frontend/src/lib/utils.ts`
- Modify: `frontend/src/lib/utils.test.ts` (already exists at this path).

**Snapshot rules invoked:** Frontend rules — TypeScript required, prettier formatting, vitest unit-test convention.

- [ ] **Step 1: Write the failing test**

Extend the existing `frontend/src/lib/utils.test.ts` file with these vitest cases:

```typescript
import { describe, it, expect } from "vitest";
import { formatVersion } from "./utils";

describe("formatVersion", () => {
  it("shortens 40-hex git SHA to first 12 chars + ellipsis", () => {
    expect(formatVersion("f260c775073816860fef8a37c032ac77e2ff5821")).toBe("f260c7750738…");
  });
  it("passes already-formatted dates through to locale rendering", () => {
    // existing path: ISO 8601 → browser-locale. Snapshot or pattern-check only.
    const out = formatVersion("2026-06-11T01:15:00Z");
    expect(out).not.toBe("2026-06-11T01:15:00Z");
    expect(out.length).toBeGreaterThan(0);
  });
  it("passes short or non-hex strings through unchanged", () => {
    expect(formatVersion("1.2.3")).toBe("1.2.3");
    expect(formatVersion("f260c775")).toBe("f260c775"); // < 40 chars
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && npx vitest run src/lib/utils.test.ts` Expected: the SHA case FAILS — current `formatVersion` passes the 40-char SHA
through unchanged.

- [ ] **Step 3: Add the branch in `formatVersion`**

In `frontend/src/lib/utils.ts`, around the existing `sha256:` shortener (search for `sha256:`), add a sibling branch:

```typescript
// Git tree SHA (40-char lowercase hex) → shorten like the sha256: digest path.
if (/^[0-9a-f]{40}$/i.test(value)) {
  return `${value.slice(0, 12)}…`;
}
```

(Use `…` for the ellipsis — matches the existing `sha256:` shortener's escape style. Do NOT use the literal `…`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && npx vitest run src/lib/utils.test.ts` Expected: 3 PASS.

- [ ] **Step 5: Run frontend quality gates**

Run: `cd frontend && npm run lint && npm run format:check && npm run check` Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/utils.ts frontend/src/lib/utils.test.ts
git commit -m "feat(frontend): short-SHA fallback in formatVersion"
```

---

## Phase 8: Documentation

### Task 14: ADR-0021 — Installed Version Enrichment Role

**Files:**

- Create: `docs/adr/0021-installed-version-enrichment-role.md`

**Snapshot rules invoked:** ADR documents architectural decisions (referenced from snapshot ADR index); markdownlint-clean (workspace rule).

- [ ] **Step 1: Draft the ADR**

Create `docs/adr/0021-installed-version-enrichment-role.md` following the pattern in `docs/adr/0015-release-fetcher-context.md` (sibling,
same shape):

```markdown
# ADR-0021: Installed Version Enrichment Role

Date: 2026-06-17

## Status

Accepted.

## Context

Some plugins represent installed software versions as opaque identifiers — for example, the LLM Skills plugin uses a git tree SHA
(`skillFolderHash` from `~/.agents/.skill-lock.json`) as its installed version. Rendering the raw identifier in the Dashboard produces
unreadable rows next to plugins like Docker that already show human-friendly labels (commit date, semver tag).

Translating the identifier to a display string sometimes requires upstream metadata (e.g. a GitHub commits-by-path API call) that only the
controller can reach — the agent has no global GitHub provider. The existing `ReleaseFetcher` role (controller-only, ADR-0015 context
injection) covers the _latest_ side. The _installed_ side previously had no controller-side hook.

A naïve fix would be to inline a `match plugin_type` in `handle_version_check_results` and call a Skills-specific helper. That violates
ADR-0018 (typed plugin extension boundary): the web-api would gain plugin-type knowledge.

## Decision

Add a new typed plugin role `InstalledVersionEnricher`:

- Controller-only async trait with one method,
  `enrich_installed_versions(items: &[InstalledVersionItem]) -> Result<Vec<InstalledVersionDisplay>>`.
- Returned `Vec` is the same length and order as `items`; dispatcher zips by index, not by `package_identifier`, so two host_software_item
  rows sharing a package_identifier with different SHAs stay distinct.
- `InstalledVersionDisplay` carries `installed_version_echo` for sanity-check; mismatch logs `race_skipped` and writes `None`.
- Bespoke `InstalledVersionEnricherSlot` mirroring `ReleaseFetcherSlot` (3-arg factory).
- New capability bit `PluginCapability::EnrichInstalledVersion` gates dispatch.
- `InstalledVersionEnrichmentContext` mirrors `ReleaseFetchContext` from ADR-0015 and carries the optional `GlobalProviderLookup`.

`handle_version_check_results` dispatches purely via the typed registry:

- Resolve `host_software_item_id → plugin_type` via `host_software_item_plugin` join (tenant-scoped through `software_item`).
- For each plugin_type with the capability + slot, instantiate the enricher and call once with the per-group batch.
- Verify length + echo per item; fold display values into a `HashMap<host_software_item_id, Option<String>>` side-channel.
- Thread the override into the existing single `update_many` that already writes `InstalledVersion` and `InstalledDisplayVersion` atomically
  (`messages.rs:758-789`).

## Write semantics

`installed_display_version` is always overwritten alongside `installed_version` in the same UPDATE. Enricher miss / throttle / out-of-window
→ write `None`. Prior display values are never preserved across a SHA change — they would map to the wrong SHA.

## Observability

`warn!` logs distinguish four reason tags:

- `provider_error` — Throttled, AuthFailed, transient network, non-404 HTTP.
- `upstream_gone` — strictly 404 from `commits?path=…`.
- `out_of_window` — walk completed but SHA never appeared (subsumes force-push past, path rename, fork-merge unreachability, SHA older than
  90 commits).
- `race_skipped` — length / echo / identifier mismatch from the enricher.

## Operational note: 90-commit ceiling

The Skills enricher caps the commits-by-path walk at 90 to bound API cost. If `out_of_window` becomes a common reason tag in production,
raise the cap or pair it with a persistent per-`(owner, repo, path)` SHA→date cache.

## Alternatives considered

- **Plugin-type switch in web-api** — rejected: violates ADR-0018.
- **Agent-side lockfile extension** — rejected: couples the agent to upstream CLI internals; doesn't cover existing installs without
  controller-side backfill anyway.
- **Per-plugin-type ad-hoc roles** — rejected: same generic shape works for any plugin that surfaces opaque installed identifiers.
- **Storing `sha_history: Vec<{sha, committed_at}>` inside `latest_release_metadata`** — rejected: replaces the typed slot boundary with a
  stringly-typed JSON key; future plugin authors would couple via the blob shape rather than via the role trait.

## Related

- ADR-0015 (Release-fetcher context) — sibling pattern for the latest side.
- ADR-0018 (Typed plugin extension boundary) — invariant preserved.
- Spec: `docs/superpowers/specs/2026-06-17-skills-version-display-design.md`.
```

- [ ] **Step 2: Lint the ADR**

Run: `markdownlint --config .markdownlint.json docs/adr/0021-installed-version-enrichment-role.md` Expected: clean. If long lines fire
MD013, run `npx prettier --prose-wrap always --print-width 140 --write docs/adr/0021-installed-version-enrichment-role.md`.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0021-installed-version-enrichment-role.md
git commit -m "docs(adr): ADR-0021 installed version enrichment role"
```

---

### Task 15: `docs/development/coding-standards.md` subsection

**Files:**

- Modify: `docs/development/coding-standards.md`

**Snapshot rules invoked:** markdownlint-clean.

- [ ] **Step 1: Identify the right anchor**

Open `docs/development/coding-standards.md`. Search for the plugin section (look for `## Plugin` or `### Plugin roles`). The new subsection
sits next to the `ReleaseFetcher` discussion.

- [ ] **Step 2: Add the subsection**

Insert (near the `ReleaseFetcher` subsection):

````markdown
### InstalledVersionEnricher (controller-only)

When an agent reports an opaque `installed_version` (e.g. a git tree SHA), the controller can enrich it with a human-friendly
`installed_display_version` through the `InstalledVersionEnricher` trait. The role is controller-only and its factory receives an
`InstalledVersionEnrichmentContext` (mirror of `ReleaseFetchContext` from ADR-0015), so the enricher can reach the global GitHub provider or
similar shared resources.

Declare it like any other role:

```rust
roles: [
    // ...
    InstalledVersionEnricher { host_requirements: HostRequirements::CONTROLLER_ONLY },
],
extra_capabilities: [PluginCapability::EnrichInstalledVersion],
installed_version_enricher_create: {
    create: create_installed_version_enricher_my_plugin,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
},
```

Web-api dispatches via descriptor slot lookup; no plugin-type strings appear in the handler (ADR-0018). The trait returns a `Vec` the same
length and order as the input; dispatcher zips by index. See ADR-0021 for the full contract, observability tags, and the 90-commit
operational ceiling.
````

- [ ] **Step 3: Lint**

Run: `markdownlint --config .markdownlint.json docs/development/coding-standards.md` Expected: clean. Run prettier if MD013 fires.

- [ ] **Step 4: Commit**

```bash
git add docs/development/coding-standards.md
git commit -m "docs(coding-standards): document InstalledVersionEnricher role"
```

---

### Task 16: Verify all quality gates pass end-to-end

**Files:**

- None (verification only).

- [ ] **Step 1: Run Rust quality gates**

Run each, expecting clean:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

- [ ] **Step 2: Run markdown + CI scripts**

```bash
markdownlint --config .markdownlint.json '**/*.md'
python3 ci/check_plugin_semantic_boundary.py
bash ci/verify_no_security_audit.sh
bash ci/verify_typed_audit_actions.sh
bash ci/verify_handler_state_contract.sh
python3 ci/verify_db_access_policy.py
```

Expected: all pass.

**Conditional integration test** — Task 9 adds a new tenant-scoped DB query. Per quality-gates.md:93-130, this triggers the database
integration suite (requires Docker PG):

```bash
cargo test -p uptrakit-integration-tests --test database -- --ignored
```

Expected: clean.

- [ ] **Step 3: Run frontend gates**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build && cd ..
```

Expected: clean.

- [ ] **Step 4: Sentrux architecture pass**

Invoke `mcp__plugin_sentrux_sentrux__rescan` followed by `mcp__plugin_sentrux_sentrux__health` via the MCP tools. Confirm no dimension
regressed.

- [ ] **Step 5: Manual end-to-end check**

Rebuild and restart the controller:

```bash
cargo build -p uptrakit-controller --release
# stop existing controller, restart with the new binary
```

Then in the Dashboard:

1. Trigger a scheduler `fetch_releases` cycle (wait or kick via the task runner).
2. Inspect controller logs — confirm `package_manager_skills` no longer emits the prior `"GitHub provider unavailable"` error and instead
   emits `"controller-side fetch_releases succeeded"` for Skills items.
3. Open the Software Items list. A Skills row that was previously `f260c775073816860fef8a37c032ac77e2ff5821` must now render as
   `"11 Jun 2026 at 01:15"` style (or similar, per browser locale) in both installed and latest columns when installed == latest.
4. Edit `~/.agents/.skill-lock.json` to roll one skill's `skillFolderHash` back to an older value from the same path's commit history
   (within the last 90 commits). Trigger detect_version. Confirm the installed column shows an older date and the update arrow now points to
   a strictly newer date. Restore the lockfile after.
5. Throttle simulation: temporarily revoke the GitHub token. Trigger a cycle. Confirm `warn!` is logged with `reason = "provider_error"`,
   both display columns are cleared to `NULL`, and the row falls back to short-SHA display (`f260c7750738…`). Re-enable the token.

---

## Self-Review

After completing all tasks, run through this checklist:

**1. Spec coverage:**

| Spec section                     | Task                                 |
| -------------------------------- | ------------------------------------ |
| §1 Role trait + structs          | Task 4                               |
| §1 Slot + RoleCreators           | Task 5                               |
| §2 Capability bit                | Task 3                               |
| §3 Context                       | Task 4                               |
| §4 Macro extension               | Task 6                               |
| §5 Provider primitive            | Task 1, 2                            |
| §6 Skills latest (`batch_fetch`) | Task 7                               |
| §6 Skills enricher impl          | Task 8                               |
| §7 Web-api dispatch              | Task 10, 11                          |
| §7 Plugin-type lookup query      | Task 9                               |
| §8 Write semantics               | Task 10 (override + atomic update)   |
| §9 Frontend short-SHA fallback   | Task 13                              |
| §10 Observability reason tags    | Task 11 (warn! reason fields)        |
| §11 TDD slice ordering           | Tasks 1→13 follow spec slice 1→8     |
| §12 ADR-0021                     | Task 14                              |
| §12 coding-standards doc         | Task 15                              |
| §12 Skills plugin module doc     | Task 12                              |
| §12 CONTEXT.md (no further work) | Pre-existing — added during grilling |
| §Verification                    | Task 16                              |

All spec sections covered.

**2. Placeholder scan:** No "TBD" / "TODO" / "implement later" in tasks. Every code step shows the actual code. Every test step shows the
actual assertions.

**3. Type consistency:** `InstalledVersionItem`, `InstalledVersionDisplay`, `InstalledVersionEnricher`, `InstalledVersionEnrichmentContext`,
`InstalledVersionEnricherSlot`, `CreateInstalledVersionEnricherFn`, `EnrichInstalledVersion`, `TreeCommit`, `DISPLAY_FMT`,
`list_recent_commit_dates_for_path`, `installed_version_enricher_create`, `installed_display_version_override` — used consistently across
tasks. `PACKAGE_MANAGER_SKILLS` reused. `host_software_item_id: Uuid` used as the side-channel key consistently.

**4. Idiom audit (post-draft):** No task suppresses lints. No task fights the framework — every extension follows the precedent of an
existing role (`ReleaseFetcher` for both latest and the new enricher). Date formatting uses `format_description!` (NOT `Rfc3339`) — flagged
explicitly in spec and Task 7. Tenant isolation uses `find_via_tenant_join` per binding rule (Task 9). Single-UPDATE atomicity preserved
(Task 10). No new external dependencies introduced. All `match` arms over `#[non_exhaustive]` enums explicit per snapshot rule.

**5. Dependency version audit:** Zero new external crates. All used crates (`time`, `async_trait`, `tokio`, `serde`, `serde_json`,
`rootcause`, `tracing`, `sea-orm`, `parking_lot`, `uptrakit-*` workspace crates) are already in `[workspace.dependencies]` of the root
`Cargo.toml` per AGENTS.md:336-354. No version-pinning task needed.

**6. Doc deliverables:** ADR-0021 (Task 14), `docs/development/coding-standards.md` (Task 15), Skills module-doc-comment (Task 12, step 3),
CONTEXT.md (pre-existing — covered during grilling). All four enumerated.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-17-skills-version-display.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
