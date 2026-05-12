# Semantic Audit Logs V2 — Plan C: Catalog and CI Gate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `audit-catalog.toml` file plus the `audit-coverage-check` binary (a `syn`-based Rust source walker), seed it with
the full set of mutation sites in the workspace, wire it into CI, and add the complementary "every Stateful action has ≥1 emit
call site" check.

**Architecture:** A single-binary checker at `crates/shared/audit-log/tools/audit-coverage-check/` walks the workspace tree, identifies
state-changing sites by AST shape (Axum POST/PUT/PATCH/DELETE handlers, scheduler executor `run()` impls, named runtime mutation
entry points, and functions annotated `#[audit_required]`), and matches each against entries in `audit-catalog.toml`. Missing entry →
non-zero exit with an offending-site list. A second pass scans the workspace for `AuditEntry::<verb>(` constructor calls and ensures
every registered Stateful action has at least one. Both passes run as one CI job.

**Tech Stack:** Rust, `syn` v2, `walkdir`, `toml`, `serde`, `proc-macro2`. Source of truth: spec §"Catalog and static-analysis CI gate".

**Quality gates:** `cargo fmt --all`, `cargo check --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features`, `cargo run -p audit-coverage-check`, `cargo deny check`, `markdownlint --config .markdownlint.json
'**/*.md'`.

---

## File structure

| File                                                                      | Status | Responsibility                                                          |
| ------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------- |
| `crates/shared/audit-log/tools/audit-coverage-check/Cargo.toml`           | create | Binary crate manifest                                                   |
| `crates/shared/audit-log/tools/audit-coverage-check/src/main.rs`          | create | CLI entry, exit-code orchestration                                      |
| `crates/shared/audit-log/tools/audit-coverage-check/src/catalog.rs`       | create | TOML parsing of `audit-catalog.toml`                                    |
| `crates/shared/audit-log/tools/audit-coverage-check/src/walker.rs`        | create | `syn`-based AST traversal to identify mutation sites                    |
| `crates/shared/audit-log/tools/audit-coverage-check/src/registry.rs`      | create | Reflect the action registry (parse `action_type.rs` and read constants) |
| `crates/shared/audit-log/tools/audit-coverage-check/src/emit_sweep.rs`    | create | Constructor-call sweep (every Stateful action ≥1 emit)                  |
| `crates/shared/audit-log/tools/audit-coverage-check/tests/fixtures/...`   | create | Fixture crates: one with a missing-catalog handler, one clean           |
| `crates/shared/audit-log/tools/audit-coverage-check/tests/integration.rs` | create | Run checker against fixtures; assert exit codes                         |
| `crates/shared/audit-log/audit-catalog.toml`                              | create | Source-of-truth catalog (initial sweep)                                 |
| `crates/shared/audit-log/Cargo.toml`                                      | modify | Re-export `#[audit_required]` attribute macro placeholder               |
| `Cargo.toml` (workspace)                                                  | modify | Add `walkdir`, `toml` to `[workspace.dependencies]`                     |
| `.github/workflows/ci.yml` (or local CI runner script)                    | modify | Run `cargo run -p audit-coverage-check` in the lint job                 |

---

## Task 1: Branch

- [ ] `git checkout -b feat/audit-v2-catalog-ci-gate` from the head of Plan B's branch.

---

## Task 2: Add workspace dependencies

**Files:** `Cargo.toml`

- [ ] **Step 1:** In `[workspace.dependencies]`, append:

  ```toml
  walkdir = "2"
  toml = "0.8"
  ```

- [ ] **Step 2:** `cargo check --workspace` — Expected: success.

- [ ] **Step 3:** Commit:

  ```bash
  git add Cargo.toml Cargo.lock
  git commit -m "build(audit-v2): add walkdir and toml workspace deps for coverage checker"
  ```

---

## Task 3: Scaffold the checker binary

**Files:**

- Create: `crates/shared/audit-log/tools/audit-coverage-check/Cargo.toml`
- Create: `crates/shared/audit-log/tools/audit-coverage-check/src/main.rs`
- Modify: root `Cargo.toml` (add to `[workspace] members`)

- [ ] **Step 1:** Append `"crates/shared/audit-log/tools/audit-coverage-check"` to `members`.

- [ ] **Step 2:** Write the manifest:

  ```toml
  [package]
  name = "audit-coverage-check"
  description = "Static-analysis gate ensuring every state-changing site has an audit-catalog decision"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"

  [[bin]]
  name = "audit-coverage-check"
  path = "src/main.rs"

  [dependencies]
  syn = { workspace = true }
  proc-macro2 = { workspace = true }
  walkdir = { workspace = true }
  toml = { workspace = true }
  serde = { workspace = true, features = ["derive"] }

  [lints]
  workspace = true
  ```

- [ ] **Step 3:** Write the entry point:

  ```rust
  use std::process::ExitCode;

  mod catalog;
  mod walker;
  mod registry;
  mod emit_sweep;

  fn main() -> ExitCode {
      let workspace_root = std::env::current_dir().expect("cwd");
      let catalog = match catalog::load(&workspace_root.join("crates/shared/audit-log/audit-catalog.toml")) {
          Ok(c) => c,
          Err(e) => { eprintln!("failed to read catalog: {e}"); return ExitCode::from(2); }
      };
      let registry = match registry::load(&workspace_root.join("crates/shared/audit-log/src/action_type.rs")) {
          Ok(r) => r,
          Err(e) => { eprintln!("failed to read registry: {e}"); return ExitCode::from(2); }
      };

      let mut failed = false;

      match walker::scan(&workspace_root, &catalog, &registry) {
          Ok(report) => {
              for s in &report.missing_catalog_entry { eprintln!("missing catalog entry: {s}"); failed |= true; }
              for s in &report.unknown_action { eprintln!("catalog action not registered: {s}"); failed |= true; }
              for s in &report.stale_skip { eprintln!("stale catalog skip (site not found): {s}"); failed |= true; }
          }
          Err(e) => { eprintln!("walker failed: {e}"); return ExitCode::from(2); }
      }

      match emit_sweep::scan(&workspace_root, &registry) {
          Ok(report) => {
              for a in &report.stateful_actions_without_emit_site {
                  eprintln!("registered Stateful action with no emit call site: {a}");
                  failed |= true;
              }
          }
          Err(e) => { eprintln!("emit sweep failed: {e}"); return ExitCode::from(2); }
      }

      if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
  }
  ```

- [ ] **Step 4:** Stub the four modules so the binary builds (each is a `pub fn` returning an empty report `Result`):
  - `catalog::load(path) -> Result<Catalog, String>` returning a struct with empty fields.
  - `registry::load(path) -> Result<Registry, String>` returning empty.
  - `walker::scan(root, catalog, registry) -> Result<WalkReport, String>` returning empty.
  - `emit_sweep::scan(root, registry) -> Result<EmitReport, String>` returning empty.

  Each stub returns `Ok` with empty data so the binary compiles and exits 0 against an empty workspace.

- [ ] **Step 5:** `cargo build -p audit-coverage-check` — Expected: success.

- [ ] **Step 6:** Commit:

  ```bash
  git add Cargo.toml crates/shared/audit-log/tools/audit-coverage-check/
  git commit -m "feat(audit-v2): scaffold audit-coverage-check binary"
  ```

---

## Task 4: Catalog parsing

**Files:**

- Modify: `crates/shared/audit-log/tools/audit-coverage-check/src/catalog.rs`
- Create: `crates/shared/audit-log/audit-catalog.toml` (start with one fixture-only entry; full sweep lands in Task 8)

- [ ] **Step 1: Catalog schema**

  ```rust
  use serde::Deserialize;

  #[derive(Deserialize, Debug)]
  pub struct Catalog {
      pub entries: Vec<Entry>,
  }

  #[derive(Deserialize, Debug)]
  pub struct Entry {
      /// Fully-qualified path to the site, e.g. "uptrakit_web_api::routes::plugin_configs::create".
      pub site: String,
      /// One of `action` (audited) or `skip` (intentionally not audited).
      pub action: Option<String>,
      pub skip: Option<String>,
  }

  pub fn load(path: &std::path::Path) -> Result<Catalog, String> {
      let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
      let catalog: Catalog = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
      for e in &catalog.entries {
          if e.action.is_some() == e.skip.is_some() {
              return Err(format!("catalog entry {}: must set exactly one of `action` or `skip`", e.site));
          }
      }
      Ok(catalog)
  }
  ```

- [ ] **Step 2: Seed catalog with the fixture entry only (full sweep in Task 8)**

  ```toml
  # Audit coverage catalog. Every state-changing site lands here with either an `action`
  # reference (audited via the named action) or a `skip = "<reason>"` (intentionally not audited).

  [[entries]]
  site = "uptrakit_web_api::routes::host_status::record_heartbeat"
  skip = "heartbeat denormalization; covered by transport access log, no security event"
  ```

- [ ] **Step 3: Write a unit test for catalog parsing**

  Add `crates/shared/audit-log/tools/audit-coverage-check/tests/catalog_parse.rs`:

  ```rust
  use audit_coverage_check::catalog;

  #[test]
  fn catalog_round_trip() {
      let tmp = tempfile::NamedTempFile::new().unwrap();
      std::fs::write(tmp.path(), r#"
          [[entries]]
          site = "x::y"
          action = "auth.login"

          [[entries]]
          site = "x::z"
          skip = "covered by access log"
      "#).unwrap();
      let cat = catalog::load(tmp.path()).expect("parse");
      assert_eq!(cat.entries.len(), 2);
  }
  ```

  Requires exposing `catalog` from `main.rs` — for testability, split the binary into `lib.rs` + `bin/audit-coverage-check.rs`. Adjust
  `Cargo.toml` accordingly.

- [ ] **Step 4:** `cargo test -p audit-coverage-check` — Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/audit-log/tools/audit-coverage-check/ crates/shared/audit-log/audit-catalog.toml
  git commit -m "feat(audit-v2): coverage-check catalog parsing"
  ```

---

## Task 5: Registry reflection

**Files:**

- Modify: `crates/shared/audit-log/tools/audit-coverage-check/src/registry.rs`

The registry parser reads `crates/shared/audit-log/src/action_type.rs` as a `syn::File` and extracts every
`pub const NAME: RegisteredAuditAction = RegisteredAuditAction::new("<value>", AuditActionKind::<kind>);`.

- [ ] **Step 1: Implement parser**

  ```rust
  use std::collections::HashMap;
  use std::path::Path;
  use syn::{Expr, ExprCall, ExprLit, ExprPath, ExprStruct, Item, Lit, Meta};

  #[derive(Debug)]
  pub struct Registry {
      pub actions: HashMap<String, RegistryEntry>,
  }

  #[derive(Debug, Clone)]
  pub struct RegistryEntry {
      pub const_ident: String,    // e.g. "AUTH_LOGIN"
      pub value: String,          // e.g. "auth.login"
      pub kind: Kind,             // Stateful | Event
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Kind { Stateful, Event }

  pub fn load(path: &Path) -> Result<Registry, String> {
      let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
      let file: syn::File = syn::parse_str(&src).map_err(|e| e.to_string())?;
      let mut actions = HashMap::new();
      visit_items(&file.items, &mut actions)?;
      Ok(Registry { actions })
  }

  fn visit_items(items: &[Item], out: &mut HashMap<String, RegistryEntry>) -> Result<(), String> {
      for item in items {
          match item {
              Item::Impl(imp) => {
                  for it in &imp.items {
                      if let syn::ImplItem::Const(c) = it {
                          if let Some(entry) = parse_registered(&c.ident.to_string(), &c.expr) {
                              out.insert(entry.value.clone(), entry);
                          }
                      }
                  }
              }
              Item::Mod(m) => if let Some((_, items)) = &m.content { visit_items(items, out)?; },
              _ => {}
          }
      }
      Ok(())
  }

  fn parse_registered(const_ident: &str, expr: &Expr) -> Option<RegistryEntry> {
      // Expect: RegisteredAuditAction::new("auth.login", AuditActionKind::Event)
      let Expr::Call(ExprCall { func, args, .. }) = expr else { return None; };
      let Expr::Path(ExprPath { path, .. }) = &**func else { return None; };
      if !path.segments.last().is_some_and(|s| s.ident == "new") { return None; }
      if args.len() != 2 { return None; }
      let value = match &args[0] {
          Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => s.value(),
          _ => return None,
      };
      let kind = match &args[1] {
          Expr::Path(p) => {
              let last = p.path.segments.last()?;
              match last.ident.to_string().as_str() {
                  "Stateful" => Kind::Stateful,
                  "Event" => Kind::Event,
                  _ => return None,
              }
          }
          _ => return None,
      };
      Some(RegistryEntry { const_ident: const_ident.into(), value, kind })
  }
  ```

- [ ] **Step 2: Test**

  ```rust
  #[test]
  fn registry_load_finds_known_actions() {
      let reg = registry::load(std::path::Path::new("../../src/action_type.rs")).expect("parse");
      let login = reg.actions.get("auth.login").expect("auth.login");
      assert_eq!(login.kind, registry::Kind::Event);
      let pcu = reg.actions.get("plugin_config.update").expect("plugin_config.update");
      assert_eq!(pcu.kind, registry::Kind::Stateful);
  }
  ```

- [ ] **Step 3:** `cargo test -p audit-coverage-check` — Expected: PASS.

- [ ] **Step 4: Commit:**

  ```bash
  git commit -am "feat(audit-v2): coverage-check registry reflection"
  ```

---

## Task 6: Walker — identify mutation sites

**Files:**

- Modify: `crates/shared/audit-log/tools/audit-coverage-check/src/walker.rs`

The walker visits every `.rs` file under `crates/` (skipping `target/`, `node_modules/`, hidden dirs) and identifies state-changing
sites by AST shape. Three primary detection rules:

1. **Axum HTTP handler functions reachable from any router builder, filtered by HTTP verb.** Implementation: scan for
   `Router::new().route("...", post(...))` / `put` / `patch` / `delete` calls; resolve the handler ident; record its
   fully-qualified path.

2. **Scheduler executors:** any `impl ScheduledTaskExecutor for X` or `impl Executor for X` block where the `run` method
   is implemented. Records `<crate>::<module path>::<X>::run`.

3. **Functions with `#[audit_required]` attribute** (escape hatch). Records the function's fully-qualified path.

The walker honours `#[cfg(feature = "...")]` attributes as metadata only — it does not skip cfg-out sites. Catalog entries must
exist regardless of build feature set.

- [ ] **Step 1: Implement file collection**

  ```rust
  use std::path::{Path, PathBuf};
  use walkdir::WalkDir;

  pub fn collect_rust_sources(root: &Path) -> Vec<PathBuf> {
      WalkDir::new(root.join("crates"))
          .into_iter()
          .filter_entry(|e| {
              let n = e.file_name().to_string_lossy();
              !(n == "target" || n.starts_with('.') || n == "node_modules")
          })
          .filter_map(Result::ok)
          .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
          .map(|e| e.path().to_owned())
          .collect()
  }
  ```

- [ ] **Step 2: Axum verb-handler detection**

  ```rust
  use syn::{visit::Visit, ExprCall, ExprMethodCall, Path as SynPath};

  /// Collect identifiers used as POST/PUT/PATCH/DELETE handlers in `.route("...", post(handler))` calls.
  pub struct VerbHandlerCollector { pub handlers: Vec<String> }

  impl<'ast> Visit<'ast> for VerbHandlerCollector {
      fn visit_expr_method_call(&mut self, m: &'ast ExprMethodCall) {
          if m.method == "route" {
              // route("/x", post(handler).put(other_handler))
              for arg in &m.args {
                  collect_handler_idents_in_expr(arg, &mut self.handlers);
              }
          }
          syn::visit::visit_expr_method_call(self, m);
      }
  }

  fn collect_handler_idents_in_expr(e: &syn::Expr, out: &mut Vec<String>) {
      // recursively look for ExprCall whose func path is one of post/put/patch/delete (single segment)
      use syn::Expr::*;
      match e {
          Call(ExprCall { func, args, .. }) => {
              if let syn::Expr::Path(p) = &**func {
                  if matches!(verb_name(&p.path), Some("post") | Some("put") | Some("patch") | Some("delete")) {
                      for a in args {
                          if let syn::Expr::Path(handler) = a {
                              if let Some(seg) = handler.path.segments.last() {
                                  out.push(seg.ident.to_string());
                              }
                          }
                      }
                  }
              }
              for a in args { collect_handler_idents_in_expr(a, out); }
          }
          MethodCall(m) => {
              collect_handler_idents_in_expr(&m.receiver, out);
              for a in &m.args { collect_handler_idents_in_expr(a, out); }
          }
          _ => {}
      }
  }

  fn verb_name(p: &SynPath) -> Option<&str> {
      p.segments.last().map(|s| s.ident.to_string()).as_deref().and_then(|n| match n {
          "post" | "put" | "patch" | "delete" => Some(n),
          _ => None,
      })
  }
  ```

  > Note: handler-ident collection by single segment matches the codebase pattern where `post(handler)` is used with imported
  > functions. Where the handler is referenced as `module::handler`, the walker recognises only the trailing segment — combined
  > with the file's crate + module path, this becomes the fully-qualified site identifier.

- [ ] **Step 3: Scheduler executor detection**

  ```rust
  pub struct ExecutorCollector { pub run_sites: Vec<String> }
  impl<'ast> Visit<'ast> for ExecutorCollector {
      fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
          if let Some((_, trait_path, _)) = &i.trait_ {
              let trait_name = trait_path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
              if trait_name.ends_with("Executor") || trait_name == "ScheduledTaskExecutor" {
                  if let syn::Type::Path(ty) = &*i.self_ty {
                      if let Some(seg) = ty.path.segments.last() {
                          self.run_sites.push(format!("{}::run", seg.ident));
                      }
                  }
              }
          }
          syn::visit::visit_item_impl(self, i);
      }
  }
  ```

- [ ] **Step 4: `#[audit_required]` attribute detection**

  ```rust
  pub struct AttrRequiredCollector { pub sites: Vec<String> }
  impl<'ast> Visit<'ast> for AttrRequiredCollector {
      fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
          let has = f.attrs.iter().any(|a| a.path().is_ident("audit_required"));
          if has { self.sites.push(f.sig.ident.to_string()); }
          syn::visit::visit_item_fn(self, f);
      }
  }
  ```

- [ ] **Step 5: Assemble `scan`**

  ```rust
  pub struct WalkReport {
      pub missing_catalog_entry: Vec<String>,
      pub unknown_action: Vec<String>,
      pub stale_skip: Vec<String>,
  }

  pub fn scan(root: &Path, catalog: &Catalog, registry: &Registry) -> Result<WalkReport, String> {
      let files = collect_rust_sources(root);
      let mut catalog_sites: std::collections::HashSet<&str> = catalog.entries.iter().map(|e| e.site.as_str()).collect();
      let mut discovered: std::collections::HashSet<String> = Default::default();

      for path in &files {
          let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
          let file: syn::File = match syn::parse_str(&src) {
              Ok(f) => f, Err(_) => continue, // skip unparseable .rs (build scripts etc.)
          };
          let module_path = derive_module_path(root, path);

          let mut verbs = VerbHandlerCollector { handlers: vec![] };
          syn::visit::visit_file(&mut verbs, &file);
          for h in verbs.handlers { discovered.insert(format!("{module_path}::{h}")); }

          let mut execs = ExecutorCollector { run_sites: vec![] };
          syn::visit::visit_file(&mut execs, &file);
          for r in execs.run_sites { discovered.insert(format!("{module_path}::{r}")); }

          let mut attrs = AttrRequiredCollector { sites: vec![] };
          syn::visit::visit_file(&mut attrs, &file);
          for s in attrs.sites { discovered.insert(format!("{module_path}::{s}")); }
      }

      let mut report = WalkReport { missing_catalog_entry: vec![], unknown_action: vec![], stale_skip: vec![] };

      for site in &discovered {
          if !catalog_sites.contains(site.as_str()) {
              report.missing_catalog_entry.push(site.clone());
          }
      }
      for e in &catalog.entries {
          if !discovered.contains(&e.site) {
              report.stale_skip.push(e.site.clone());
          }
          if let Some(action) = &e.action {
              if !registry.actions.contains_key(action) {
                  report.unknown_action.push(format!("{} -> {}", e.site, action));
              }
          }
      }
      Ok(report)
  }

  fn derive_module_path(root: &Path, file: &Path) -> String {
      // crates/<crate>/src/<a>/<b>.rs  ->  <crate>::<a>::<b>
      let rel = file.strip_prefix(root).unwrap_or(file);
      let comps: Vec<_> = rel.components().filter_map(|c| match c {
          std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
          _ => None,
      }).collect();
      // Drop the leading "crates/<crate>/src" and ".rs"
      // …implementation detail; uses the same crate-name detection the existing tooling uses…
      comps.join("::")
  }
  ```

- [ ] **Step 6: Fixture-driven integration tests**

  Create `crates/shared/audit-log/tools/audit-coverage-check/tests/fixtures/missing_entry/` containing a tiny synthetic crate with an
  unaudited POST handler. Create `tests/fixtures/clean/` containing the same handler with a matching catalog entry. Add
  `tests/integration.rs`:

  ```rust
  #[test]
  fn fails_on_missing_catalog_entry() {
      let output = std::process::Command::new(env!("CARGO_BIN_EXE_audit-coverage-check"))
          .current_dir("tests/fixtures/missing_entry")
          .output().expect("run");
      assert!(!output.status.success());
      let stderr = String::from_utf8_lossy(&output.stderr);
      assert!(stderr.contains("missing catalog entry"));
  }

  #[test]
  fn passes_when_clean() {
      let output = std::process::Command::new(env!("CARGO_BIN_EXE_audit-coverage-check"))
          .current_dir("tests/fixtures/clean")
          .output().expect("run");
      assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
  }
  ```

- [ ] **Step 7: Run tests + commit**

  ```bash
  cargo test -p audit-coverage-check
  git commit -am "feat(audit-v2): coverage-check walker (Axum + scheduler + audit_required)"
  ```

---

## Task 7: Emit-sweep check

**Files:**

- Modify: `crates/shared/audit-log/tools/audit-coverage-check/src/emit_sweep.rs`

For each registered Stateful action, the macro-generated constructor method is `AuditEntry::<snake_case>(`. The sweep greps all
`.rs` under `crates/` (excluding the audit-log crate's own definition + tests) for the call pattern. Missing call sites are
reported.

- [ ] **Step 1: Implement**

  ```rust
  use std::path::Path;
  use crate::registry::{Kind, Registry};

  pub struct EmitReport {
      pub stateful_actions_without_emit_site: Vec<String>,
  }

  pub fn scan(root: &Path, registry: &Registry) -> Result<EmitReport, String> {
      let files = crate::walker::collect_rust_sources(root);
      let stateful: Vec<&crate::registry::RegistryEntry> = registry.actions.values()
          .filter(|e| e.kind == Kind::Stateful)
          .collect();

      let mut missing = Vec::new();
      for action in stateful {
          let method = method_name_from_action(&action.value);
          let needle = format!("AuditEntry::{method}(");
          let mut found = false;
          'files: for f in &files {
              // Skip the audit-log crate itself.
              if f.to_string_lossy().contains("/audit-log/src/") { continue; }
              if let Ok(src) = std::fs::read_to_string(f) {
                  if src.contains(&needle) { found = true; break 'files; }
              }
          }
          if !found {
              missing.push(action.value.clone());
          }
      }
      Ok(EmitReport { stateful_actions_without_emit_site: missing })
  }

  /// "plugin_config.update" -> "plugin_config_update"
  fn method_name_from_action(value: &str) -> String { value.replace('.', "_") }
  ```

- [ ] **Step 2: Test against the workspace**

  Add a test that runs `emit_sweep::scan` against the actual workspace root and asserts no missing entries (assumes Plan B has
  landed). For pre-Plan-B local runs, this assertion will fail — document that this test must be temporarily skipped until Plan B
  ships.

- [ ] **Step 3: Commit**

  ```bash
  git commit -am "feat(audit-v2): coverage-check emit-sweep complement check"
  ```

---

## Task 8: Seed the catalog

**Files:**

- Modify: `crates/shared/audit-log/audit-catalog.toml`

This task populates the catalog with every Stateful and Event call site discovered in the workspace, plus every out-of-scope site
that needs an explicit `skip`.

- [ ] **Step 1: Run the walker against the workspace to harvest missing entries**

  ```bash
  cargo run -p audit-coverage-check 2>&1 | tee /tmp/missing-sites.txt
  ```

  Read `/tmp/missing-sites.txt` for the list of `missing catalog entry: <site>` lines.

- [ ] **Step 2: For each site, decide either `action = "..."` or `skip = "..."`**

  Procedure:
  - If the site is a state-changing HTTP route handler: look up the action it emits and add `action = "<action>"`.
  - If the site is a GET handler / heartbeat / lifecycle path / cache write: add `skip = "<reason>"` per the spec's
    out-of-scope categories (§"Out-of-scope sites").
  - If the site is a scheduler executor: add the corresponding `system.scheduler.<x>` action.

- [ ] **Step 3: Re-run until clean**

  ```bash
  cargo run -p audit-coverage-check
  ```

  Exit 0 expected.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/shared/audit-log/audit-catalog.toml
  git commit -m "feat(audit-v2): seed audit-catalog.toml for full workspace coverage"
  ```

---

## Task 9: Wire into CI

**Files:**

- Modify: `.github/workflows/ci.yml` (or the workspace CI runner script — confirm with `ls .github/workflows/`)

- [ ] **Step 1: Locate CI lint job**

  Run: `ls .github/workflows/`. Identify the workflow that runs `cargo clippy` / `cargo deny`.

- [ ] **Step 2: Add the gate step**

  In the lint job, alongside `cargo deny check`:

  ```yaml
  - name: Audit coverage check
    run: cargo run -p audit-coverage-check
  ```

  If the project uses a `Justfile` or `Makefile` for local runs, add the same step to it.

- [ ] **Step 3: Test CI manually by pushing a no-op branch**

  Push and verify the audit-coverage-check job runs and passes.

- [ ] **Step 4: Commit**

  ```bash
  git add .github/workflows/
  git commit -m "ci(audit-v2): run audit-coverage-check in lint job"
  ```

---

## Task 10: `#[audit_required]` escape hatch

**Files:**

- Modify: `crates/shared/audit-log-derive/src/lib.rs`

The walker recognises functions annotated `#[audit_required]` as mutation sites that don't match the Axum/scheduler/runtime shape.
Provide the attribute as a no-op proc-macro.

- [ ] **Step 1: Add the attribute macro**

  In `crates/shared/audit-log-derive/src/lib.rs`:

  ```rust
  #[proc_macro_attribute]
  pub fn audit_required(_attrs: TokenStream, item: TokenStream) -> TokenStream {
      // Marker only; the walker reads this attribute. No code transformation.
      item
  }
  ```

  Re-export from `uptrakit-audit-log`:

  ```rust
  pub use uptrakit_audit_log_derive::audit_required;
  ```

- [ ] **Step 2: Quality gates + commit**

  ```bash
  cargo check --all-features
  git commit -am "feat(audit-v2): #[audit_required] marker attribute for coverage-check"
  ```

---

## Task 11: Final quality gates + push

- [ ] **Step 1:**

  ```bash
  cargo fmt --all
  cargo check --all-features
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  cargo run -p audit-coverage-check
  cargo deny check
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: green.

- [ ] **Step 2:**

  ```bash
  git push -u origin feat/audit-v2-catalog-ci-gate
  ```

---

## Spec coverage check (Plan C scope)

This plan delivers:

- Spec §"Catalog and static-analysis CI gate" — `audit-catalog.toml`, the `audit-coverage-check` binary, three detection rules,
  cfg-feature awareness, CI wiring.
- Spec §"Complementary assertion: every Stateful action has an emit site" — Task 7.
- Spec §"Why a Rust source walker over grep/regex" — implementation uses `syn`-based AST traversal.
- `#[audit_required]` escape hatch — Task 10.

Deferred to Plan D: frontend State tab + correlation_id filter + CLI rendering.
Deferred to Plan E: documentation deliverables + new ADR.
