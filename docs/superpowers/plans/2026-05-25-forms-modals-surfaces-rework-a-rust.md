# Forms, Modals & Surfaces Rework — Plan A: Rust & Contract Changes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the Rust-side contract additions (`header_action_ids` on `SurfaceNode::Section`,
`submit_label` on `InteractionDescriptor`) and mechanical UI fixes (SSH Hosts button order, notification
tab grouping, Rust title casing), clearing the compile break so Plan B frontend work can proceed.

**Architecture:** `SurfaceNode` is `#[non_exhaustive]` — adding a field to the `Section` variant
requires constructor functions to avoid breaking all construction sites. The migration is workspace-wide
(~27 sites). `InteractionDescriptor` is a public struct and must also carry `#[non_exhaustive]` per
the coding standard — Task 5 adds the attribute and a `new()` constructor, then migrates the ~80
struct literal sites to use struct-update syntax (`InteractionDescriptor { submit_label, ..InteractionDescriptor::new(...) }`)
rather than adding `submit_label: None` inline. Validation gates for `header_action_ids` live in
both `wire_validate_impls.rs` (wire-level count check) and `surface-proxy/registry.rs`
(semantic interaction-kind check). Tasks 1–2 must be committed before any other crate can compile.

**Tech Stack:** Rust 2024 edition, serde 1 (skip_serializing_if / default), rootcause 0.12
(`report!()` / `bail!()` for error propagation at boundary functions — never `Report::new()`),
thiserror 2 (error enums), uptrakit-surfaces crate, uptrakit-wire crate, uptrakit-surface-proxy
crate, uptrakit-notification-plugin-{webhook,telegram,email}

**Error handling:** Domain-specific error types (`WireValidationError`, `SurfaceProviderRejectionReason`,
`InteractionValidationError`) are accumulated or returned directly — not wrapped in `rootcause::Report`.
`rootcause::Report` wrapping applies at the outermost boundary function (e.g., `register_plugin`). When
adding new error paths that propagate through Report boundaries, use `report!()` / `bail!()`, never
`unwrap()` or `Err("string")`. `impl_report_conversion!` is required for boundary conversion from
thiserror enums to `rootcause::Report` where those boundaries exist.

---

## Tasks

### Task 1: Add `header_action_ids` field and constructors to `SurfaceNode::Section`

**Files:**

- Modify: `crates/shared/surfaces/src/surface.rs`

- [ ] **Step 1: Add `header_action_ids` field to the `Section` variant**

  In `SurfaceNode::Section`, insert the new field between `title` and `children`:

  ```rust
  Section {
      #[serde(default, skip_serializing_if = "Option::is_none")]
      title: Option<String>,
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      header_action_ids: Vec<InteractionId>,  // NEW — defaults to empty vec
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      children: Vec<SurfaceNode>,
  },
  ```

- [ ] **Step 2: Add constructor functions**

  After the `SurfaceNode` enum definition, add an `impl` block:

  ```rust
  impl SurfaceNode {
      /// Creates a `Section` node with no header actions.
      pub fn section(title: impl Into<Option<String>>, children: Vec<SurfaceNode>) -> Self {
          SurfaceNode::Section {
              title: title.into(),
              header_action_ids: vec![],
              children,
          }
      }

      /// Creates a `Section` node with header action buttons.
      ///
      /// Each ID in `header_action_ids` must reference a `Workflow` or `MutationAction`
      /// interaction with `form_ui: None`. Validated at surface registration time.
      pub fn section_with_header_actions(
          title: impl Into<Option<String>>,
          header_action_ids: Vec<InteractionId>,
          children: Vec<SurfaceNode>,
      ) -> Self {
          SurfaceNode::Section {
              title: title.into(),
              header_action_ids,
              children,
          }
      }
  }
  ```

- [ ] **Step 3: Update the `SurfaceDescriptor::builder` doc example**

  The rustdoc example at line ~284 uses a raw struct literal. Update it to use the constructor:

  ```rust
  // Before:
  .root_node(SurfaceNode::Section { title: None, children: vec![] })

  // After:
  .root_node(SurfaceNode::section(None, vec![]))
  ```

- [ ] **Step 4: Update the two struct-literal tests inside `surface.rs`**

  Find the two `SurfaceDescriptor { ... root_node: SurfaceNode::Section { title: None, children: vec![] }, ... }`
  struct literals in the `#[cfg(test)]` module (around lines 610 and 652). Replace each `root_node` value:

  ```rust
  // Before:
  root_node: SurfaceNode::Section {
      title: None,
      children: vec![],
  },

  // After:
  root_node: SurfaceNode::section(None, vec![]),
  ```

- [ ] **Step 5: Also update the doc test in `SurfaceDescriptorBuilder::build` if present**

  Search `crates/shared/surfaces/src/surface.rs` for any remaining `SurfaceNode::Section {` literal:

  ```bash
  grep -n 'SurfaceNode::Section {' crates/shared/surfaces/src/surface.rs
  ```

  Fix any remaining occurrences.

- [ ] **Step 6: Run tests for just this crate to verify it compiles and passes**

  ```bash
  cargo test -p uptrakit-surfaces --all-features 2>&1
  ```

  Expected: all tests pass. The workspace-wide E0063 errors in other crates are expected at this stage and are fixed in Task 2.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/shared/surfaces/src/surface.rs
  git commit -m "feat(surfaces): add header_action_ids to SurfaceNode::Section with constructors

  Adds optional Vec<InteractionId> to Section for surface header action buttons.
  Uses #[serde(default, skip_serializing_if = \"Vec::is_empty\")] for backward
  compatibility. Adds section() and section_with_header_actions() constructors
  since SurfaceNode is #[non_exhaustive] — raw struct literals are anti-pattern.
  Workspace-wide construction site migration follows in next commit.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 2: Migrate all `SurfaceNode::Section {}` struct literal sites (workspace-wide)

**Files:**

- Modify: `crates/shared/wire/src/wire_validate_impls.rs` (2 construction sites + 1 destructuring)
- Modify: `crates/shared/wire/src/tests.rs` (1 construction site)
- Modify: `crates/shared/surfaces/tests/protocol.rs` (1 construction site)
- Modify: `crates/ui/surface-proxy/src/proxy/entity_enrichment.rs` (1 construction site)
- Modify: `crates/ui/cli/src/commands/surfaces.rs` (1 construction site)
- Modify: `crates/ui/cli/tests/command_execution.rs` (2 construction sites)
- Modify: `crates/core/mqtt-runtime/src/surface_runtime.rs` (1 construction site)
- Modify: `crates/core/agent-ssh-runtime/src/surface_runtime.rs` (1 construction + R1 reorder + test updates)
- Modify: `crates/core/agent-ssh-runtime/src/surface_runtime/registration.rs` (1 construction site)
- Modify: `crates/core/agent-ssh-runtime/src/surface_runtime/registration/capabilities.rs` (4 construction sites)
- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs` (5 construction sites)
- Modify: `crates/plugins/notifications/webhook/src/plugin.rs` (1 construction site)
- Modify: `crates/plugins/notifications/email/src/plugin.rs` (2 construction sites)
- Modify: `crates/plugins/notifications/telegram/src/plugin.rs` (1 construction site)

- [ ] **Step 1: Identify all failing construction sites**

  ```bash
  cargo check --all-features 2>&1 | grep "error\[E0063\]" | head -40
  ```

  Expected: ~25 sites across the files listed above. Destructuring patterns (`Section { children, .. }`) do not
  need updating — only construction sites without `..` fail.

- [ ] **Step 2: Apply the constructor pattern to every site**

  The transformation for every `SurfaceNode::Section { title: T, children: C }` construction:

  ```rust
  // Before:
  SurfaceNode::Section {
      title: None,
      children: vec![...],
  }

  // After:
  SurfaceNode::section(None, vec![...])
  ```

  ```rust
  // Before:
  SurfaceNode::Section {
      title: Some("Section Title".to_string()),
      children: vec![...],
  }

  // After:
  SurfaceNode::section(Some("Section Title".to_string()), vec![...])
  ```

  For any site that uses `root_node(SurfaceNode::Section { ... })` in a builder chain:

  ```rust
  // Before:
  .root_node(SurfaceNode::Section { title: None, children: vec![...] })

  // After:
  .root_node(SurfaceNode::section(None, vec![...]))
  ```

- [ ] **Step 3: Apply R1 — reorder SSH Hosts children (ActionBar before Table)**

  In `crates/core/agent-ssh-runtime/src/surface_runtime.rs`, find the `SurfaceNode::Section {}`
  construction at line ~337 and reorder its children so `ActionBar` comes first:

  ```rust
  // Before:
  let root = SurfaceNode::Section {
      title: None,
      children: vec![
          SurfaceNode::Table { data_source_id: data_source_id.clone(), columns: ..., row_actions: row_ids },
          SurfaceNode::ActionBar { action_ids: primary_ids },
      ],
  };

  // After:
  let root = SurfaceNode::section(
      None,
      vec![
          SurfaceNode::ActionBar { action_ids: primary_ids },
          SurfaceNode::Table {
              data_source_id: data_source_id.clone(),
              columns: SSH_HOSTS_COLUMNS
                  .iter()
                  .map(|(key, label)| SurfaceTableColumn::new(*key, *label))
                  .collect(),
              row_actions: row_ids,
          },
      ],
  );
  ```

- [ ] **Step 4: Update the SSH Hosts tests that assert child ordering**

  Four test assertions reference child positions. After the reorder, ActionBar is at index 0 and Table is at index 1.

  In `crates/core/agent-ssh-runtime/src/surface_runtime/tests.rs`:

  ```rust
  // Test 1 (line ~125–132): was asserting children.first() is Table
  // After: assert children.first() is ActionBar, children.get(1) is Table
  let SurfaceNode::Section { children, .. } = &surface.descriptor.root_node else {
      panic!("root node should be a section");
  };
  let Some(SurfaceNode::ActionBar { action_ids }) = children.first() else {
      panic!("first section child should be an action bar");
  };
  // ... existing action_ids assertions ...
  let Some(SurfaceNode::Table { columns, row_actions, .. }) = children.get(1) else {
      panic!("second section child should be a table");
  };
  // ... existing column/row_action assertions ...
  ```

  ```rust
  // Test 2 (line ~162–167): was asserting children.get(1) is ActionBar
  // After: assert children.first() is ActionBar
  let Some(SurfaceNode::ActionBar { action_ids }) = children.first() else {
      panic!("first section child should be an action bar");
  };
  ```

  In `crates/core/agent-ssh-runtime/src/surface_runtime.rs` (inline tests, lines ~2476 and ~2516):
  Apply the same inversions as above (Table at `get(1)`, ActionBar at `first()`).

- [ ] **Step 5: Apply R4-Rust — fix surface node title casing while touching these files**

  Audit every `title: Some("...")` string in surface definitions across these files. Titles must follow title case (capitalize major words, acronyms uppercase).

  Known fixes to apply (verify against actual strings in the files):

  | File                    | Search                | Replace               |
  | ----------------------- | --------------------- | --------------------- |
  | Any surface plugin file | `"Webhook channels"`  | `"Webhook Channels"`  |
  | Any surface plugin file | `"Telegram channels"` | `"Telegram Channels"` |
  | Any surface plugin file | `"Email channels"`    | `"Email Channels"`    |

  Run this audit after updating each file:

  ```bash
  grep -rn 'title: Some' crates/plugins/notifications/ crates/core/agent-ssh-runtime/ | grep -v target
  ```

  Review every match. Acronyms (SSH, SMTP, MCP, OIDC, CA, API) stay uppercase.

- [ ] **Step 6: Verify compilation**

  ```bash
  cargo check --all-features 2>&1 | grep "error\[E0063\]"
  ```

  Expected: empty output (no more struct literal construction errors).

- [ ] **Step 7: Run all tests**

  ```bash
  cargo test --all-features 2>&1 | tail -30
  ```

  Expected: all tests pass.

- [ ] **Step 8: Commit (all migration + behavioral changes in one commit)**

  All changes (constructor migration, R1 reorder, R4-Rust title casing) must be committed together
  because `agent-ssh-runtime/src/surface_runtime.rs` contains both a construction site (migration)
  and the R1 reorder — excluding it from the migration commit would leave the workspace in a
  non-compilable state between commits.

  ```bash
  git commit --only \
    crates/shared/wire/src/wire_validate_impls.rs \
    crates/shared/wire/src/tests.rs \
    crates/shared/surfaces/tests/protocol.rs \
    crates/ui/surface-proxy/src/proxy/entity_enrichment.rs \
    crates/ui/cli/src/commands/surfaces.rs \
    crates/ui/cli/tests/command_execution.rs \
    crates/core/mqtt-runtime/src/surface_runtime.rs \
    crates/core/agent-ssh-runtime/src/surface_runtime.rs \
    crates/core/agent-ssh-runtime/src/surface_runtime/tests.rs \
    crates/core/agent-ssh-runtime/src/surface_runtime/registration.rs \
    crates/core/agent-ssh-runtime/src/surface_runtime/registration/capabilities.rs \
    crates/plugins/infrastructure/proxmox/src/plugin.rs \
    crates/plugins/notifications/webhook/src/plugin.rs \
    crates/plugins/notifications/email/src/plugin.rs \
    crates/plugins/notifications/telegram/src/plugin.rs \
    -m "refactor(surfaces): migrate SurfaceNode::Section literals to constructors; R1+R4-Rust fixes

  Workspace-wide migration from SurfaceNode::Section { title, children } to
  SurfaceNode::section(title, children). Required by #[non_exhaustive] coding
  standard — raw struct literals break on any future field addition.

  Also applies R1 (SSH Hosts ActionBar-before-Table reorder) and R4-Rust (title
  casing on surface section titles) in the same commit since both touch
  agent-ssh-runtime/src/surface_runtime.rs alongside the constructor migration.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 3: Update `wire_validate_impls.rs` Section match arm — bind `header_action_ids` and validate count

**Files:**

- Modify: `crates/shared/wire/src/wire_validate_impls.rs` (around line 390)

The `validate_surface_node` function at line 376 has a match arm for `Section` that destructures
`{ title, children }` — it will fail to compile once the field is added (Task 1 already fixed this for
construction sites, but the destructuring match in `validate_surface_node` needs explicit binding).

- [ ] **Step 1: Update the Section match arm to bind `header_action_ids`**

  ```rust
  // Before (line ~390):
  surfaces::SurfaceNode::Section { title, children } => {
      check_opt_string_len(
          title,
          MAX_SHORT_STRING_LEN,
          "surfaces[].descriptor.root_node.title",
      )?;
      check_vec_len(
          children,
          MAX_SURFACE_FIELDS,
          "surfaces[].descriptor.root_node.children",
      )?;
      for child in children {
          validate_surface_node(child, depth + 1)?;
      }
  }

  // After (Step 1 only — binds header_action_ids, count check added in Step 3b):
  surfaces::SurfaceNode::Section { title, header_action_ids: _, children } => {
      check_opt_string_len(
          title,
          MAX_SHORT_STRING_LEN,
          "surfaces[].descriptor.root_node.title",
      )?;
      check_vec_len(
          children,
          MAX_SURFACE_FIELDS,
          "surfaces[].descriptor.root_node.children",
      )?;
      for child in children {
          validate_surface_node(child, depth + 1)?;
      }
  }
  ```

  Note: `header_action_ids: _` silences the unused-variable warning for now. Step 3b replaces
  `_` with the actual binding and adds the count check after tests are verified to fail.

- [ ] **Step 2: Write the tests (red phase — write before implementation)**

  Add to the `#[cfg(test)]` module in `wire_validate_impls.rs` **before** adding any validation
  logic to the match arm (the match arm was updated in Step 1 to bind `header_action_ids` but
  does not yet enforce the count limit):

  ```rust
  #[test]
  fn section_header_action_ids_count_exceeds_limit_is_rejected() {
      let mut payload = test_surface_registration();
      // Since we can't use struct literal syntax (non_exhaustive), use section_with_header_actions:
      payload.surfaces[0].descriptor.root_node =
          surfaces::SurfaceNode::section_with_header_actions(
              None,
              vec![
                  surfaces::InteractionId::new("a1").unwrap(),
                  surfaces::InteractionId::new("a2").unwrap(),
                  surfaces::InteractionId::new("a3").unwrap(),
                  surfaces::InteractionId::new("a4").unwrap(),
              ],
              vec![],
          );

      let err = payload.wire_validate().unwrap_err();
      assert_eq!(err.field, "surfaces[].descriptor.root_node.header_action_ids");
      assert!(err.message.contains("max 3"));
  }

  #[test]
  fn section_header_action_ids_at_limit_is_accepted() {
      let mut payload = test_surface_registration();
      payload.surfaces[0].descriptor.root_node =
          surfaces::SurfaceNode::section_with_header_actions(
              None,
              vec![
                  surfaces::InteractionId::new("a1").unwrap(),
                  surfaces::InteractionId::new("a2").unwrap(),
                  surfaces::InteractionId::new("a3").unwrap(),
              ],
              vec![],
          );

      assert!(payload.wire_validate().is_ok());
  }
  ```

- [ ] **Step 3: Run tests to verify they fail (red phase)**

  ```bash
  cargo test -p uptrakit-wire section_header_action_ids --all-features 2>&1
  ```

  Expected: `section_header_action_ids_count_exceeds_limit_is_rejected` FAILS (count not yet
  validated), `section_header_action_ids_at_limit_is_accepted` PASSES.

- [ ] **Step 3b: Add the count validation to the match arm (green phase)**

  Inside the `surfaces::SurfaceNode::Section { title, header_action_ids, children } =>` arm added
  in Step 1, insert the count check after the `check_opt_string_len` call:

  ```rust
  if header_action_ids.len() > 3 {
      return Err(WireValidationError {
          field: "surfaces[].descriptor.root_node.header_action_ids",
          message: format!(
              "section header_action_ids has {} entries, max 3",
              header_action_ids.len()
          ),
      });
  }
  ```

- [ ] **Step 3c: Run tests again to verify they pass**

  ```bash
  cargo test -p uptrakit-wire section_header_action_ids --all-features 2>&1
  ```

  Expected: both tests PASS.

- [ ] **Step 4: Run full wire test suite**

  ```bash
  cargo test -p uptrakit-wire --all-features 2>&1
  ```

  Expected: all tests pass.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/wire/src/wire_validate_impls.rs
  git commit -m "test(wire): validate header_action_ids count limit in Section node wire validation

  Binds header_action_ids in SurfaceNode::Section match arm and rejects payloads
  where more than 3 header action IDs are declared. Wire-level count check is the
  first gate; semantic kind validation follows in surface-proxy registry.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 4: Add semantic `header_action_ids` validation in the surface-proxy registry

**Files:**

- Modify: `crates/ui/surface-proxy/src/registry.rs`

The `validate_registration_basics` method already validates per-surface constraints. Add a recursive
helper that validates `header_action_ids` interaction kind and `form_ui` constraints at each `Section`
node throughout the surface tree.

- [ ] **Step 1: Check the `register_plugin` return type before writing tests**

  ```bash
  grep -n "fn register_plugin\|fn register_built_in\|fn validate_registration" \
    crates/ui/surface-proxy/src/registry.rs | head -10
  ```

  If `register_plugin` returns `Result<_, rootcause::Report>`, test assertions must use `.is_err()` +
  inspect the Report string (`.to_string().contains("...")`). If it returns a domain-specific error type,
  inspect fields directly. Match the pattern of existing tests in the same module.

- [ ] **Step 2: Write the failing tests**

  **Before writing tests:** Step 1's grep output tells you the actual registration method name and
  signature. The stubs below use `registry().register_plugin(registration)` as a readable placeholder.
  Adapt the call site to the actual method found by Step 1 (e.g., it may be
  `registry().register_service(service_id, app_name, tenant_id, registration)` or similar).
  Likewise, adapt `make_minimal_plugin_registration_with_root` to the actual registration builder
  or helper in the test module — if no such helper exists, add one by copying the minimal test setup
  from any existing test in `registry.rs`.

  Also note: since `InteractionDescriptor` is now `#[non_exhaustive]` (added in Task 5 Step 1), use
  the `InteractionDescriptor::new(...)` constructor with struct-update syntax instead of raw struct
  literals in the test stubs.

  Add to the existing test module in `registry.rs`:

  ```rust
  #[test]
  fn header_action_ids_unknown_interaction_id_is_rejected() {
      let registration = make_minimal_plugin_registration_with_root(
          surfaces::SurfaceNode::section_with_header_actions(
              None,
              vec![surfaces::InteractionId::new("nonexistent").unwrap()],
              vec![],
          ),
          vec![/* no interactions */],
      );
      // Adapt to actual registration method from Step 1 grep:
      let result = registry().register_plugin(registration);
      assert!(result.is_err(), "unknown header_action_id must be rejected");
  }

  #[test]
  fn header_action_ids_form_submit_kind_is_rejected() {
      let interaction_id = surfaces::InteractionId::new("submit-action").unwrap();
      let registration = make_minimal_plugin_registration_with_root(
          surfaces::SurfaceNode::section_with_header_actions(
              None,
              vec![interaction_id.clone()],
              vec![],
          ),
          vec![surfaces::InteractionDescriptor {
              form_ui: None,
              ..surfaces::InteractionDescriptor::new(
                  interaction_id.clone(),
                  surfaces::InteractionKind::FormSubmit,
                  "Submit",
                  surfaces::InteractionTransport::ProviderProxied,
              )
          }],
      );
      let result = registry().register_plugin(registration);
      assert!(result.is_err(), "FormSubmit kind must be rejected in header_action_ids");
  }

  #[test]
  fn header_action_ids_mutation_action_with_form_ui_is_rejected() {
      let interaction_id = surfaces::InteractionId::new("form-action").unwrap();
      let registration = make_minimal_plugin_registration_with_root(
          surfaces::SurfaceNode::section_with_header_actions(
              None,
              vec![interaction_id.clone()],
              vec![],
          ),
          vec![surfaces::InteractionDescriptor {
              form_ui: Some(surfaces::FormUiDescriptor { fields: vec![], pre_load_interaction_id: None }),
              ..surfaces::InteractionDescriptor::new(
                  interaction_id.clone(),
                  surfaces::InteractionKind::MutationAction,
                  "Save",
                  surfaces::InteractionTransport::ProviderProxied,
              )
          }],
      );
      let result = registry().register_plugin(registration);
      assert!(result.is_err(), "MutationAction with form_ui must be rejected in header_action_ids");
  }

  #[test]
  fn header_action_ids_mutation_action_without_form_ui_is_accepted() {
      let interaction_id = surfaces::InteractionId::new("btn-action").unwrap();
      let registration = make_minimal_plugin_registration_with_root(
          surfaces::SurfaceNode::section_with_header_actions(
              None,
              vec![interaction_id.clone()],
              vec![],
          ),
          vec![surfaces::InteractionDescriptor::new(
              interaction_id.clone(),
              surfaces::InteractionKind::MutationAction,
              "Refresh",
              surfaces::InteractionTransport::ProviderProxied,
          )],
      );
      let result = registry().register_plugin(registration);
      assert!(result.is_ok(), "MutationAction without form_ui must be accepted");
  }
  ```

- [ ] **Step 2: Run the tests to verify they fail**

  ```bash
  cargo test -p uptrakit-surface-proxy header_action_ids --all-features 2>&1
  ```

  Expected: all 4 tests FAIL (validation not yet implemented).

- [ ] **Step 3: Add the validation helper function**

  Add this function before `validate_registration_basics`:

  ```rust
  fn validate_section_header_actions(
      node: &surfaces::SurfaceNode,
      interactions: &[surfaces::InteractionDescriptor],
      surface_id: Option<String>,
      reasons: &mut Vec<SurfaceProviderRejectionReason>,
  ) {
      match node {
          surfaces::SurfaceNode::Section {
              header_action_ids,
              children,
              ..
          } => {
              // Count check here covers built-in/plugin providers that register programmatically
              // and bypass the wire-layer validation path. The wire-layer check (Task 3) is an
              // additional early rejection for service providers; this is the authoritative gate.
              if header_action_ids.len() > 3 {
                  reasons.push(SurfaceProviderRejectionReason {
                      code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                      message: format!(
                          "section header_action_ids has {} entries, max 3",
                          header_action_ids.len()
                      ),
                      surface_id: surface_id.clone(),
                  });
              }
              for action_id in header_action_ids {
                  match interactions.iter().find(|i| &i.interaction_id == action_id) {
                      None => {
                          reasons.push(SurfaceProviderRejectionReason {
                              code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                              message: format!(
                                  "section header_action_ids references unknown interaction `{action_id}`"
                              ),
                              surface_id: surface_id.clone(),
                          });
                      }
                      Some(interaction) => {
                          let valid_kind = matches!(
                              interaction.kind,
                              surfaces::InteractionKind::Workflow
                                  | surfaces::InteractionKind::MutationAction
                          );
                          if !valid_kind {
                              reasons.push(SurfaceProviderRejectionReason {
                                  code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                                  message: format!(
                                      "interaction `{action_id}` in header_action_ids must be \
                                       kind Workflow or MutationAction (got {:?})",
                                      interaction.kind
                                  ),
                                  surface_id: surface_id.clone(),
                              });
                          }
                          if interaction.form_ui.is_some() {
                              reasons.push(SurfaceProviderRejectionReason {
                                  code: SurfaceProviderRejectionCode::SchemaOrLimitFailure,
                                  message: format!(
                                      "interaction `{action_id}` in header_action_ids must not \
                                       have form_ui set"
                                  ),
                                  surface_id: surface_id.clone(),
                              });
                          }
                      }
                  }
              }
              for child in children {
                  validate_section_header_actions(child, interactions, surface_id.clone(), reasons);
              }
          }
          surfaces::SurfaceNode::Tabs { tabs } => {
              for tab in tabs {
                  validate_section_header_actions(
                      &tab.root,
                      interactions,
                      surface_id.clone(),
                      reasons,
                  );
              }
          }
          surfaces::SurfaceNode::ModalTrigger { modal_nodes, .. } => {
              for node in modal_nodes {
                  validate_section_header_actions(node, interactions, surface_id.clone(), reasons);
              }
          }
          surfaces::SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
              for node in step_nodes {
                  validate_section_header_actions(node, interactions, surface_id.clone(), reasons);
              }
          }
          _ => {}
      }
  }
  ```

- [ ] **Step 4: Call the helper in `validate_registration_basics`**

  Find the `for surface in &registration.surfaces { ... }` loop in `validate_registration_basics`
  (line ~648 in `registry.rs`). Anywhere inside that loop body — after any existing per-surface
  validation calls — add:

  ```rust
  validate_section_header_actions(
      &surface.descriptor.root_node,
      &surface.interactions,
      Some(surface.descriptor.surface_id.to_string()),
      &mut reasons,
  );
  ```

  `validate_surface_node` is a wire-level function in `wire_validate_impls.rs` (not called here);
  this step operates on the registry's own validation loop.

- [ ] **Step 5: Run the tests to verify they pass**

  ```bash
  cargo test -p uptrakit-surface-proxy header_action_ids --all-features 2>&1
  ```

  Expected: all 4 tests PASS.

- [ ] **Step 6: Run the full surface-proxy test suite**

  ```bash
  cargo test -p uptrakit-surface-proxy --all-features 2>&1
  ```

  Expected: all tests pass.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/ui/surface-proxy/src/registry.rs
  git commit -m "feat(surface-proxy): validate header_action_ids kind and form_ui at registration

  Adds validate_section_header_actions() helper that walks the surface node tree and
  rejects any Section whose header_action_ids reference: unknown interactions, non-
  Workflow/MutationAction kinds, or MutationAction interactions with form_ui set.
  Called from validate_registration_basics() alongside existing per-surface checks.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 5: Add `submit_label` field to `InteractionDescriptor` and migrate struct literal sites

**Files:**

- Modify: `crates/shared/surfaces/src/interaction.rs`
- Modify: `crates/shared/wire/src/wire_validate_impls.rs`
- Modify: `frontend/src/lib/surfaces/contract.ts`
- Modify: ~80 Rust files containing `InteractionDescriptor { ... }` struct literals

- [ ] **Step 1: Add `#[non_exhaustive]`, `submit_label` field, and `new()` constructor to `InteractionDescriptor`**

  In `crates/shared/surfaces/src/interaction.rs`, apply three changes:

  **1a. Add `#[non_exhaustive]` attribute** (coding standard: all public structs must carry it):

  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[non_exhaustive]
  pub struct InteractionDescriptor {
      pub interaction_id: InteractionId,
      pub kind: InteractionKind,
      pub label: String,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub required_permission: Option<String>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub input_schema: Option<SchemaContract>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub result_schema: Option<SchemaContract>,
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      pub sensitive_fields: Vec<String>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub timeout_seconds: Option<u16>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub confirmation: Option<InteractionConfirmation>,
      pub transport: InteractionTransport,
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      pub workflow_steps: Vec<WorkflowStepDescriptor>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub form_ui: Option<FormUiDescriptor>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub icon: Option<String>,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub submit_label: Option<String>,   // NEW — overrides default "Save" label for form_submit interactions
  }
  ```

  **1b. Add a `new()` constructor** in the existing `impl InteractionDescriptor` block (or add one):

  ```rust
  impl InteractionDescriptor {
      pub fn new(
          interaction_id: InteractionId,
          kind: InteractionKind,
          label: impl Into<String>,
          transport: InteractionTransport,
      ) -> Self {
          Self {
              interaction_id,
              kind,
              label: label.into(),
              transport,
              required_permission: None,
              input_schema: None,
              result_schema: None,
              sensitive_fields: vec![],
              timeout_seconds: None,
              confirmation: None,
              workflow_steps: vec![],
              form_ui: None,
              icon: None,
              submit_label: None,
          }
      }
  }
  ```

- [ ] **Step 2: Write a failing test for submit_label validation**

  Add to the `#[cfg(test)]` module in `interaction.rs` (use struct-update syntax since
  `InteractionDescriptor` is now `#[non_exhaustive]`):

  ```rust
  #[test]
  fn validate_for_provider_rejects_empty_submit_label() {
      let descriptor = InteractionDescriptor {
          submit_label: Some("   ".to_string()),
          ..InteractionDescriptor::new(
              InteractionId::new("act").unwrap(),
              InteractionKind::FormSubmit,
              "Save Settings",
              InteractionTransport::ProviderProxied,
          )
      };
      let err = descriptor
          .validate_for_provider(ProviderKind::Plugin)
          .unwrap_err();
      assert!(matches!(err, InteractionValidationError::SubmitLabelInvalid { .. }));
  }

  #[test]
  fn validate_for_provider_rejects_submit_label_exceeding_50_chars() {
      let descriptor = InteractionDescriptor {
          submit_label: Some("a".repeat(51)),
          ..InteractionDescriptor::new(
              InteractionId::new("act").unwrap(),
              InteractionKind::FormSubmit,
              "Save",
              InteractionTransport::ProviderProxied,
          )
      };
      let err = descriptor
          .validate_for_provider(ProviderKind::Plugin)
          .unwrap_err();
      assert!(matches!(err, InteractionValidationError::SubmitLabelInvalid { .. }));
  }

  #[test]
  fn validate_for_provider_accepts_valid_submit_label() {
      let descriptor = InteractionDescriptor {
          submit_label: Some("Connect".to_string()),
          ..InteractionDescriptor::new(
              InteractionId::new("act").unwrap(),
              InteractionKind::FormSubmit,
              "Save",
              InteractionTransport::ProviderProxied,
          )
      };
      assert!(descriptor
          .validate_for_provider(ProviderKind::Plugin)
          .is_ok());
  }
  ```

- [ ] **Step 2b: Run tests to verify they fail (red phase)**

  ```bash
  cargo test -p uptrakit-surfaces validate_for_provider_rejects --all-features 2>&1
  ```

  Expected: tests fail to compile or panic — `SubmitLabelInvalid` variant does not exist yet and
  validation logic is absent. Confirm the red state before proceeding.

- [ ] **Step 3: Add the `SubmitLabelInvalid` error variant to `InteractionValidationError`**

  ```rust
  #[error("interaction `{interaction_id}` has invalid submit_label: {reason}")]
  SubmitLabelInvalid {
      interaction_id: InteractionId,
      reason: String,
  },
  ```

- [ ] **Step 4: Add validation in `validate_for_provider`**

  Add after the `icon` validation block:

  ```rust
  if let Some(submit_label) = &self.submit_label {
      if submit_label.trim().is_empty() {
          return Err(InteractionValidationError::SubmitLabelInvalid {
              interaction_id: self.interaction_id.clone(),
              reason: "must not be empty".to_string(),
          });
      }
      if submit_label.len() > 50 {
          return Err(InteractionValidationError::SubmitLabelInvalid {
              interaction_id: self.interaction_id.clone(),
              reason: format!("exceeds max 50 characters ({} given)", submit_label.len()),
          });
      }
  }
  ```

- [ ] **Step 5: Run the `uptrakit-surfaces` tests to verify the new tests pass**

  ```bash
  cargo test -p uptrakit-surfaces --all-features 2>&1
  ```

  Expected: new tests pass; existing tests will FAIL with E0063 because all 80 struct literal sites in the
  test module also need `submit_label: None`. Fix them in Step 6.

- [ ] **Step 6: Migrate all `InteractionDescriptor` struct literal sites across the workspace**

  Because `#[non_exhaustive]` is added in Step 1, all raw struct literals now fail to compile.
  The migration uses the constructor + struct-update syntax instead of bare field addition.

  Find every failing site:

  ```bash
  cargo check --all-features 2>&1 | grep "error\[E0063\]\|error\[E0639\]" | grep "InteractionDescriptor"
  ```

  **Important: `#[non_exhaustive]` only restricts EXTERNAL crates.** Struct literals inside
  `crates/shared/surfaces/` (the defining crate) remain valid and will NOT appear in E0639 output.
  Those sites can optionally be converted to use the constructor for consistency, but this is
  voluntary — do not include them in the compile-driven migration loop below.

  **Pattern A — all fields were defaults (the common case):** replace the entire struct literal
  with `InteractionDescriptor::new(...)`:

  ```rust
  // Before:
  InteractionDescriptor {
      interaction_id: InteractionId::new("refresh").unwrap(),
      kind: InteractionKind::MutationAction,
      label: "Refresh".to_string(),
      required_permission: None,
      input_schema: None,
      result_schema: None,
      sensitive_fields: vec![],
      timeout_seconds: None,
      confirmation: None,
      transport: InteractionTransport::ProviderProxied,
      workflow_steps: vec![],
      form_ui: None,
      icon: None,
  }

  // After:
  InteractionDescriptor::new(
      InteractionId::new("refresh").unwrap(),
      InteractionKind::MutationAction,
      "Refresh",
      InteractionTransport::ProviderProxied,
  )
  ```

  **Pattern B — one or more non-default fields:** use struct-update syntax:

  ```rust
  // Before (has icon, confirmation, or other non-default fields):
  InteractionDescriptor {
      interaction_id: InteractionId::new("add-provider").unwrap(),
      kind: InteractionKind::MutationAction,
      label: "Add Provider".to_string(),
      required_permission: None,
      input_schema: None,
      result_schema: None,
      sensitive_fields: vec![],
      timeout_seconds: None,
      confirmation: Some(InteractionConfirmation { ... }),
      transport: InteractionTransport::ControllerLocal,
      workflow_steps: vec![],
      form_ui: None,
      icon: Some("plus".to_string()),
  }

  // After:
  InteractionDescriptor {
      icon: Some("plus".to_string()),
      confirmation: Some(InteractionConfirmation { ... }),
      ..InteractionDescriptor::new(
          InteractionId::new("add-provider").unwrap(),
          InteractionKind::MutationAction,
          "Add Provider",
          InteractionTransport::ControllerLocal,
      )
  }
  ```

  After patching all files, verify:

  ```bash
  cargo check --all-features 2>&1 | grep "error\[E0063\]\|error\[E0639\]" | grep "InteractionDescriptor"
  ```

  Expected: empty output.

- [ ] **Step 7: Add `submit_label` to TypeScript `InteractionDescriptor` in contract.ts**

  In `frontend/src/lib/surfaces/contract.ts`, add the optional field to `InteractionDescriptor`:

  ```typescript
  export interface InteractionDescriptor {
    interaction_id: InteractionId;
    kind: InteractionKind;
    label: string;
    icon?: string;
    required_permission?: string;
    input_schema?: SchemaContract;
    result_schema?: SchemaContract;
    sensitive_fields?: string[];
    timeout_seconds?: number;
    confirmation?: InteractionConfirmation;
    transport: InteractionTransport;
    workflow_steps?: WorkflowStepDescriptor[];
    form_ui?: FormUiDescriptor;
    submit_label?: string; // NEW — overrides default "Save" label for form_submit interactions
  }
  ```

- [ ] **Step 8: Also add `header_action_ids` to the TypeScript `section` node type**

  In `frontend/src/lib/surfaces/contract.ts`, update the `section` variant in `SurfaceNode`:

  ```typescript
  // Before:
  | {
      kind: 'section';
      title?: string;
      children?: SurfaceNode[];
    }

  // After:
  | {
      kind: 'section';
      title?: string;
      header_action_ids?: InteractionId[];
      children?: SurfaceNode[];
    }
  ```

- [ ] **Step 9: Verify TypeScript compilation**

  ```bash
  cd frontend && npm run check 2>&1
  ```

  Expected: no type errors.

- [ ] **Step 10: Run full workspace tests**

  ```bash
  cargo test --all-features 2>&1 | tail -20
  ```

  Expected: all tests pass.

- [ ] **Step 11: Commit**

  ```bash
  git commit --only crates/shared/surfaces/src/interaction.rs \
                    frontend/src/lib/surfaces/contract.ts \
                    -m "feat(surfaces): add submit_label field to InteractionDescriptor

  Adds optional submit_label to override the default 'Save' button label for
  form_submit interactions. Validated at registration: empty string rejected,
  >50 chars rejected. Matching TypeScript contract update in contract.ts.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

  Then commit the struct literal migrations separately:

  ```bash
  git add crates/
  git commit -m "chore(surfaces): migrate InteractionDescriptor struct literals to constructor pattern

  Mechanical migration required by adding #[non_exhaustive] to InteractionDescriptor.
  All-default sites use InteractionDescriptor::new(...); non-default sites use struct
  update syntax (InteractionDescriptor { field, ..InteractionDescriptor::new(...) }).

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 6: R8 — Notification channels tab grouping

**Files:**

- Modify: `crates/plugins/notifications/webhook/src/plugin.rs`
- Modify: `crates/plugins/notifications/telegram/src/plugin.rs`
- Modify: `crates/plugins/notifications/email/src/plugin.rs`

- [ ] **Step 0: Verify `tab_group()` builder method exists**

  ```bash
  grep -n "fn tab_group" crates/shared/surfaces/src/surface.rs
  ```

  Expected: one match showing `pub fn tab_group(...)` on `SurfaceDescriptorBuilder`. If absent,
  you must add the builder method to `SurfaceDescriptorBuilder` in `surface.rs` first (store
  `tab_group: Option<String>` and `tab_group_label: Option<String>` fields on the builder,
  propagate to `SurfaceDescriptor`). Then continue with Step 1.

- [ ] **Step 1: Add `.tab_group(...)` to `notifications.webhook` surface builder**

  In `webhook/src/plugin.rs`, find the `SurfaceDescriptor::builder()` chain for `notifications.webhook`
  (around line 331). Add `.tab_group("notification-channels", "Notification Channels")` before `.build()`:

  ```rust
  SurfaceDescriptor::builder()
      .surface_id(surfaces::SurfaceId::new("notifications.webhook").unwrap())
      // ... existing builder calls ...
      .tab_group("notification-channels", "Notification Channels")
      .build()
  ```

- [ ] **Step 2: Add `.tab_group(...)` to `notifications.telegram` surface builder (not the global settings surface)**

  In `telegram/src/plugin.rs`, there are TWO surfaces:
  - `notifications.telegram` (line ~301) — add `tab_group`
  - `notifications.telegram.global_settings` (line ~609) — do NOT add `tab_group`

  Apply `tab_group` only to `notifications.telegram`:

  ```rust
  surfaces::SurfaceDescriptor::builder()
      .surface_id(surfaces::SurfaceId::new("notifications.telegram").unwrap())
      // ... existing builder calls ...
      .tab_group("notification-channels", "Notification Channels")
      .build()
  ```

- [ ] **Step 3: Add `.tab_group(...)` to `notifications.email` surface builder (not the global SMTP surface)**

  In `email/src/plugin.rs`, there are TWO surfaces:
  - `notifications.email` (line ~594) — add `tab_group`
  - `notifications.email.global_smtp` (line ~1028) — do NOT add `tab_group`

  Apply only to `notifications.email`.

- [ ] **Step 4: Verify `BUILTIN_TAB_IDS` in settings page does NOT include `"notification-channels"`**

  ```bash
  grep -n "BUILTIN_TAB_IDS\|notification-channels" frontend/src/routes/settings/+page.svelte
  ```

  Expected: `BUILTIN_TAB_IDS` does not contain `"notification-channels"` — it's surface-provided, not built-in. No change needed.

- [ ] **Step 5: Run notification plugin tests**

  ```bash
  cargo test -p uptrakit-notification-plugin-webhook --all-features 2>&1
  cargo test -p uptrakit-notification-plugin-telegram --all-features 2>&1
  cargo test -p uptrakit-notification-plugin-email --all-features 2>&1
  ```

  Expected: all tests pass.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/plugins/notifications/webhook/src/plugin.rs \
          crates/plugins/notifications/telegram/src/plugin.rs \
          crates/plugins/notifications/email/src/plugin.rs
  git commit -m "feat(notifications): group webhook/telegram/email surfaces under Notification Channels tab

  Sets tab_group('notification-channels', 'Notification Channels') on the three
  channel-list surfaces. The global settings surfaces (telegram.global_settings,
  email.global_smtp) remain outside the tab group. The frontend already handles
  tab_group grouping; no frontend changes needed.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 7: Update `docs/development/ui/surfaces.md` with Rust-side contract additions

**Files:**

- Modify: `docs/development/ui/surfaces.md`

The spec marks these documentation deliverables as non-deferred. Plan A introduces Rust contract
changes visible to plugin authors (new `header_action_ids` field, constructors, `submit_label`
field, ActionBar ordering rule). These must be documented before Plan B frontend work lands.

- [ ] **Step 1: Update the Surface Primitives table — `Section` row**

  Add to the `Section` primitive row:

  > `header_action_ids` accepts an array of interaction IDs (max 3). Each referenced interaction
  > must be kind `Workflow` or `MutationAction` (with `form_ui: None`). The host renders them as
  > buttons in the card header. `ActionBar` and `FormSubmit` interactions must not appear here.
  > Validation enforced at surface registration.

- [ ] **Step 2: Update the Surface Primitives table — `ActionBar` row**

  Add to the `ActionBar` row:

  > Must appear before any `Table` sibling in the `Section` children list — buttons display above
  > the data table. See the SSH Hosts surface for the canonical ordering example.

- [ ] **Step 3: Add a Section Layout Rules section**

  Add a new `## Section Layout Rules` (or equivalent heading level) section documenting:
  - ActionBar-before-Table ordering rule with correct/incorrect Rust examples using the new
    constructor: `SurfaceNode::section(None, vec![SurfaceNode::ActionBar { ... }, SurfaceNode::Table { ... }])`
  - `#[non_exhaustive]` constructor requirement: always use `SurfaceNode::section()` or
    `SurfaceNode::section_with_header_actions()`, never raw struct literals

- [ ] **Step 4: Add a Section Header Actions section**

  Add a new section documenting `header_action_ids`:
  - Valid kinds: `Workflow`, `MutationAction` with `form_ui: None`
  - Max count: 3
  - Rust constructor usage: `SurfaceNode::section_with_header_actions(title, ids, children)`
  - Registration validation: count check at wire level, kind + form_ui check at surface-proxy
  - TypeScript shape: `header_action_ids?: InteractionId[]` in the `section` node variant

- [ ] **Step 5: Update the `Form` primitive row — note `submit_label`**

  Add to the `Form` / `InteractionDescriptor` row:

  > `submit_label: Option<String>` overrides the default "Save" button label. Validated at
  > registration: empty string and strings longer than 50 chars are rejected. TypeScript:
  > `submit_label?: string` on `InteractionDescriptor`.

- [ ] **Step 6: Run markdownlint**

  ```bash
  npx markdownlint --config .markdownlint.json 'docs/development/ui/surfaces.md' 2>&1
  ```

  Expected: no errors.

- [ ] **Step 7: Commit**

  ```bash
  git commit --only docs/development/ui/surfaces.md \
    -m "docs(surfaces): document header_action_ids, submit_label, and ActionBar ordering rules

  Updates surfaces.md with Rust-side contract additions from the forms/modals rework:
  - Section header_action_ids field (constructor usage, valid kinds, max 3, registration validation)
  - ActionBar-before-Table ordering rule with correct/incorrect examples
  - submit_label field on InteractionDescriptor (validation rules, TypeScript shape)

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

### Task 8: Final quality gates for Plan A

- [ ] **Step 1: Full workspace build**

  ```bash
  cargo build --all-features 2>&1
  ```

  Expected: builds clean with 0 errors.

- [ ] **Step 2: Full workspace test suite**

  ```bash
  cargo test --all-features 2>&1 | grep -E "^test|FAILED|error" | tail -40
  ```

  Expected: all tests pass.

- [ ] **Step 3: Clippy**

  ```bash
  cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 4: Frontend type-check**

  ```bash
  cd frontend && npm run check 2>&1
  ```

  Expected: no type errors.

- [ ] **Step 5: Markdownlint (if any docs were touched)**

  ```bash
  npx markdownlint --config .markdownlint.json 'docs/**/*.md' 2>&1
  ```

  Expected: no errors.
