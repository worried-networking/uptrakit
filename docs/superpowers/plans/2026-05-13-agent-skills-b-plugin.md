# Agent Skills Plugin — Plan B: Plugin Crate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `uptrakit-plugin-package-manager-skills` — the crate that discovers, detects,
fetches releases for, and updates globally-installed LLM-agent Skills via `npx skills@latest`.

**Architecture:** One struct (`SkillsPlugin`) implements all four role traits. Agent-side roles
(Discoverer, VersionDetector, UpdateExecutor) read `~/.agents/.skill-lock.json` via
`CommandExecutor::execute_quiet`. Controller-side ReleaseFetcher is wired through a custom
`create_release_fetcher_skills` factory that extracts the GitHub provider from
`ReleaseFetchContext` (introduced in Plan A). The `declare_plugin!` macro wires agent-side
roles; the custom factory is registered via the new `release_fetcher_create` macro field.

**Tech Stack:** Rust 2021 · `uptrakit-plugin-infrastructure-core` · `uptrakit-global-github-provider`
(for `GitHubProviderHandle`, `PACKAGE_MANAGER_SKILLS` consumer constant) · `uptrakit-shared-macros`
(`impl_report_conversion!`) · `url` crate (GitHub URL parsing) · `rootcause` · `async-trait`
· `serde` / `serde_json` · `thiserror` · `tracing`

**Prerequisite:** Plan A must be fully implemented and compiled before starting Plan B.

---

## File Map

| File                                                      | Action | Responsibility                                                                            |
| --------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------- |
| `crates/shared/types/src/plugin_type_id.rs`               | Modify | Add `PACKAGE_MANAGER_SKILLS` constant and `ALL` entry                                     |
| `Cargo.toml` (workspace root)                             | Modify | Add `uptrakit-plugin-package-manager-skills` workspace dep                                |
| `crates/plugins/package-managers/skills/Cargo.toml`       | Create | Crate manifest                                                                            |
| `crates/plugins/package-managers/skills/CODEREVIEW.md`    | Create | Empty review scaffold                                                                     |
| `crates/plugins/package-managers/skills/src/error.rs`     | Create | `SkillsError` enum + `Result` alias + `impl_report_conversion!`                           |
| `crates/plugins/package-managers/skills/src/config.rs`    | Create | `SkillsConfig` (empty unit-like struct)                                                   |
| `crates/plugins/package-managers/skills/src/lock.rs`      | Create | `SkillLockEntry`, `parse_skill_lock`, `encode_skill_identifier`, `parse_skill_identifier` |
| `crates/plugins/package-managers/skills/src/plugin.rs`    | Create | `SkillsPlugin` struct, `new`, `create_release_fetcher_skills` factory, `declare_plugin!`  |
| `crates/plugins/package-managers/skills/src/discovery.rs` | Create | `Discoverer` + `detect_host_compatibility` impls                                          |
| `crates/plugins/package-managers/skills/src/detection.rs` | Create | `VersionDetector` + `batch_detect` impls                                                  |
| `crates/plugins/package-managers/skills/src/releases.rs`  | Create | `ReleaseFetcher` impl with custom `batch_fetch`                                           |
| `crates/plugins/package-managers/skills/src/update.rs`    | Create | `UpdateExecutor` impl                                                                     |
| `crates/plugins/package-managers/skills/src/lib.rs`       | Create | Module declarations + re-exports                                                          |
| `crates/plugins/infrastructure/registry/Cargo.toml`       | Modify | Add skills crate as dependency                                                            |
| `crates/plugins/infrastructure/registry/src/registry.rs`  | Modify | Register in `all_descriptors`, `is_package_manager_plugin`, test arrays                   |
| `docs/adr/0015-release-fetcher-context.md`                | Create | ADR recording the `ReleaseFetchContext` factory-parameter extension                       |
| `docs/development/plugin-guidelines.md`                   | Modify | Add "Consuming global providers in ReleaseFetcher" section                                |
| `docs/end-user/skills-plugin.md`                          | Create | Operator guide                                                                            |

---

### Task 1: Add `PACKAGE_MANAGER_SKILLS` to `plugin_ids`

**Files:**

- Modify: `crates/shared/types/src/plugin_type_id.rs`

- [ ] **Step 1: Write a failing test for the new constant**

  In the `#[cfg(test)]` block near the bottom of `plugin_type_id.rs`, add before the
  `all_constants_count` test:

  ```rust
  #[test]
  fn package_manager_skills_constant_is_correct() {
      assert_eq!(
          plugin_ids::PACKAGE_MANAGER_SKILLS.as_str(),
          "package_manager_skills"
      );
  }
  ```

- [ ] **Step 2: Run the test to confirm it fails**

  ```bash
  cargo test -p uptrakit-shared-types package_manager_skills_constant -- --nocapture 2>&1 | tail -10
  ```

  Expected: compile error — `PACKAGE_MANAGER_SKILLS` does not exist.

- [ ] **Step 3: Add the constant and update `ALL`**

  In `crates/shared/types/src/plugin_type_id.rs`, inside `pub mod plugin_ids { ... }`, after the
  `PACKAGE_MANAGER_ROUTEROS` constant:

  ```rust
  pub const PACKAGE_MANAGER_SKILLS: PluginTypeId =
      PluginTypeId::from_static("package_manager_skills");
  ```

  Add `PACKAGE_MANAGER_SKILLS` to the `ALL` array after `PACKAGE_MANAGER_ROUTEROS`:

  ```rust
  pub const ALL: &[PluginTypeId] = &[
      RELEASES_GITHUB,
      RELEASES_GITLAB,
      RELEASES_FORGEJO,
      RELEASES_DOCKER,
      DISCOVERY_PROXMOX_HELPER_SCRIPTS,
      DISCOVERY_UPTRAKIT_SELF_UPDATE,
      PACKAGE_MANAGER_APT,
      PACKAGE_MANAGER_HOMEBREW,
      PACKAGE_MANAGER_DNF,
      PACKAGE_MANAGER_NPM,
      PACKAGE_MANAGER_MAS,
      PACKAGE_MANAGER_PACMAN,
      PACKAGE_MANAGER_PKG,
      PACKAGE_MANAGER_APK,
      PACKAGE_MANAGER_SNAP,
      PACKAGE_MANAGER_CARGO,
      PACKAGE_MANAGER_ROUTEROS,
      PACKAGE_MANAGER_SKILLS,
      GENERIC_SHELL,
      HOOK_SHELL,
      HOOK_SYSTEMD,
      INFRASTRUCTURE_PROXMOX,
      WEBHOOK,
      TELEGRAM,
      EMAIL,
      ENHANCEMENT_DASHBOARD_ICONS,
  ];
  ```

- [ ] **Step 4: Update the count assertion in `all_constants_count`**

  Find:

  ```rust
  assert_eq!(plugin_ids::ALL.len(), 25);
  ```

  Change to:

  ```rust
  assert_eq!(plugin_ids::ALL.len(), 26);
  ```

- [ ] **Step 5: Run tests**

  ```bash
  cargo test -p uptrakit-shared-types --all-features 2>&1 | tail -10
  ```

  Expected: all pass, including the new `package_manager_skills_constant_is_correct` test.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/shared/types/src/plugin_type_id.rs
  git commit -m "feat(shared-types): add PACKAGE_MANAGER_SKILLS plugin type id"
  ```

---

### Task 2: Crate skeleton and workspace registration

**Files:**

- Create: `crates/plugins/package-managers/skills/Cargo.toml`
- Create: `crates/plugins/package-managers/skills/CODEREVIEW.md`
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/plugins/infrastructure/registry/Cargo.toml`

- [ ] **Step 1: Create the crate `Cargo.toml`**

  Create `crates/plugins/package-managers/skills/Cargo.toml`:

  ```toml
  [package]
  name = "uptrakit-plugin-package-manager-skills"
  description = "Uptrakit package-manager plugin for globally-installed LLM-agent Skills"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"

  [dependencies]
  uptrakit-plugin-infrastructure-core = { workspace = true }
  uptrakit-global-github-provider = { workspace = true }
  uptrakit-shared-macros = { workspace = true }
  rootcause = { workspace = true }
  thiserror = { workspace = true }
  async-trait = { workspace = true }
  serde = { workspace = true }
  serde_json = { workspace = true }
  tracing = { workspace = true }
  url = { workspace = true }

  [dev-dependencies]
  uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }
  tokio = { workspace = true, features = ["macros", "rt"] }

  [lints]
  workspace = true
  ```

- [ ] **Step 2: Create the CODEREVIEW.md skeleton**

  Create `crates/plugins/package-managers/skills/CODEREVIEW.md`:

  ```markdown
  # Code Review — uptrakit-plugin-package-manager-skills

  <!-- TODO: populate at first code review -->
  ```

- [ ] **Step 3: Add the workspace dependency**

  In the root `Cargo.toml`, after the `uptrakit-plugin-package-manager-npm` line in
  `[workspace.dependencies]`:

  ```toml
  uptrakit-plugin-package-manager-skills = { path = "crates/plugins/package-managers/skills", version = "0.0.1" }
  ```

  (The workspace `members` glob `"crates/plugins/*/*"` already covers this crate — no change
  needed there.)

- [ ] **Step 4: Add to registry `Cargo.toml`**

  In `crates/plugins/infrastructure/registry/Cargo.toml`, add after the npm line in
  `[dependencies]`:

  ```toml
  uptrakit-plugin-package-manager-skills = { workspace = true }
  ```

- [ ] **Step 5: Create a minimal `src/lib.rs` so the crate compiles**

  Create `crates/plugins/package-managers/skills/src/lib.rs`:

  ```rust
  // placeholder — modules added in subsequent tasks
  ```

- [ ] **Step 6: Compile-check**

  ```bash
  cargo check -p uptrakit-plugin-package-manager-skills --all-features 2>&1 | tail -10
  ```

  Expected: no errors (empty lib).

- [ ] **Step 7: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/Cargo.toml \
    crates/plugins/package-managers/skills/CODEREVIEW.md \
    crates/plugins/package-managers/skills/src/lib.rs \
    Cargo.toml \
    crates/plugins/infrastructure/registry/Cargo.toml
  git commit -m "feat(skills): add crate skeleton and workspace registration"
  ```

---

### Task 3: `error.rs` and `config.rs`

**Files:**

- Create: `crates/plugins/package-managers/skills/src/error.rs`
- Create: `crates/plugins/package-managers/skills/src/config.rs`

- [ ] **Step 1: Create `error.rs`**

  ```rust
  use rootcause::prelude::*;
  use thiserror::Error;
  use uptrakit_plugin_infrastructure_core::PluginError;
  use uptrakit_shared_macros::impl_report_conversion;

  /// Errors specific to the Agent Skills plugin.
  #[derive(Debug, Error)]
  pub(crate) enum SkillsError {
      #[error("lock file missing or command failed")]
      LockFileMissing,

      #[error("lock file malformed: {0}")]
      LockFileMalformed(String),

      #[error("lock entry not found: {0}")]
      LockEntryNotFound(String),

      #[error("invalid identifier: {0}")]
      InvalidIdentifier(String),

      #[error("unsupported source type: {0}")]
      UnsupportedSource(String),

      #[error("GitHub provider unavailable: {0}")]
      ProviderUnavailable(String),

      #[error("GitHub provider error: {0}")]
      ProviderError(String),

      #[error("command failed with exit code {0}")]
      CommandFailed(i32),

      #[error("configuration error: {0}")]
      Configuration(String),

      #[error("plugin error: {0}")]
      Plugin(String),
  }

  /// Result type alias for the Skills plugin.
  pub(crate) type Result<T> = std::result::Result<T, Report<SkillsError>>;

  impl_report_conversion!(SkillsError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
  impl_report_conversion!(PluginError => SkillsError, |e| SkillsError::Plugin(e.to_string()));
  ```

- [ ] **Step 2: Create `config.rs`**

  ```rust
  use serde::{Deserialize, Serialize};
  use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

  use crate::lock::parse_skill_identifier;

  /// Configuration for the Agent Skills package-manager plugin.
  ///
  /// Empty in v1 — no user-facing fields. GitHub Provider credentials are
  /// managed in Settings → GitHub Provider; no per-plugin auth surface.
  #[derive(Debug, Default, Clone, Serialize, Deserialize)]
  pub struct SkillsConfig {}

  impl PluginConfig for SkillsConfig {
      // `validate`, `with_secrets_masked`, `restore_secrets_from`, and `form_schema`
      // all have correct default implementations for an empty config (no secrets,
      // no validation, empty schema). Only `validate_identifier` is overridden.
      fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
          parse_skill_identifier(value)
              .map(|_| ())
              .map_err(|e| PluginConfigValidationError::InvalidIdentifier(e.to_string()))
      }
  }
  ```

  **Note:** `config.rs` imports `lock::parse_skill_identifier` which does not exist yet (added in
  Task 4). The crate won't compile until Task 4 is done. This is expected — add `mod lock;`
  only after Task 4 is complete.

- [ ] **Step 3: Declare modules in `lib.rs`**

  Replace the placeholder content in `src/lib.rs`:

  ```rust
  pub(crate) mod config;
  pub(crate) mod error;
  ```

- [ ] **Step 4: Compile-check**

  ```bash
  cargo check -p uptrakit-plugin-package-manager-skills --all-features 2>&1 | tail -10
  ```

  Expected: no errors.

- [ ] **Step 5: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/error.rs \
    crates/plugins/package-managers/skills/src/config.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): add error and config modules"
  ```

---

### Task 4: `lock.rs` — lock file parsing and identifier encoding

**Files:**

- Create: `crates/plugins/package-managers/skills/src/lock.rs`

The `~/.agents/.skill-lock.json` v3 format is a JSON object keyed by Skill name:

```json
{
  "brainstorming": {
    "source": "obra/superpowers",
    "sourceUrl": "https://github.com/obra/superpowers",
    "sourceType": "github",
    "skillPath": "skills/brainstorming/SKILL.md",
    "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
    "installedAt": "2025-01-01T00:00:00Z",
    "updatedAt": "2025-01-02T00:00:00Z"
  }
}
```

- [ ] **Step 1: Write lock parsing tests first**

  Create `crates/plugins/package-managers/skills/src/lock.rs` with just the test module:

  ```rust
  pub struct SkillLockEntry {
      pub name: String,
      pub source_url: String,
      pub source_type: String,
      pub skill_path: String,
      pub skill_folder_hash: String,
  }

  pub fn parse_skill_lock(json: &str) -> crate::error::Result<Vec<SkillLockEntry>> {
      todo!()
  }

  pub fn encode_skill_identifier(source_url: &str, skill_path: &str) -> String {
      todo!()
  }

  pub fn parse_skill_identifier(id: &str) -> crate::error::Result<(url::Url, String)> {
      todo!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      const SAMPLE_LOCK: &str = r#"{
        "brainstorming": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/brainstorming/SKILL.md",
          "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
          "installedAt": "2025-01-01T00:00:00Z",
          "updatedAt": "2025-01-02T00:00:00Z"
        },
        "spec": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/spec/SKILL.md",
          "skillFolderHash": "cafecafecafecafecafecafecafecafecafecafe",
          "installedAt": "2025-01-01T00:00:00Z",
          "updatedAt": "2025-01-02T00:00:00Z"
        }
      }"#;

      const NON_GITHUB_LOCK: &str = r#"{
        "local-skill": {
          "source": "local/source",
          "sourceUrl": "https://gitlab.com/local/source",
          "sourceType": "gitlab",
          "skillPath": "skills/local-skill/SKILL.md",
          "skillFolderHash": "aabbccddaabbccddaabbccddaabbccddaabbccdd"
        }
      }"#;

      #[test]
      fn parse_valid_lock_returns_entries() {
          let entries = parse_skill_lock(SAMPLE_LOCK).expect("parse ok");
          assert_eq!(entries.len(), 2);
          let bs = entries.iter().find(|e| e.name == "brainstorming").expect("brainstorming");
          assert_eq!(bs.source_url, "https://github.com/obra/superpowers");
          assert_eq!(bs.source_type, "github");
          assert_eq!(bs.skill_path, "skills/brainstorming/SKILL.md");
          assert_eq!(bs.skill_folder_hash, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
      }

      #[test]
      fn parse_non_github_entry_still_included() {
          // parse_skill_lock itself does not filter; callers decide what to skip
          let entries = parse_skill_lock(NON_GITHUB_LOCK).expect("parse ok");
          assert_eq!(entries.len(), 1);
          assert_eq!(entries[0].source_type, "gitlab");
      }

      #[test]
      fn parse_malformed_json_fails() {
          let result = parse_skill_lock("not json");
          assert!(result.is_err());
      }

      #[test]
      fn parse_empty_object_returns_empty_vec() {
          let entries = parse_skill_lock("{}").expect("parse ok");
          assert!(entries.is_empty());
      }

      #[test]
      fn encode_roundtrips_through_parse() {
          let encoded = encode_skill_identifier(
              "https://github.com/obra/superpowers",
              "skills/brainstorming/SKILL.md",
          );
          let (url, path) = parse_skill_identifier(&encoded).expect("parse ok");
          assert_eq!(url.as_str(), "https://github.com/obra/superpowers");
          assert_eq!(path, "skills/brainstorming/SKILL.md");
      }

      #[test]
      fn parse_identifier_rejects_no_hash() {
          let result = parse_skill_identifier("https://github.com/owner/repo");
          assert!(result.is_err());
      }

      #[test]
      fn parse_identifier_rejects_path_traversal() {
          let result = parse_skill_identifier(
              "https://github.com/owner/repo#skills/../etc/passwd"
          );
          assert!(result.is_err());
      }

      #[test]
      fn parse_identifier_rejects_leading_slash_in_path() {
          let result = parse_skill_identifier(
              "https://github.com/owner/repo#/skills/brainstorming/SKILL.md"
          );
          assert!(result.is_err());
      }

      #[test]
      fn parse_identifier_rejects_empty_path() {
          let result = parse_skill_identifier("https://github.com/owner/repo#");
          assert!(result.is_err());
      }

      #[test]
      fn parse_identifier_rejects_total_length_over_1024() {
          let long_path = "a".repeat(1014);
          let id = format!("https://github.com/o/r#{long_path}");
          assert!(id.len() > 1024);
          let result = parse_skill_identifier(&id);
          assert!(result.is_err());
      }
  }
  ```

- [ ] **Step 2: Run the tests to confirm they all fail**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills lock -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile error (functions are `todo!()`).

- [ ] **Step 3: Implement the module**

  Replace the `todo!()` placeholders with the real implementations:

  ```rust
  use serde::Deserialize;
  use url::Url;

  use crate::error::{Result, SkillsError};

  #[derive(Debug, Deserialize)]
  #[serde(rename_all = "camelCase")]
  struct SkillEntryDto {
      source: String,
      source_url: String,
      source_type: String,
      skill_path: String,
      skill_folder_hash: String,
  }

  #[derive(Debug)]
  pub struct SkillLockEntry {
      /// Skill name — the JSON map key (e.g. `"brainstorming"`).
      pub name: String,
      /// Repository identifier used in the lockfile (e.g. `"obra/superpowers"`).
      #[expect(dead_code, reason = "stored from lock file for completeness; not read at runtime")]
      pub source: String,
      /// Full source URL (e.g. `"https://github.com/obra/superpowers"`).
      pub source_url: String,
      /// Source type string from the lock file (e.g. `"github"`).
      pub source_type: String,
      /// Relative path to the SKILL.md file (e.g. `"skills/brainstorming/SKILL.md"`).
      pub skill_path: String,
      /// Git tree SHA of the skill folder at install time.
      pub skill_folder_hash: String,
  }

  /// Parse the contents of `~/.agents/.skill-lock.json`.
  ///
  /// Returns a flat `Vec` with one `SkillLockEntry` per JSON map key. Callers are
  /// responsible for filtering by `source_type` — this function does not discriminate.
  ///
  /// Returns `Err` only for JSON structural problems, not for individual entry issues.
  pub fn parse_skill_lock(json: &str) -> Result<Vec<SkillLockEntry>> {
      let raw: std::collections::HashMap<String, serde_json::Value> =
          serde_json::from_str(json).map_err(|e| {
              rootcause::report!(SkillsError::LockFileMalformed(e.to_string()))
          })?;

      let mut entries = Vec::with_capacity(raw.len());
      for (name, value) in raw {
          let dto: SkillEntryDto = match serde_json::from_value(value) {
              Ok(d) => d,
              Err(e) => {
                  // Skip malformed individual entries rather than failing the whole file.
                  // A future skills CLI version may add new required fields; we do not
                  // want a single new entry to wipe out all existing skill tracking.
                  tracing::warn!(skill = %name, error = %e, "skipping malformed skill lock entry");
                  continue;
              }
          };
          entries.push(SkillLockEntry {
              name,
              source: dto.source,
              source_url: dto.source_url,
              source_type: dto.source_type,
              skill_path: dto.skill_path,
              skill_folder_hash: dto.skill_folder_hash,
          });
      }
      Ok(entries)
  }

  /// Encode a `(source_url, skill_path)` pair into the composite package identifier.
  ///
  /// The identifier is `"{source_url}#{skill_path}"`, e.g.
  /// `"https://github.com/obra/superpowers#skills/brainstorming/SKILL.md"`.
  pub fn encode_skill_identifier(source_url: &str, skill_path: &str) -> String {
      format!("{source_url}#{skill_path}")
  }

  /// Decode a composite skill identifier back into its URL and path components.
  ///
  /// Validates:
  /// - Total length ≤ 1024 bytes.
  /// - Exactly one `#` separator.
  /// - URL prefix is a valid `https://` or `http://` URL.
  /// - Path is non-empty, ≤ 512 bytes, no control chars, no leading `/`, no `..` segments.
  pub fn parse_skill_identifier(id: &str) -> Result<(Url, String)> {
      if id.len() > 1024 {
          return Err(rootcause::report!(SkillsError::InvalidIdentifier(
              "identifier exceeds 1024 bytes".to_string()
          )));
      }

      let hash_pos = id.find('#').ok_or_else(|| {
          rootcause::report!(SkillsError::InvalidIdentifier(
              "identifier must contain '#' separator between URL and skill path".to_string()
          ))
      })?;

      let url_part = &id[..hash_pos];
      let path_part = &id[hash_pos + 1..];

      if !url_part.starts_with("https://") && !url_part.starts_with("http://") {
          return Err(rootcause::report!(SkillsError::InvalidIdentifier(
              "identifier URL must start with https:// or http://".to_string()
          )));
      }

      let url = Url::parse(url_part).map_err(|e| {
          rootcause::report!(SkillsError::InvalidIdentifier(format!(
              "invalid URL in identifier: {e}"
          )))
      })?;

      if path_part.is_empty() || path_part.len() > 512 {
          return Err(rootcause::report!(SkillsError::InvalidIdentifier(
              "skill path must be 1–512 bytes".to_string()
          )));
      }
      if path_part.starts_with('/') {
          return Err(rootcause::report!(SkillsError::InvalidIdentifier(
              "skill path must not start with '/'".to_string()
          )));
      }
      for ch in path_part.chars() {
          if ch.is_control() {
              return Err(rootcause::report!(SkillsError::InvalidIdentifier(
                  "skill path must not contain control characters".to_string()
              )));
          }
      }
      for segment in path_part.split('/') {
          if segment == ".." {
              return Err(rootcause::report!(SkillsError::InvalidIdentifier(
                  "skill path must not contain '..' segments".to_string()
              )));
          }
      }

      Ok((url, path_part.to_string()))
  }
  ```

  Add `use rootcause::prelude::*;` if `rootcause::report!` complains about the macro path — or keep the full `rootcause::report!` path as shown.

- [ ] **Step 4: Add `lock` to `lib.rs`**

  In `src/lib.rs`, add:

  ```rust
  pub(crate) mod lock;
  ```

- [ ] **Step 5: Run tests**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills lock -- --nocapture 2>&1 | tail -20
  ```

  Expected: all tests pass.

- [ ] **Step 6: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/lock.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): add lock file parsing and skill identifier codec"
  ```

---

### Task 5: `plugin.rs` — `SkillsPlugin` struct and `declare_plugin!`

**Files:**

- Create: `crates/plugins/package-managers/skills/src/plugin.rs`

This file contains: the `SkillsPlugin` struct, `new` constructor, `create_release_fetcher_skills`
factory (the 3-arg function registered via `release_fetcher_create`), `validate_identifier`
(adapts `parse_skill_identifier` to `PluginConfigValidationError`), and the `declare_plugin!`
invocation.

- [ ] **Step 1: Write a failing descriptor test**

  Create `crates/plugins/package-managers/skills/src/plugin.rs` with a stub struct and the
  test module:

  ```rust
  use std::sync::Arc;
  use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
  use uptrakit_plugin_infrastructure_core::{
      ConfigModel, HostRequirements, HostRuntime, PluginFamily, declare_plugin,
  };
  use uptrakit_global_github_provider::GitHubProviderClient;

  use crate::config::SkillsConfig;

  #[non_exhaustive]
  pub struct SkillsPlugin {
      pub(crate) executor: Arc<dyn CommandExecutor>,
      pub(crate) provider: Option<Arc<dyn GitHubProviderClient>>,
  }

  impl SkillsPlugin {
      pub fn new(_config: SkillsConfig, runtime: Arc<dyn HostRuntime>) -> Result<Self, String> {
          Ok(Self {
              executor: runtime.executor(),
              provider: None,
          })
      }
  }

  pub fn validate_identifier(
      _value: &str,
  ) -> std::result::Result<(), uptrakit_plugin_infrastructure_core::PluginConfigValidationError> {
      todo!()
  }

  pub(crate) fn create_release_fetcher_skills(
      _config: &serde_json::Value,
      _runtime: Arc<dyn HostRuntime>,
      _ctx: &uptrakit_plugin_infrastructure_core::roles::ReleaseFetchContext,
  ) -> uptrakit_plugin_infrastructure_core::error::Result<
      Box<dyn uptrakit_plugin_infrastructure_core::ReleaseFetcher>,
  > {
      todo!()
  }

  // ReleaseFetcher is listed in `roles` so that:
  //   1. `__compute_capabilities!` adds `PluginCapability::ReleaseFetching` to the descriptor.
  //   2. `__assert_role_impl!` asserts `SkillsPlugin: ReleaseFetcher` at compile time.
  // The auto-generated factory from `roles: [ReleaseFetcher]` is then REPLACED by
  // `release_fetcher_create`, which injects the GitHub provider from `ReleaseFetchContext`.
  declare_plugin!(SkillsPlugin, SkillsConfig, "package_manager_skills", {
      display_name: "Agent Skills",
      family: PluginFamily::Software,
      config_model: ConfigModel::PluginConfig,
      host_requirements: HostRequirements::POSIX,
      roles: [
          Discoverer,
          VersionDetector,
          ReleaseFetcher { host_requirements: HostRequirements::CONTROLLER_ONLY },
          UpdateExecutor,
      ],
      release_fetcher_create: {
          create: create_release_fetcher_skills,
          host_requirements: HostRequirements::CONTROLLER_ONLY,
      },
  });

  #[cfg(test)]
  mod tests {
      use super::*;
      use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta};

      #[test]
      fn descriptor_type_id() {
          use uptrakit_plugin_infrastructure_core::testing::test_runtime;
          let plugin = SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create");
          assert_eq!(plugin.plugin_type_id().as_str(), "package_manager_skills");
      }

      #[test]
      fn descriptor_capabilities() {
          assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::DiscoverLocalSoftware));
          assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::DetectHostCompatibility));
          assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::VersionDetection));
          assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::ReleaseFetching));
          assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::UpdateExecution));
          // No sudo, no refresh-index, no config-test
          assert!(!DESCRIPTOR.capabilities.contains(&PluginCapability::RefreshPackageIndex));
          assert!(!DESCRIPTOR.capabilities.contains(&PluginCapability::ConfigTest));
      }

      #[test]
      fn descriptor_release_fetcher_is_controller_only() {
          let slot = DESCRIPTOR.roles.release_fetcher.as_ref().expect("slot present");
          assert!(slot.host_requirements.controller_only);
      }

      #[test]
      fn descriptor_has_expected_roles() {
          assert!(DESCRIPTOR.roles.discoverer.is_some());
          assert!(DESCRIPTOR.roles.version_detector.is_some());
          assert!(DESCRIPTOR.roles.release_fetcher.is_some());
          assert!(DESCRIPTOR.roles.update_executor.is_some());
          assert!(DESCRIPTOR.roles.package_indexer.is_none());
          assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
      }
  }
  ```

- [ ] **Step 2: Run the test to verify it fails to compile**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills plugin -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile errors — `todo!()` in factories, `Discoverer`/`VersionDetector`/`UpdateExecutor`
  traits not yet implemented.

- [ ] **Step 3: Implement `validate_identifier` and `create_release_fetcher_skills`**

  Replace the `todo!()` bodies with the real code. Add the needed imports at the top of `plugin.rs`:

  ```rust
  use std::sync::Arc;
  use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
  use uptrakit_plugin_infrastructure_core::{
      ConfigModel, HostRequirements, HostRuntime, PluginConfigValidationError, PluginFamily,
      PluginError, declare_plugin, roles::ReleaseFetchContext,
  };
  use uptrakit_global_github_provider::{GitHubProviderClient, GitHubProviderHandle};

  use crate::config::SkillsConfig;

  // ... SkillsPlugin struct and new() as above ...

  pub fn validate_identifier(
      value: &str,
  ) -> std::result::Result<(), PluginConfigValidationError> {
      crate::lock::parse_skill_identifier(value)
          .map(|_| ())
          .map_err(|e| PluginConfigValidationError::InvalidIdentifier(e.to_string()))
  }

  fn lookup_github_provider_from_ctx(
      ctx: &ReleaseFetchContext,
  ) -> Option<Arc<dyn GitHubProviderClient>> {
      #[cfg(feature = "catalog")]
      {
          let lookup = ctx.global_provider_lookup.as_ref()?;
          let handle = lookup.lookup("github")?;
          Arc::downcast::<GitHubProviderHandle>(handle)
              .ok()
              .map(|h| h.client())
      }
      #[cfg(not(feature = "catalog"))]
      { None }
  }

  pub(crate) fn create_release_fetcher_skills(
      config: &serde_json::Value,
      runtime: Arc<dyn HostRuntime>,
      ctx: &ReleaseFetchContext,
  ) -> uptrakit_plugin_infrastructure_core::error::Result<Box<dyn uptrakit_plugin_infrastructure_core::ReleaseFetcher>> {
      let _cfg: SkillsConfig = serde_json::from_value(config.clone()).map_err(|e| {
          rootcause::report!(PluginError::Configuration(format!(
              "failed to parse skills config: {e}"
          )))
      })?;
      let provider = lookup_github_provider_from_ctx(ctx);
      Ok(Box::new(SkillsPlugin {
          executor: runtime.executor(),
          provider,
      }))
  }
  ```

  Add `rootcause = { workspace = true }` is already in `Cargo.toml` — import `rootcause::report!`
  via `use rootcause::prelude::*;` at the top of the file, or use the full path `rootcause::report!`.

- [ ] **Step 4: Stub the three agent-side trait impls so the crate compiles**

  The `declare_plugin!` macro generates a struct that calls `Discoverer`, `VersionDetector`, and
  `UpdateExecutor` on `SkillsPlugin`. These trait impls live in `discovery.rs`, `detection.rs`, and
  `update.rs` respectively. For now, add minimal stubs in a temporary inline block in `plugin.rs`
  just to let this task compile:

  ```rust
  // Temporary stubs — removed when real impls land in Tasks 6–8.
  use async_trait::async_trait;
  use uptrakit_plugin_infrastructure_core::{
      DiscoveredSoftware, DiscoveryTarget, HostCompatibility, Result,
      BatchDetectItem, BatchDetectResult, Version,
      BatchUpdateItem, BatchUpdateResult, ExecuteUpdateResult, ReleaseInfo, UpdateOutputSender,
  };

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::Discoverer for SkillsPlugin {
      async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> { Ok(vec![]) }
      async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
          Ok(HostCompatibility::Compatible)
      }
  }

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::VersionDetector for SkillsPlugin {
      async fn detect_installed_version(&self, _id: &str) -> Result<Option<Version>> { Ok(None) }
  }

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for SkillsPlugin {
      async fn fetch_releases(&self, _id: &str) -> Result<Vec<uptrakit_plugin_infrastructure_core::UpstreamRelease>> { Ok(vec![]) }
  }

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::UpdateExecutor for SkillsPlugin {
      async fn execute_update(&self, _id: &str, _ver: &str, _: Option<&ReleaseInfo>, _tx: &UpdateOutputSender) -> Result<ExecuteUpdateResult> {
          Ok(ExecuteUpdateResult::new(vec![], false))
      }
  }
  ```

  **Note:** These stub impls will be replaced by the real impls in Tasks 6–8, which move the
  `impl` blocks into separate files. When those files are added, remove these stubs from
  `plugin.rs`.

- [ ] **Step 5: Add `plugin` to `lib.rs`**

  In `src/lib.rs`, add:

  ```rust
  pub(crate) mod plugin;
  pub use plugin::{DESCRIPTOR, SkillsPlugin, validate_identifier};
  ```

- [ ] **Step 6: Run the descriptor tests**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills plugin -- --nocapture 2>&1 | tail -20
  ```

  Expected: `descriptor_type_id`, `descriptor_capabilities`, `descriptor_release_fetcher_is_controller_only`,
  `descriptor_has_expected_roles` all pass.

- [ ] **Step 7: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/plugin.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): add SkillsPlugin struct, declare_plugin!, and release_fetcher factory"
  ```

---

### Task 6: `discovery.rs`

**Files:**

- Create: `crates/plugins/package-managers/skills/src/discovery.rs`

Remove the stub `Discoverer` impl from `plugin.rs` and add the real one here.

- [ ] **Step 1: Write failing tests**

  Create `crates/plugins/package-managers/skills/src/discovery.rs` with the test module only:

  ```rust
  use async_trait::async_trait;
  use rootcause::prelude::*;
  use uptrakit_plugin_infrastructure_core::command::CommandSpec;
  use uptrakit_plugin_infrastructure_core::{
      DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginError, PluginRole, Result,
      plugin_ids,
  };

  use crate::lock::{SkillLockEntry, encode_skill_identifier, parse_skill_lock};
  use crate::plugin::SkillsPlugin;

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::Discoverer for SkillsPlugin {
      async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
          todo!()
      }

      async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use uptrakit_plugin_infrastructure_core::testing::{
          FixedOutputExecutor, test_runtime_with_executor,
      };
      use uptrakit_plugin_infrastructure_core::Discoverer;

      use crate::config::SkillsConfig;
      use crate::plugin::SkillsPlugin;

      fn make_plugin(output: &str, exit_code: i32) -> SkillsPlugin {
          SkillsPlugin::new(
              SkillsConfig::default(),
              test_runtime_with_executor(FixedOutputExecutor::new(output, exit_code)),
          )
          .expect("create")
      }

      const SAMPLE_LOCK: &str = r#"{
        "brainstorming": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/brainstorming/SKILL.md",
          "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
          "installedAt": "2025-01-01T00:00:00Z",
          "updatedAt": "2025-01-02T00:00:00Z"
        }
      }"#;

      const MIXED_LOCK: &str = r#"{
        "brainstorming": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/brainstorming/SKILL.md",
          "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        },
        "local-skill": {
          "source": "local/source",
          "sourceUrl": "https://gitlab.com/local/source",
          "sourceType": "gitlab",
          "skillPath": "skills/local-skill/SKILL.md",
          "skillFolderHash": "aabbccddaabbccddaabbccddaabbccddaabbccdd"
        }
      }"#;

      #[tokio::test]
      async fn empty_lock_file_returns_empty_discovery() {
          let plugin = make_plugin("{}", 0);
          let result = plugin.discover_software().await.expect("ok");
          assert!(result.is_empty());
      }

      #[tokio::test]
      async fn command_failure_returns_empty_discovery() {
          let plugin = make_plugin("", 1);
          let result = plugin.discover_software().await.expect("ok");
          assert!(result.is_empty());
      }

      #[tokio::test]
      async fn github_entries_are_discovered() {
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let result = plugin.discover_software().await.expect("ok");
          assert_eq!(result.len(), 1);
          let sw = &result[0];
          assert_eq!(sw.name, "brainstorming");
          assert_eq!(sw.package_identifier, "brainstorming");
          assert_eq!(sw.installed_version, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
          assert_eq!(sw.targets.len(), 1);
          // DiscoveryTarget.package_identifier is the encoded form
          let encoded = sw.targets[0].package_identifier.as_deref().expect("set");
          assert!(encoded.starts_with("https://github.com/obra/superpowers#"));
          assert!(encoded.contains("skills/brainstorming/SKILL.md"));
      }

      #[tokio::test]
      async fn non_github_entries_skipped() {
          let plugin = make_plugin(MIXED_LOCK, 0);
          let result = plugin.discover_software().await.expect("ok");
          assert_eq!(result.len(), 1);
          assert_eq!(result[0].name, "brainstorming");
      }

      #[tokio::test]
      async fn detect_host_compatibility_compatible_when_npx_found() {
          use uptrakit_plugin_infrastructure_core::HostCompatibility;
          let plugin = make_plugin("", 0);
          let result = plugin.detect_host_compatibility().await.expect("ok");
          assert_eq!(result, HostCompatibility::Compatible);
      }

      #[tokio::test]
      async fn detect_host_compatibility_incompatible_when_npx_missing() {
          use uptrakit_plugin_infrastructure_core::HostCompatibility;
          let plugin = make_plugin("", 1);
          let result = plugin.detect_host_compatibility().await.expect("ok");
          match result {
              HostCompatibility::Incompatible(msg) => {
                  assert!(msg.contains("npx"), "message should mention npx, got: {msg}");
              }
              _ => panic!("expected Incompatible"),
          }
      }
  }
  ```

- [ ] **Step 2: Run the tests to confirm they fail**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills discovery -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile error or runtime panic from `todo!()`.

- [ ] **Step 3: Implement `discover_software` and `detect_host_compatibility`**

  Replace the `todo!()` bodies:

  ```rust
  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::Discoverer for SkillsPlugin {
      #[tracing::instrument(skip_all)]
      async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
          tracing::info!("discovering globally installed Skills");

          // `~` is not expanded by the kernel when passed as a path argument;
          // run through sh -c so the shell expands it. On agent-SSH hosts this
          // routes via the remote executor — tokio::fs is prohibited here.
          let cmd_output = match self
              .executor
              .execute_quiet(&CommandSpec::shell("cat ~/.agents/.skill-lock.json"))
              .await
          {
              Ok(out) if out.exit_code == 0 => out,
              Ok(_) | Err(_) => {
                  tracing::debug!("skill lock file absent or unreadable; returning empty discovery");
                  return Ok(vec![]);
              }
          };

          let entries = match parse_skill_lock(&cmd_output.output) {
              Ok(e) => e,
              Err(err) => {
                  tracing::warn!(error = %err, "failed to parse skill lock file");
                  return Ok(vec![]);
              }
          };

          let mut discovered = Vec::new();
          for entry in entries {
              if entry.source_type != "github" {
                  tracing::warn!(
                      name = %entry.name,
                      source_type = %entry.source_type,
                      "unsupported skill source type; skipping"
                  );
                  continue;
              }

              let encoded_id = encode_skill_identifier(&entry.source_url, &entry.skill_path);

              let target = DiscoveryTarget {
                  plugin_type: plugin_ids::PACKAGE_MANAGER_SKILLS.clone(),
                  plugin_config: serde_json::json!({}),
                  plugin_config_name: "Agent Skills".to_string(),
                  roles: vec![
                      PluginRole::DetectVersion,
                      PluginRole::FetchReleases,
                      PluginRole::ExecuteUpdate,
                  ],
                  package_identifier: Some(encoded_id),
                  config_override: None,
                  execution_site: None,
              };

              let extra = serde_json::json!({
                  "source_url": entry.source_url,
                  "skill_path": entry.skill_path,
              });

              discovered.push(DiscoveredSoftware {
                  package_identifier: entry.name.clone(),
                  name: entry.name,
                  installed_version: entry.skill_folder_hash,
                  targets: vec![target],
                  extra: Some(extra),
                  qualifier: None,
                  plugin_package_identifier: None,
                  featured: false,
                  installed_display_version: None,
              });
          }

          tracing::debug!(count = discovered.len(), "skills discovery complete");
          Ok(discovered)
      }

      #[tracing::instrument(skip_all)]
      async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
          match self
              .executor
              .execute_quiet(&CommandSpec::exec("which", ["npx".to_string()]))
              .await
          {
              Ok(_) => Ok(HostCompatibility::Compatible),
              Err(_) => Ok(HostCompatibility::Incompatible("npx not found".to_string())),
          }
      }
  }
  ```

  **Important:** Check the exact API of `CommandSpec::shell` and `CommandSpec::exec` against the
  codebase — use the exact variant names that exist. If `CommandSpec::shell` does not exist,
  use `CommandSpec::exec("sh", ["-c".to_string(), "cat ~/.agents/.skill-lock.json".to_string()])`.

- [ ] **Step 4: Remove the stub `Discoverer` impl from `plugin.rs`**

  Find the `#[async_trait] impl Discoverer for SkillsPlugin` block in `plugin.rs` and delete it.
  Also remove the stub imports from `plugin.rs` that are no longer needed.

- [ ] **Step 5: Add `discovery` to `lib.rs`**

  ```rust
  pub(crate) mod discovery;
  ```

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills discovery -- --nocapture 2>&1 | tail -20
  ```

  Expected: all 6 tests pass.

- [ ] **Step 7: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/discovery.rs \
    crates/plugins/package-managers/skills/src/plugin.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): implement Discoverer and detect_host_compatibility"
  ```

---

### Task 7: `detection.rs`

**Files:**

- Create: `crates/plugins/package-managers/skills/src/detection.rs`

- [ ] **Step 1: Write failing tests**

  Create `crates/plugins/package-managers/skills/src/detection.rs`:

  ```rust
  use async_trait::async_trait;
  use uptrakit_plugin_infrastructure_core::{
      BatchDetectItem, BatchDetectResult, Result, Version,
  };
  use uptrakit_plugin_infrastructure_core::command::CommandSpec;

  use crate::lock::{parse_skill_identifier, parse_skill_lock};
  use crate::plugin::SkillsPlugin;

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::VersionDetector for SkillsPlugin {
      async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
          todo!()
      }

      async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use uptrakit_plugin_infrastructure_core::testing::{
          FixedOutputExecutor, test_runtime_with_executor,
      };
      use uptrakit_plugin_infrastructure_core::{BatchDetectItem, Version, VersionDetector};

      use crate::config::SkillsConfig;
      use crate::lock::encode_skill_identifier;
      use crate::plugin::SkillsPlugin;

      const SAMPLE_LOCK: &str = r#"{
        "brainstorming": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/brainstorming/SKILL.md",
          "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        }
      }"#;

      fn make_plugin(lock_output: &str, exit_code: i32) -> SkillsPlugin {
          SkillsPlugin::new(
              SkillsConfig::default(),
              test_runtime_with_executor(FixedOutputExecutor::new(lock_output, exit_code)),
          )
          .expect("create")
      }

      fn brainstorming_id() -> String {
          encode_skill_identifier(
              "https://github.com/obra/superpowers",
              "skills/brainstorming/SKILL.md",
          )
      }

      #[tokio::test]
      async fn detect_installed_version_found() {
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let result = plugin.detect_installed_version(&brainstorming_id()).await.expect("ok");
          assert_eq!(
              result,
              Some(Version::new("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
          );
      }

      #[tokio::test]
      async fn detect_installed_version_skill_not_in_lock() {
          let other_id = encode_skill_identifier(
              "https://github.com/other/repo",
              "skills/other/SKILL.md",
          );
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let result = plugin.detect_installed_version(&other_id).await.expect("ok");
          assert_eq!(result, None);
      }

      #[tokio::test]
      async fn detect_installed_version_command_fails_returns_none() {
          let plugin = make_plugin("", 1);
          let result = plugin.detect_installed_version(&brainstorming_id()).await.expect("ok");
          assert_eq!(result, None);
      }

      #[tokio::test]
      async fn detect_installed_version_invalid_identifier_fails() {
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let result = plugin.detect_installed_version("not-an-identifier").await;
          assert!(result.is_err());
      }

      #[tokio::test]
      async fn batch_detect_single_match() {
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let items = vec![BatchDetectItem::new(brainstorming_id())];
          let results = plugin.batch_detect(&items).await.expect("ok");
          assert_eq!(results.len(), 1);
          assert_eq!(
              results[0].installed_version,
              Some(Version::new("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
          );
          assert!(results[0].error.is_none());
      }

      #[tokio::test]
      async fn batch_detect_partial_miss_does_not_fail_batch() {
          let other_id = encode_skill_identifier("https://github.com/other/repo", "skills/other/SKILL.md");
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let items = vec![
              BatchDetectItem::new(brainstorming_id()),
              BatchDetectItem::new(other_id.clone()),
          ];
          let results = plugin.batch_detect(&items).await.expect("ok");
          assert_eq!(results.len(), 2);
          let found = results.iter().find(|r| r.package_identifier == brainstorming_id()).expect("found");
          assert!(found.installed_version.is_some());
          assert!(found.error.is_none());
          let miss = results.iter().find(|r| r.package_identifier == other_id).expect("miss");
          assert_eq!(miss.installed_version, None);
          assert!(miss.error.is_none());
      }

      #[tokio::test]
      async fn batch_detect_invalid_id_returns_per_item_error() {
          let plugin = make_plugin(SAMPLE_LOCK, 0);
          let items = vec![
              BatchDetectItem::new(brainstorming_id()),
              BatchDetectItem::new("invalid-id".to_string()),
          ];
          let results = plugin.batch_detect(&items).await.expect("batch succeeds");
          let invalid = results.iter().find(|r| r.package_identifier == "invalid-id").expect("invalid");
          assert!(invalid.error.is_some(), "invalid id should produce per-item error");
      }
  }
  ```

- [ ] **Step 2: Run to confirm failures**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills detection -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile error from `todo!()`.

- [ ] **Step 3: Implement `detect_installed_version` and `batch_detect`**

  Replace the `todo!()` bodies:

  ```rust
  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::VersionDetector for SkillsPlugin {
      #[tracing::instrument(skip_all)]
      async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
          let (url, skill_path) = parse_skill_identifier(package_identifier)
              .map_err(|e| e.change_context(uptrakit_plugin_infrastructure_core::PluginError::Configuration(
                  format!("invalid identifier: {package_identifier}: {e}")
              )))?;

          let cmd_output = match self
              .executor
              .execute_quiet(&CommandSpec::exec("sh", [
                  "-c".to_string(),
                  "cat ~/.agents/.skill-lock.json".to_string(),
              ]))
              .await
          {
              Ok(out) if out.exit_code == 0 => out,
              Ok(_) | Err(_) => return Ok(None),
          };

          let entries = match parse_skill_lock(&cmd_output.output) {
              Ok(e) => e,
              Err(_) => return Ok(None),
          };

          let source_url = url.as_str().trim_end_matches('/');
          let found = entries.iter().find(|e| {
              e.source_url.trim_end_matches('/') == source_url && e.skill_path == skill_path
          });

          Ok(found.map(|e| Version::new(&e.skill_folder_hash)))
      }

      #[tracing::instrument(skip_all)]
      async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
          if items.is_empty() {
              return Ok(vec![]);
          }

          // Read lock file once.
          let lock_content: Option<Vec<crate::lock::SkillLockEntry>> = match self
              .executor
              .execute_quiet(&CommandSpec::exec("sh", [
                  "-c".to_string(),
                  "cat ~/.agents/.skill-lock.json".to_string(),
              ]))
              .await
          {
              Ok(out) if out.exit_code == 0 => parse_skill_lock(&out.output).ok(),
              _ => None,
          };

          let results = items
              .iter()
              .map(|item| {
                  match parse_skill_identifier(&item.package_identifier) {
                      Err(e) => BatchDetectResult::error(
                          item.package_identifier.clone(),
                          e.to_string(),
                      ),
                      Ok((url, skill_path)) => {
                          let version = lock_content.as_ref().and_then(|entries| {
                              let source_url = url.as_str().trim_end_matches('/');
                              entries
                                  .iter()
                                  .find(|e| {
                                      e.source_url.trim_end_matches('/') == source_url
                                          && e.skill_path == skill_path
                                  })
                                  .map(|e| Version::new(&e.skill_folder_hash))
                          });
                          BatchDetectResult::new(item.package_identifier.clone(), version, None)
                      }
                  }
              })
              .collect();

          Ok(results)
      }
  }
  ```

  Note: `BatchDetectResult::new(id, version, error)` — verify the constructor signature against
  the actual codebase (some crates use `BatchDetectResult::found` / `BatchDetectResult::empty` /
  `BatchDetectResult::error`). Use whatever variant constructors exist.

- [ ] **Step 4: Remove the stub `VersionDetector` impl from `plugin.rs`**

  Delete the stub `impl VersionDetector for SkillsPlugin` block from `plugin.rs`.

- [ ] **Step 5: Add `detection` to `lib.rs`**

  ```rust
  pub(crate) mod detection;
  ```

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills detection -- --nocapture 2>&1 | tail -20
  ```

  Expected: all 7 tests pass.

- [ ] **Step 7: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/detection.rs \
    crates/plugins/package-managers/skills/src/plugin.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): implement VersionDetector with lock-file lookup"
  ```

---

### Task 8: `releases.rs`

**Files:**

- Create: `crates/plugins/package-managers/skills/src/releases.rs`

The controller-side `ReleaseFetcher`. Uses `self.provider` (populated by
`create_release_fetcher_skills`). Overrides `batch_fetch` to issue one GitHub tree call per
`(owner, repo)` pair.

- [ ] **Step 1: Write failing tests**

  Create `crates/plugins/package-managers/skills/src/releases.rs`:

  ```rust
  use std::collections::HashMap;
  use std::sync::Arc;

  use async_trait::async_trait;
  use rootcause::prelude::*;
  use uptrakit_global_github_provider::{
      GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntryKind,
      GlobalProviderConsumerId, PACKAGE_MANAGER_SKILLS,
  };
  use uptrakit_plugin_infrastructure_core::{PluginError, Result, UpstreamRelease, Version};
  use uptrakit_plugin_infrastructure_core::{BatchFetchItem, BatchFetchResult};

  use crate::error::SkillsError;
  use crate::lock::parse_skill_identifier;
  use crate::plugin::SkillsPlugin;

  /// Strip the filename from a skill path to get the folder path.
  ///
  /// `"skills/brainstorming/SKILL.md"` → `"skills/brainstorming"`.
  /// If the path has no `/`, returns `""` (malformed; caller emits zero releases).
  fn derive_skill_dir(skill_path: &str) -> &str {
      skill_path.rfind('/').map(|i| &skill_path[..i]).unwrap_or("")
  }

  fn map_provider_error(e: GitHubProviderError) -> Report<PluginError> {
      match e {
          GitHubProviderError::Throttled => {
              report!(PluginError::PluginInternal("GitHub rate limit exceeded".to_string()))
          }
          GitHubProviderError::AuthFailed(msg) | GitHubProviderError::Misconfigured(msg) => {
              report!(PluginError::Configuration(format!("GitHub auth error: {msg}")))
          }
          GitHubProviderError::UpstreamUnavailable(msg)
          | GitHubProviderError::RequestFailed(msg) => {
              report!(PluginError::PluginInternal(format!(
                  "GitHub upstream error: {msg}"
              )))
          }
          _ => report!(PluginError::PluginInternal(format!("GitHub provider error: {e}"))),
      }
  }

  fn parse_github_owner_repo(url: &url::Url) -> Result<(String, String)> {
      let host = url.host_str().unwrap_or("");
      if host != "github.com" {
          return Err(report!(SkillsError::UnsupportedSource(format!(
              "non-GitHub source not supported: {host}"
          ))));
      }
      let path = url.path().trim_start_matches('/');
      let mut parts = path.splitn(3, '/');
      let owner = parts
          .next()
          .filter(|s| !s.is_empty())
          .ok_or_else(|| report!(SkillsError::InvalidIdentifier("missing owner in URL".to_string())))?;
      let repo = parts
          .next()
          .map(|s| s.trim_end_matches(".git"))
          .filter(|s| !s.is_empty())
          .ok_or_else(|| report!(SkillsError::InvalidIdentifier("missing repo in URL".to_string())))?;
      Ok((owner.to_string(), repo.to_string()))
  }

  fn tree_to_release(
      tree: &GitHubRepositoryTree,
      skill_dir: &str,
      owner: &str,
      repo: &str,
  ) -> Option<UpstreamRelease> {
      if tree.truncated {
          return None; // handled at call site
      }
      let entry = tree.entries.iter().find(|e| {
          e.path == skill_dir && matches!(e.kind, GitHubTreeEntryKind::Tree)
      })?;
      Some(UpstreamRelease::new(
          Version::new(&entry.sha),
          entry.sha.clone(),
          false,
          format!("https://github.com/{owner}/{repo}/tree/HEAD/{skill_dir}"),
      ))
  }

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for SkillsPlugin {
      #[tracing::instrument(skip_all)]
      async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
          let (url, skill_path) = parse_skill_identifier(package_identifier)
              .map_err(|e| e.change_context(PluginError::Configuration(e.to_string())))?;

          let (owner, repo) = parse_github_owner_repo(&url)?;
          let skill_dir = derive_skill_dir(&skill_path);
          if skill_dir.is_empty() {
              tracing::warn!(package_identifier, "skill path has no directory component; returning zero releases");
              return Ok(vec![]);
          }

          let provider = self.provider.as_ref().ok_or_else(|| {
              report!(PluginError::PluginInternal(
                  "global GitHub provider not available".to_string()
              ))
          })?;

          let tree = provider
              .fetch_repository_tree(PACKAGE_MANAGER_SKILLS, &owner, &repo, "HEAD", true)
              .await
              .map_err(map_provider_error)?;

          if tree.truncated {
              tracing::warn!(
                  owner = %owner,
                  repo = %repo,
                  truncated = true,
                  "GitHub tree truncated; returning zero releases for this skill"
              );
              return Err(report!(PluginError::PluginInternal(format!(
                  "GitHub tree truncated for {owner}/{repo}; repository may be too large"
              ))));
          }

          let release = tree_to_release(&tree, skill_dir, &owner, &repo);
          match release {
              Some(r) => Ok(vec![r]),
              None => Err(report!(PluginError::PluginInternal(format!(
                  "skill folder '{skill_dir}' not found in {owner}/{repo} HEAD tree"
              )))),
          }
      }

      #[tracing::instrument(skip_all)]
      async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
          if items.is_empty() {
              return Ok(vec![]);
          }

          // Group items by (owner, repo).
          // Items with invalid identifiers get an immediate per-item error.
          struct ParsedItem {
              id: String,
              owner: String,
              repo: String,
              skill_dir: String,
          }

          let mut parsed: Vec<ParsedItem> = Vec::new();
          let mut results: Vec<BatchFetchResult> = Vec::new();

          for item in items {
              match parse_skill_identifier(&item.package_identifier) {
                  Err(e) => {
                      results.push(BatchFetchResult::error(
                          item.package_identifier.clone(),
                          e.to_string(),
                      ));
                  }
                  Ok((url, skill_path)) => {
                      match parse_github_owner_repo(&url) {
                          Err(e) => {
                              results.push(BatchFetchResult::error(
                                  item.package_identifier.clone(),
                                  e.to_string(),
                              ));
                          }
                          Ok((owner, repo)) => {
                              let skill_dir = derive_skill_dir(&skill_path).to_string();
                              parsed.push(ParsedItem {
                                  id: item.package_identifier.clone(),
                                  owner,
                                  repo,
                                  skill_dir,
                              });
                          }
                      }
                  }
              }
          }

          if parsed.is_empty() {
              return Ok(results);
          }

          let provider = match &self.provider {
              Some(p) => p,
              None => {
                  for item in &parsed {
                      results.push(BatchFetchResult::error(
                          item.id.clone(),
                          "global GitHub provider not available".to_string(),
                      ));
                  }
                  return Ok(results);
              }
          };

          // Group by (owner, repo).
          let mut groups: HashMap<(String, String), Vec<&ParsedItem>> = HashMap::new();
          for item in &parsed {
              groups
                  .entry((item.owner.clone(), item.repo.clone()))
                  .or_default()
                  .push(item);
          }

          for ((owner, repo), group_items) in groups {
              match provider
                  .fetch_repository_tree(PACKAGE_MANAGER_SKILLS, &owner, &repo, "HEAD", true)
                  .await
              {
                  Err(e) => {
                      let msg = format!("{e}");
                      for item in group_items {
                          results.push(BatchFetchResult::error(item.id.clone(), msg.clone()));
                      }
                  }
                  Ok(tree) if tree.truncated => {
                      let msg = format!(
                          "GitHub tree truncated for {owner}/{repo}; repository may be too large"
                      );
                      tracing::warn!(owner = %owner, repo = %repo, "GitHub tree truncated");
                      for item in group_items {
                          results.push(BatchFetchResult::error(item.id.clone(), msg.clone()));
                      }
                  }
                  Ok(tree) => {
                      for item in group_items {
                          if item.skill_dir.is_empty() {
                              results.push(BatchFetchResult::empty(item.id.clone()));
                              continue;
                          }
                          match tree_to_release(&tree, &item.skill_dir, &owner, &repo) {
                              Some(release) => {
                                  results.push(BatchFetchResult::found(
                                      item.id.clone(),
                                      vec![release],
                                  ));
                              }
                              None => {
                                  results.push(BatchFetchResult::error(
                                      item.id.clone(),
                                      format!(
                                          "skill folder '{}' not found in {owner}/{repo} HEAD tree",
                                          item.skill_dir
                                      ),
                                  ));
                              }
                          }
                      }
                  }
              }
          }

          Ok(results)
      }
  }

  #[cfg(test)]
  mod tests {
      use std::sync::Arc;

      use async_trait::async_trait;
      use uptrakit_global_github_provider::{
          GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntry,
          GitHubTreeEntryKind, GlobalProviderConsumerId,
      };
      use uptrakit_plugin_infrastructure_core::{BatchFetchItem, ReleaseFetcher};
      use uptrakit_plugin_infrastructure_core::testing::test_runtime;

      use crate::config::SkillsConfig;
      use crate::lock::encode_skill_identifier;
      use crate::plugin::SkillsPlugin;

      fn skill_with_provider(provider: Arc<dyn GitHubProviderClient>) -> SkillsPlugin {
          SkillsPlugin {
              executor: test_runtime().executor(),
              provider: Some(provider),
          }
      }

      fn skill_no_provider() -> SkillsPlugin {
          SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create")
      }

      fn brainstorming_id() -> String {
          encode_skill_identifier(
              "https://github.com/obra/superpowers",
              "skills/brainstorming/SKILL.md",
          )
      }

      struct FakeProvider {
          tree: GitHubRepositoryTree,
      }

      #[async_trait]
      impl GitHubProviderClient for FakeProvider {
          async fn fetch_repository_tree(
              &self,
              _consumer: GlobalProviderConsumerId,
              _owner: &str,
              _repo: &str,
              _git_ref: &str,
              _recursive: bool,
          ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
              Ok(self.tree.clone())
          }
      }

      struct FailProvider {
          error: GitHubProviderError,
      }

      #[async_trait]
      impl GitHubProviderClient for FailProvider {
          async fn fetch_repository_tree(
              &self,
              _consumer: GlobalProviderConsumerId,
              _owner: &str,
              _repo: &str,
              _git_ref: &str,
              _recursive: bool,
          ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
              Err(self.error.clone())
          }
      }

      struct CountingProvider {
          calls: std::sync::atomic::AtomicUsize,
          tree: GitHubRepositoryTree,
      }

      #[async_trait]
      impl GitHubProviderClient for CountingProvider {
          async fn fetch_repository_tree(
              &self,
              _consumer: GlobalProviderConsumerId,
              _owner: &str,
              _repo: &str,
              _git_ref: &str,
              _recursive: bool,
          ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
              self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
              Ok(self.tree.clone())
          }
      }

      fn make_tree(entries: Vec<(&str, GitHubTreeEntryKind, &str)>) -> GitHubRepositoryTree {
          GitHubRepositoryTree {
              truncated: false,
              entries: entries
                  .into_iter()
                  .map(|(path, kind, sha)| GitHubTreeEntry {
                      path: path.to_string(),
                      kind,
                      sha: sha.to_string(),
                  })
                  .collect(),
          }
      }

      #[tokio::test]
      async fn fetch_releases_skill_folder_found_returns_one_release() {
          let tree = make_tree(vec![
              ("skills/brainstorming", GitHubTreeEntryKind::Tree, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
              ("skills/brainstorming/SKILL.md", GitHubTreeEntryKind::Blob, "aabbccddaabbccddaabbccddaabbccddaabbccdd"),
          ]);
          let plugin = skill_with_provider(Arc::new(FakeProvider { tree }));
          let releases = plugin.fetch_releases(&brainstorming_id()).await.expect("ok");
          assert_eq!(releases.len(), 1);
          assert_eq!(releases[0].tag, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
          assert!(!releases[0].is_prerelease);
          assert!(releases[0].release_url.contains("skills/brainstorming"));
      }

      #[tokio::test]
      async fn fetch_releases_skill_folder_missing_returns_error() {
          let tree = make_tree(vec![
              ("skills/other", GitHubTreeEntryKind::Tree, "aabbccddaabbccddaabbccddaabbccddaabbccdd"),
          ]);
          let plugin = skill_with_provider(Arc::new(FakeProvider { tree }));
          let result = plugin.fetch_releases(&brainstorming_id()).await;
          assert!(result.is_err());
      }

      #[tokio::test]
      async fn fetch_releases_no_provider_returns_error() {
          let plugin = skill_no_provider();
          let result = plugin.fetch_releases(&brainstorming_id()).await;
          let err = result.unwrap_err();
          assert!(
              err.to_string().contains("provider not available")
                  || err.to_string().contains("provider")
          );
      }

      #[tokio::test]
      async fn fetch_releases_non_github_id_returns_error() {
          let id = encode_skill_identifier(
              "https://gitlab.com/owner/repo",
              "skills/foo/SKILL.md",
          );
          let tree = make_tree(vec![]);
          let plugin = skill_with_provider(Arc::new(FakeProvider { tree }));
          let result = plugin.fetch_releases(&id).await;
          assert!(result.is_err());
      }

      #[tokio::test]
      async fn fetch_releases_throttled_maps_to_plugin_internal() {
          let plugin = skill_with_provider(Arc::new(FailProvider {
              error: GitHubProviderError::Throttled,
          }));
          let result = plugin.fetch_releases(&brainstorming_id()).await;
          assert!(result.is_err());
      }

      #[tokio::test]
      async fn batch_fetch_one_tree_call_per_repo() {
          let tree = make_tree(vec![
              ("skills/brainstorming", GitHubTreeEntryKind::Tree, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
              ("skills/spec", GitHubTreeEntryKind::Tree, "cafecafecafecafecafecafecafecafecafecafe"),
          ]);
          let provider = Arc::new(CountingProvider {
              calls: std::sync::atomic::AtomicUsize::new(0),
              tree,
          });
          let plugin = skill_with_provider(Arc::clone(&provider) as Arc<dyn GitHubProviderClient>);
          let items = vec![
              BatchFetchItem::new(brainstorming_id()),
              BatchFetchItem::new(encode_skill_identifier(
                  "https://github.com/obra/superpowers",
                  "skills/spec/SKILL.md",
              )),
          ];
          let results = plugin.batch_fetch(&items).await.expect("ok");
          // Only ONE tree call for two skills in the same repo
          assert_eq!(provider.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
          assert_eq!(results.len(), 2);
          for r in &results {
              assert!(r.error.is_none(), "no error expected; got: {:?}", r.error);
              assert_eq!(r.releases.len(), 1);
          }
      }
  }
  ```

- [ ] **Step 2: Run to confirm failures**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills releases -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile errors (stub `ReleaseFetcher` from `plugin.rs` conflicts with the real impl).

- [ ] **Step 3: Remove the stub `ReleaseFetcher` impl from `plugin.rs`**

  Delete the stub `impl ReleaseFetcher for SkillsPlugin` block from `plugin.rs`.

- [ ] **Step 4: Add `releases` to `lib.rs`**

  ```rust
  pub(crate) mod releases;
  ```

- [ ] **Step 5: Run tests**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills releases -- --nocapture 2>&1 | tail -20
  ```

  Expected: all 6 tests pass.

- [ ] **Step 6: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/releases.rs \
    crates/plugins/package-managers/skills/src/plugin.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): implement ReleaseFetcher with batch_fetch repo grouping"
  ```

---

### Task 9: `update.rs`

**Files:**

- Create: `crates/plugins/package-managers/skills/src/update.rs`

- [ ] **Step 1: Write failing tests**

  Create `crates/plugins/package-managers/skills/src/update.rs`:

  ```rust
  use async_trait::async_trait;
  use rootcause::prelude::*;
  use uptrakit_plugin_infrastructure_core::{
      ExecuteUpdateResult, PluginError, ReleaseInfo, Result, UpdateOutputSender,
  };

  use crate::lock::{parse_skill_identifier, parse_skill_lock};
  use crate::plugin::SkillsPlugin;
  use uptrakit_plugin_infrastructure_core::command::CommandSpec;

  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::UpdateExecutor for SkillsPlugin {
      #[tracing::instrument(skip_all)]
      async fn execute_update(
          &self,
          package_identifier: &str,
          to_version: &str,
          _release_info: Option<&ReleaseInfo>,
          output_tx: &UpdateOutputSender,
      ) -> Result<ExecuteUpdateResult> {
          todo!()
      }
  }

  #[cfg(test)]
  mod tests {
      use std::sync::Arc;

      use uptrakit_plugin_infrastructure_core::UpdateExecutor;
      use uptrakit_plugin_infrastructure_core::testing::{
          FixedOutputExecutor, test_runtime, test_runtime_with_executor,
      };
      use uptrakit_plugin_infrastructure_core::command::MockUpdateOutputSender;

      use crate::config::SkillsConfig;
      use crate::lock::encode_skill_identifier;
      use crate::plugin::SkillsPlugin;

      const SAMPLE_LOCK: &str = r#"{
        "brainstorming": {
          "source": "obra/superpowers",
          "sourceUrl": "https://github.com/obra/superpowers",
          "sourceType": "github",
          "skillPath": "skills/brainstorming/SKILL.md",
          "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        }
      }"#;

      fn brainstorming_id() -> String {
          encode_skill_identifier(
              "https://github.com/obra/superpowers",
              "skills/brainstorming/SKILL.md",
          )
      }

      #[tokio::test]
      async fn execute_update_calls_npx_skills_update() {
          // The executor returns SAMPLE_LOCK on the first call (lock file read)
          // and "" exit 0 on the second call (npx skills update).
          // FixedOutputExecutor returns the same output for every call — so we
          // need a sequenced executor or a custom one. Since FixedOutputExecutor
          // always returns the same output, set it to SAMPLE_LOCK so both calls
          // succeed.
          let executor = FixedOutputExecutor::new(SAMPLE_LOCK, 0);
          let plugin = SkillsPlugin::new(
              SkillsConfig::default(),
              test_runtime_with_executor(executor),
          )
          .expect("create");

          let (tx, _rx) = uptrakit_plugin_infrastructure_core::command::update_output_channel();
          let result = plugin
              .execute_update(&brainstorming_id(), "some_sha", None, &tx)
              .await
              .expect("update ok");
          assert!(!result.reboot_required);
      }

      #[tokio::test]
      async fn execute_update_invalid_identifier_fails() {
          let plugin = SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create");
          let (tx, _rx) = uptrakit_plugin_infrastructure_core::command::update_output_channel();
          let result = plugin
              .execute_update("not-an-id", "sha", None, &tx)
              .await;
          assert!(result.is_err());
      }

      #[tokio::test]
      async fn execute_update_skill_not_in_lock_fails() {
          let executor = FixedOutputExecutor::new("{}", 0);
          let plugin = SkillsPlugin::new(
              SkillsConfig::default(),
              test_runtime_with_executor(executor),
          )
          .expect("create");
          let (tx, _rx) = uptrakit_plugin_infrastructure_core::command::update_output_channel();
          let result = plugin
              .execute_update(&brainstorming_id(), "sha", None, &tx)
              .await;
          assert!(result.is_err());
      }
  }
  ```

  **Note on test helpers:** check what update output sender helpers exist in the codebase. If
  `uptrakit_plugin_infrastructure_core::command::update_output_channel()` does not exist, search
  for the correct function/type by running:

  ```bash
  grep -rn "update_output_channel\|UpdateOutputSender\|fn.*output.*channel" \
    crates/plugins/infrastructure/core/src/ | head -20
  ```

  Use the exact API found.

- [ ] **Step 2: Run to confirm failures**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills update -- --nocapture 2>&1 | tail -20
  ```

  Expected: compile error from `todo!()`.

- [ ] **Step 3: Implement `execute_update`**

  Replace the `todo!()` body:

  ```rust
  #[async_trait]
  impl uptrakit_plugin_infrastructure_core::UpdateExecutor for SkillsPlugin {
      #[tracing::instrument(skip_all)]
      async fn execute_update(
          &self,
          package_identifier: &str,
          to_version: &str,
          _release_info: Option<&ReleaseInfo>,
          output_tx: &UpdateOutputSender,
      ) -> Result<ExecuteUpdateResult> {
          let (url, skill_path) = parse_skill_identifier(package_identifier)
              .map_err(|e| e.change_context(PluginError::Configuration(e.to_string())))?;

          // Read the lock file to recover the Skill name for the CLI.
          let lock_output = self
              .executor
              .execute_quiet(&CommandSpec::exec("sh", [
                  "-c".to_string(),
                  "cat ~/.agents/.skill-lock.json".to_string(),
              ]))
              .await
              .map_err(|e| {
                  report!(PluginError::PluginInternal(format!(
                      "failed to read skill lock file: {e}"
                  )))
              })?;

          if lock_output.exit_code != 0 {
              return Err(report!(PluginError::PluginInternal(
                  "skill lock file not found".to_string()
              )));
          }

          let entries = parse_skill_lock(&lock_output.output)
              .map_err(|e| e.change_context(PluginError::PluginInternal(e.to_string())))?;

          let source_url = url.as_str().trim_end_matches('/');
          let skill_name = entries
              .iter()
              .find(|e| {
                  e.source_url.trim_end_matches('/') == source_url && e.skill_path == skill_path
              })
              .map(|e| e.name.clone())
              .ok_or_else(|| {
                  report!(PluginError::PluginInternal(format!(
                      "skill not installed: {package_identifier}"
                  )))
              })?;

          tracing::info!(
              skill = %skill_name,
              to_version = %to_version,
              "running npx skills update"
          );

          let output = uptrakit_plugin_infrastructure_core::execute_command_update(
              uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                  executor: self.executor.as_ref(),
                  binary: "sh",
                  args: vec![
                      "-c".to_string(),
                      format!("npx skills@latest update -g {skill_name} -y"),
                  ],
                  privileged: false,
                  spec_modifier: None,
                  exit_code_success: None,
                  exit_code_error: None,
              },
              output_tx,
          )
          .await?;

          Ok(ExecuteUpdateResult::new(output, false))
      }
  }
  ```

- [ ] **Step 4: Remove the stub `UpdateExecutor` impl from `plugin.rs`**

  Delete the stub `impl UpdateExecutor for SkillsPlugin` block from `plugin.rs`. Also clean up
  stub imports no longer needed in `plugin.rs`.

- [ ] **Step 5: Add `update` to `lib.rs`**

  ```rust
  pub(crate) mod update;
  ```

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p uptrakit-plugin-package-manager-skills update -- --nocapture 2>&1 | tail -20
  ```

  Expected: at least `execute_update_invalid_identifier_fails` and
  `execute_update_skill_not_in_lock_fails` pass. The `execute_update_calls_npx_skills_update`
  test is harder because `FixedOutputExecutor` returns the same output for all calls — if it
  fails, confirm the test setup is correct against the actual `FixedOutputExecutor` API.

- [ ] **Step 7: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/update.rs \
    crates/plugins/package-managers/skills/src/plugin.rs \
    crates/plugins/package-managers/skills/src/lib.rs
  git commit -m "feat(skills): implement UpdateExecutor with npx skills update"
  ```

---

### Task 10: Final `lib.rs` + registry integration

**Files:**

- Modify: `crates/plugins/package-managers/skills/src/lib.rs`
- Modify: `crates/plugins/infrastructure/registry/src/registry.rs`

- [ ] **Step 1: Write the registry test first**

  In `crates/plugins/infrastructure/registry/src/registry.rs`, add a test at the bottom of
  the `#[cfg(test)]` block:

  ```rust
  #[test]
  fn package_manager_skills_is_registered() {
      use uptrakit_shared_types::plugin_ids;
      let descs = all_descriptors();
      assert!(
          descs.iter().any(|d| d.type_id == plugin_ids::PACKAGE_MANAGER_SKILLS),
          "PACKAGE_MANAGER_SKILLS must be in all_descriptors()"
      );
      assert!(
          is_package_manager_plugin(&plugin_ids::PACKAGE_MANAGER_SKILLS),
          "PACKAGE_MANAGER_SKILLS must be in is_package_manager_plugin"
      );
  }
  ```

- [ ] **Step 2: Run the test to confirm it fails**

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-registry package_manager_skills_is_registered -- --nocapture 2>&1 | tail -10
  ```

  Expected: test fails — descriptor not found.

- [ ] **Step 3: Update `all_descriptors()` in `registry.rs`**

  Add the new descriptor after the npm entry:

  ```rust
  &uptrakit_plugin_package_manager_npm::DESCRIPTOR,
  &uptrakit_plugin_package_manager_skills::DESCRIPTOR,
  ```

- [ ] **Step 4: Update `is_package_manager_plugin` in `registry.rs`**

  Add to the slice in the `is_package_manager_plugin` function:

  ```rust
  plugin_ids::PACKAGE_MANAGER_NPM,
  plugin_ids::PACKAGE_MANAGER_SKILLS,
  ```

- [ ] **Step 5: Update the `every_always_on_plugin_id` test array in `registry.rs`**

  Find the test `every_always_on_plugin_ids_in_all_descriptors` (or similar). This test lists
  every expected plugin type ID. Add:

  ```rust
  &plugin_ids::PACKAGE_MANAGER_SKILLS,
  ```

  after the `PACKAGE_MANAGER_NPM` entry.

- [ ] **Step 6: Finalise `lib.rs`**

  Replace `src/lib.rs` with the complete module list and re-exports:

  ```rust
  pub(crate) mod config;
  pub(crate) mod detection;
  pub(crate) mod discovery;
  pub(crate) mod error;
  pub(crate) mod lock;
  pub(crate) mod plugin;
  pub(crate) mod releases;
  pub(crate) mod update;

  pub use config::SkillsConfig;
  pub use error::{Result, SkillsError};
  pub use lock::{SkillLockEntry, encode_skill_identifier, parse_skill_identifier, parse_skill_lock};
  pub use plugin::{DESCRIPTOR, SkillsPlugin, validate_identifier};
  ```

- [ ] **Step 7: Compile the full workspace**

  ```bash
  cargo check --all-features 2>&1 | grep "^error" | head -30
  ```

  Expected: no errors.

- [ ] **Step 8: Run the registry tests**

  ```bash
  cargo test -p uptrakit-plugin-infrastructure-registry --all-features 2>&1 | tail -20
  ```

  Expected: all pass, including `package_manager_skills_is_registered`.

- [ ] **Step 9: Commit**

  ```bash
  git add \
    crates/plugins/package-managers/skills/src/lib.rs \
    crates/plugins/infrastructure/registry/src/registry.rs
  git commit -m "feat(registry): register PACKAGE_MANAGER_SKILLS in all_descriptors and is_package_manager_plugin"
  ```

---

### Task 11: Documentation

**Files:**

- Create: `docs/adr/0015-release-fetcher-context.md`
- Modify: `docs/development/plugin-guidelines.md`
- Create: `docs/end-user/skills-plugin.md`

- [ ] **Step 1: Write ADR 0015**

  Create `docs/adr/0015-release-fetcher-context.md`:

  ```markdown
  # ADR 0015 — ReleaseFetchContext: Extending ReleaseFetcher Factories

  Date: 2026-05-13
  Status: Accepted

  ## Context

  The `package_manager_skills` plugin requires controller-side access to the global GitHub
  Provider (for fetching git tree SHAs). Prior to this change, `ReleaseFetcher` factories
  received only `(config_json, runtime: Arc<dyn HostRuntime>)`. The GitHub Provider is an
  instance-level singleton that lives in `GlobalProviders`, not in `HostRuntime`.

  ## Decision

  Introduce `ReleaseFetchContext` — a `#[non_exhaustive]` struct passed as a third argument
  to `ReleaseFetcher` factory functions. The struct carries
  `global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>` (gated on the `catalog`
  feature). All existing factory functions add `_ctx: &ReleaseFetchContext` and ignore it.

  ## Alternatives Rejected

  - **Adding `global_provider_lookup()` to `HostRuntime`:** `HostRuntime` is a host-execution
    abstraction. Leaking provider-registry concerns into it would require all implementations
    (`StandardHostRuntime`, `MetadataAwareHostRuntime`, `RouterOsHostRuntime`,
    `ControllerRuntime`) to carry the lookup even when they have no access to it, and would
    create delegation footguns in wrapper runtimes.
  - **Scheduler-side token injection:** Would require a new per-plugin field in
    `PluginDescriptor` and an additional lookup phase inside the scheduler, duplicating
    what `GlobalProviders` already handles.
  - **Per-plugin `auth_token` field in config:** Exposes a parallel credential surface.
    Operators must configure two places instead of one. Rejected for ergonomic and security
    reasons.

  ## Consequences

  - `CreateReleaseFetcherFn` is now a 3-arg type alias (config, runtime, &context).
  - All existing plugin factories get a mechanical `_ctx` parameter addition — no behaviour
    change.
  - Future providers (GitLab, Forgejo) can be exposed through the same
    `ReleaseFetchContext` without changing the factory signature again.
  - Standalone scheduler deployments pass `None`; the Skills plugin returns a clear error
    when the provider is absent.
  ```

- [ ] **Step 2: Add "Consuming Global Providers in ReleaseFetcher" to plugin guidelines**

  In `docs/development/plugin-guidelines.md`, add a new section after the existing GitHub
  Provider section (or after the last major section if no such section exists):

  ````markdown
  ## Consuming global providers in `ReleaseFetcher`

  `ReleaseFetcher` factory functions receive a third argument — `&ReleaseFetchContext` —
  alongside the config JSON and `HostRuntime`. Existing plugins ignore this context via
  `_ctx: &ReleaseFetchContext`. The `package_manager_skills` plugin is the reference
  implementation for reading from it.

  To access the global GitHub provider in a controller-side `ReleaseFetcher`:

  ```rust
  use uptrakit_global_github_provider::{GitHubProviderHandle, GitHubProviderClient};
  use uptrakit_plugin_infrastructure_core::roles::ReleaseFetchContext;

  fn get_github_provider(ctx: &ReleaseFetchContext) -> Option<Arc<dyn GitHubProviderClient>> {
      #[cfg(feature = "catalog")]
      {
          let lookup = ctx.global_provider_lookup.as_ref()?;
          let handle = lookup.lookup("github")?;
          Arc::downcast::<GitHubProviderHandle>(handle).ok().map(|h| h.client())
      }
      #[cfg(not(feature = "catalog"))]
      { None }
  }
  ```
  ````

  Register the factory via `release_fetcher_create` in `declare_plugin!` instead of listing
  `ReleaseFetcher` in `roles: [...]`:

  ```rust
  declare_plugin!(MyPlugin, MyConfig, "my_plugin_id", {
      // ... other fields ...
      release_fetcher_create: {
          create: my_create_release_fetcher_fn,
          host_requirements: HostRequirements::CONTROLLER_ONLY,
      },
  });
  ```

- [ ] **Step 3: Create the end-user guide**

  Create `docs/end-user/skills-plugin.md`:

  ```markdown
  # Agent Skills Plugin

  The **Agent Skills** plugin discovers, tracks, and updates LLM-agent Skills installed
  globally on a host via `npx skills@latest`.

  ## What gets discovered

  The plugin reads `~/.agents/.skill-lock.json` on each managed host. Every Skill entry with
  `sourceType == "github"` becomes a **Software Item** in Uptrakit. The Skill name (e.g.
  `brainstorming`) is the display identifier; the installed version is the git tree SHA
  recorded as `skillFolderHash`.

  ## GitHub Provider and rate limits

  Release detection (finding whether a newer version of a Skill exists) calls the GitHub
  git-trees API through the instance-wide **GitHub Provider** configured in
  **Settings → GitHub Provider**.

  - **Without a token:** 60 unauthenticated requests per hour. Adequate for small deployments
    with skills concentrated in one or two repositories.
  - **With a token:** 5 000 requests per hour. Recommended for larger or multi-repo deployments.

  The plugin issues **one API call per source repository per refresh cycle** — not one per
  Skill — so Skills from the same repo share a single request.

  ## Update semantics

  Updates run `npx skills@latest update -g <skill-name> -y` on the agent. The `skills` CLI
  does not support version pinning; it always moves the Skill to the current HEAD tree SHA.
  Uptrakit records the requested `to_version` for audit purposes but does not pass it to
  the CLI. The detection cycle reconciles `installed_version` after the update lands.

  ## GitHub-only source restriction

  Only Skills with `sourceType == "github"` are tracked. Skills from other sources (GitLab,
  local paths) are logged as warnings and skipped. This is a known v1 limitation.

  ## Standalone scheduler

  If you run the standalone scheduler (without the embedded controller), the release-fetch
  path is unavailable — the scheduler has no access to the GitHub Provider. Release fetch
  calls return an error logged at `warn`. Discovery and version detection continue to work.

  ## Known limitations

  - **Force-push false positives:** A force-push to the source repo that rewrites history
    without changing file content still changes the git tree SHA, producing a perpetual
    "update available" signal. Running the update is idempotent — `npx skills update`
    re-installs HEAD and records the new SHA.
  - **Skill folder renamed upstream:** If a Skill folder moves in the source repo, release
    fetch returns zero releases. The stored identifier becomes stale; reinstall via the
    `skills` CLI to reconcile.
  ```

- [ ] **Step 4: markdownlint check**

  ```bash
  npx markdownlint --config .markdownlint.json \
    docs/adr/0015-release-fetcher-context.md \
    docs/end-user/skills-plugin.md \
    docs/development/plugin-guidelines.md 2>&1 | head -20
  ```

  Fix any violations (line length > 150, trailing spaces, etc.). Use
  `npx prettier --write <files>` if the project uses prettier for markdown.

- [ ] **Step 5: Commit**

  ```bash
  git add \
    docs/adr/0015-release-fetcher-context.md \
    docs/development/plugin-guidelines.md \
    docs/end-user/skills-plugin.md
  git commit -m "docs: add ADR-0015, plugin-guidelines update, and skills end-user guide"
  ```

---

### Task 12: Quality gate

- [ ] **Step 1: Full fmt + clippy**

  ```bash
  cargo fmt --all
  cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" | head -30
  cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -30
  ```

  Expected: zero errors. Fix any warnings that clippy promotes to errors under
  `[workspace.lints]`.

- [ ] **Step 2: Full test run**

  ```bash
  cargo test --all-features 2>&1 | tail -15
  ```

  Expected: all tests pass.

- [ ] **Step 3: Semantic boundary check (if the CI script exists)**

  ```bash
  python3 ci/check_plugin_semantic_boundary.py 2>&1 | tail -10
  ```

  Expected: no violations reported.

- [ ] **Step 4: cargo deny**

  ```bash
  cargo deny check 2>&1 | tail -10
  ```

  Expected: clean. The `url` crate is already in the workspace, so no new supply-chain
  surface. `uptrakit-global-github-provider` is an internal crate.

- [ ] **Step 5: Commit any fmt diffs**

  ```bash
  git diff --stat
  git add -p  # stage only fmt diffs
  git commit -m "style: cargo fmt after Plan B implementation"
  ```

---

## Self-Review Checklist

- [ ] `PACKAGE_MANAGER_SKILLS` constant exists in `plugin_ids` and is in `ALL`
- [ ] `ALL.len()` assertion updated from 25 to 26
- [ ] `SkillsPlugin` carries `executor` (agent-side) and `Option<provider>` (controller-side)
- [ ] `SkillsPlugin::new` populates `provider: None`; factory sets it from `ReleaseFetchContext`
- [ ] `create_release_fetcher_skills` is a plain `fn` (not a closure), const-compatible
- [ ] `declare_plugin!` uses `release_fetcher_create:` not `ReleaseFetcher` in `roles: [...]`
- [ ] `discover_software` reads lock via `sh -c "cat ~/.agents/.skill-lock.json"` — NOT `tokio::fs`
- [ ] Non-GitHub `sourceType` entries are logged and skipped in `discover_software`
- [ ] `DiscoveryTarget.package_identifier` is `Some(encode_skill_identifier(url, path))`
- [ ] `DiscoveredSoftware.package_identifier` is the Skill name (UI display)
- [ ] `detect_host_compatibility` checks `which npx` (not `which git`)
- [ ] `parse_skill_lock` does not filter by `sourceType` — callers filter
- [ ] `parse_skill_identifier` rejects: no `#`, leading `/` in path, `..`, control chars, > 1024 bytes
- [ ] `releases.rs` uses `PACKAGE_MANAGER_SKILLS` consumer constant — not a `GlobalProviderConsumerDecl`
- [ ] `PluginDescriptor.global_provider_consumers` is `&[]` — NOT listing `"github"` (prevents catalog gating)
- [ ] `batch_fetch` issues one tree call per `(owner, repo)`
- [ ] Truncated tree → structured `warn` log + `Err`
- [ ] Skill folder missing → `Err(PluginError::PluginInternal(...))` with owner/repo/skill_dir in message
- [ ] `execute_update` reads lock file to recover Skill name; passes name to CLI (not version)
- [ ] No `sudo` in `declare_plugin!` (user-owned `~/.agents/`)
- [ ] `all_descriptors()` contains skills descriptor
- [ ] `is_package_manager_plugin` includes `PACKAGE_MANAGER_SKILLS`
- [ ] `every_always_on_plugin_ids` test array updated
- [ ] ADR 0015 committed; `plugin-guidelines.md` updated; end-user doc created
- [ ] All tests pass (`cargo test --all-features`)
- [ ] No clippy errors
