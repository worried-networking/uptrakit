//! Source-tree walker for audit emit call sites.
//!
//! Walks the workspace source tree, identifies state-changing call sites, and
//! cross-references them against the [`Catalog`] and [`Registry`] to produce a
//! [`WalkReport`].
//!
//! # Detection rules
//!
//! Four kinds of mutation sites are detected:
//!
//! 1. **utoipa-axum handler functions** — `async fn` annotated with
//!    `#[utoipa::path(post/put/patch/delete, ...)]`; emits
//!    `<module_path>::<fn_name>` for each such function.
//! 2. **Axum handler functions** — `.route("...", post(handler))` / `put` / `patch` / `delete`
//!    chains; emits `<module_path>::<handler_ident>` for each handler ident.
//! 3. **Scheduler executors** — any `impl …Executor for X` block where a `run` method is
//!    implemented; emits `<module_path>::<X>::run`.
//! 4. **`#[audit_required]` functions** — emits `<module_path>::<fn_name>`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

use crate::{catalog::Catalog, registry::Registry};

/// Results from scanning the source tree for audit call-site coverage.
#[derive(Debug)]
pub struct WalkReport {
    /// Call sites that were detected but have no catalog entry.
    pub missing_catalog_entry: Vec<String>,
    /// Catalog entries whose `action` value does not match any registered action.
    pub unknown_action: Vec<String>,
    /// Catalog entries whose `site` pattern was not found anywhere in the source tree.
    pub stale_skip: Vec<String>,
}

// ── File collection ──────────────────────────────────────────────────────────

/// Collect all `.rs` files under `<root>/crates/`, skipping `target/`, hidden
/// directories, `node_modules/`, and `fixtures/`.
pub(crate) fn collect_rust_sources(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root.join("crates"))
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(n == "target" || n.starts_with('.') || n == "node_modules" || n == "fixtures")
        })
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
        .map(|e| e.path().to_owned())
        .collect()
}

// ── Cargo.toml cache ─────────────────────────────────────────────────────────

/// Cache of directory → crate name derived from its `Cargo.toml`.
type CargoCache = HashMap<PathBuf, Option<String>>;

/// Walk upward from `start` looking for a `Cargo.toml` that contains a
/// `[package]` section with a `name` field.  Returns the crate name (with `-`
/// replaced by `_`) or `None` if none is found above `root`.
fn find_crate_name(start: &Path, root: &Path, cache: &mut CargoCache) -> Option<String> {
    let mut dir = start;
    loop {
        if let Some(cached) = cache.get(dir) {
            return cached.clone();
        }
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            let name = read_package_name(&candidate);
            cache.insert(dir.to_owned(), name.clone());
            if name.is_some() {
                return name;
            }
        }
        let parent = dir.parent()?;
        // Don't climb above the workspace root.
        if dir == root {
            break;
        }
        dir = parent;
    }
    None
}

/// Parse `name = "..."` from a `Cargo.toml`, replacing `-` with `_`.
fn read_package_name(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    // Simple line scan — avoids an additional toml parse just for the name.
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
        }
        if in_package
            && trimmed.starts_with("name")
            && let Some(eq_pos) = trimmed.find('=')
        {
            // Use split_at on the byte boundary of '=' (ASCII, safe).
            let rhs = trimmed.split_at(eq_pos + 1).1.trim();
            let raw = rhs.trim_matches('"');
            if !raw.is_empty() {
                return Some(raw.replace('-', "_"));
            }
        }
    }
    None
}

// ── Module path derivation ────────────────────────────────────────────────────

/// Derive the Rust module path for a source file.
///
/// Maps e.g. `crates/ui/web-api/src/routes/plugin_configs.rs` to
/// `uptrakit_web_api::routes::plugin_configs`.
///
/// Algorithm:
/// 1. Strip `root` prefix.
/// 2. Walk upward from the file's parent looking for `Cargo.toml` → crate name.
/// 3. Find the `src/` component after stripping the crate root.
/// 4. Take all components after `src/`, strip `.rs`, exclude sentinel names.
pub(crate) fn derive_module_path(root: &Path, file: &Path, cache: &mut CargoCache) -> String {
    // Relative path from workspace root.
    let rel = match file.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("warning: file outside workspace root: {}", file.display());
            return file.display().to_string();
        }
    };

    // Walk upward from the file's parent to locate the nearest Cargo.toml.
    let file_parent = match file.parent() {
        Some(p) => p,
        None => {
            eprintln!("warning: file has no parent: {}", file.display());
            return file.display().to_string();
        }
    };

    let crate_name = find_crate_name(file_parent, root, cache);

    // Locate the `src/` component in the relative path.
    let components: Vec<_> = rel.components().collect();
    let src_idx = components.iter().position(|c| c.as_os_str() == "src");

    let Some(src_idx) = src_idx else {
        // No `src/` segment — use filesystem path as fallback.
        let fallback = rel
            .display()
            .to_string()
            .replace(['/', '\\'], "::")
            .replace(".rs", "");
        return match crate_name {
            Some(name) => format!("{name}::{fallback}"),
            None => {
                eprintln!("warning: no Cargo.toml found for {}", file.display());
                fallback
            }
        };
    };

    // Components after `src/`, forming the module path segments.
    let mod_segments: Vec<String> = components
        .get(src_idx + 1..)
        .unwrap_or(&[])
        .iter()
        .filter_map(|c| {
            let s = c.as_os_str().to_string_lossy();
            // Strip .rs extension from the last component.
            let seg = s
                .strip_suffix(".rs")
                .unwrap_or_else(|| s.as_ref())
                .to_owned();
            // Exclude sentinel names that are not real module segments.
            if matches!(seg.as_str(), "lib" | "main" | "mod") {
                None
            } else {
                Some(seg)
            }
        })
        .collect();

    match crate_name {
        Some(name) => {
            if mod_segments.is_empty() {
                name
            } else {
                format!("{name}::{}", mod_segments.join("::"))
            }
        }
        None => {
            eprintln!("warning: no Cargo.toml found for {}", file.display());
            mod_segments.join("::")
        }
    }
}

// ── utoipa-axum handler collector ────────────────────────────────────────────

/// Returns `true` if `attrs` contains `#[utoipa::path(post/put/patch/delete, ...)]`.
///
/// Handles both single-line (`#[utoipa::path(post, path = "...")]`) and
/// multi-line forms where the verb is the first token inside the attribute.
fn has_mutation_verb_utoipa_path(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        // Must be `utoipa::path` (two segments).
        if path.segments.len() != 2 {
            return false;
        }
        if path.segments.first().is_none_or(|s| s.ident != "utoipa") {
            return false;
        }
        if path.segments.last().is_none_or(|s| s.ident != "path") {
            return false;
        }
        // Parse the first identifier from the attribute token stream (the HTTP verb).
        let Ok(meta_list) = attr.meta.require_list() else {
            return false;
        };
        // Iterate the token tree directly to get the first token — `parse2` requires
        // consuming the entire stream, which fails when the remaining `path = "..."` args
        // are still present.
        if let Some(proc_macro2::TokenTree::Ident(ident)) =
            meta_list.tokens.clone().into_iter().next()
        {
            return ident == "post" || ident == "put" || ident == "patch" || ident == "delete";
        }
        false
    })
}

/// Collect handler idents from `async fn` functions annotated with
/// `#[utoipa::path(post/put/patch/delete, ...)]`.
///
/// This is the primary detection mechanism for the `utoipa-axum` pattern used
/// throughout the web-api crate.
pub(crate) struct UtoipaHandlerCollector {
    /// Handler function name strings found.
    pub handlers: Vec<String>,
}

impl<'ast> Visit<'ast> for UtoipaHandlerCollector {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if f.sig.asyncness.is_some() && has_mutation_verb_utoipa_path(&f.attrs) {
            self.handlers.push(f.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, f);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        for item in &i.items {
            if let syn::ImplItem::Fn(method) = item
                && method.sig.asyncness.is_some()
                && has_mutation_verb_utoipa_path(&method.attrs)
            {
                self.handlers.push(method.sig.ident.to_string());
            }
        }
        syn::visit::visit_item_impl(self, i);
    }
}

// ── Axum verb-handler collector ───────────────────────────────────────────────

/// Returns `true` if `path` ends with an axum mutation verb (`post`, `put`,
/// `patch`, or `delete`).
fn is_mutation_verb(path: &syn::Path) -> bool {
    path.segments.last().is_some_and(|s| {
        let ident = &s.ident;
        ident == "post" || ident == "put" || ident == "patch" || ident == "delete"
    })
}

/// Collect handler idents from `post(handler)` / `put(handler)` / … calls that
/// appear as arguments to `.route("…", …)` method calls.
struct VerbHandlerCollector {
    /// Handler ident strings extracted from `.route(…, verb(handler))` chains.
    pub handlers: Vec<String>,
}

impl<'ast> Visit<'ast> for VerbHandlerCollector {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        // Look for `.route(…, expr)` calls.
        if node.method == "route" && node.args.len() == 2 {
            // The second argument is the handler expression — may be a single
            // `post(handler)` call or a chain like `post(h1).delete(h2)`.
            if let Some(second_arg) = node.args.iter().nth(1) {
                self.collect_from_verb_expr(second_arg);
            }
        }
        // Continue visiting nested expressions.
        syn::visit::visit_expr_method_call(self, node);
    }
}

impl VerbHandlerCollector {
    /// Recursively collect handler idents from a verb-call expression.
    ///
    /// Handles:
    /// - `post(handler)` — single call
    /// - `post(h1).delete(h2)` — method chain built on a verb call
    /// - `axum::routing::post(handler)` — fully qualified path
    fn collect_from_verb_expr(&mut self, expr: &syn::Expr) {
        match expr {
            // `post(handler)` or `axum::routing::post(handler)`
            syn::Expr::Call(call) => {
                if let syn::Expr::Path(func_path) = &*call.func
                    && is_mutation_verb(&func_path.path)
                {
                    self.collect_handlers_from_args(&call.args);
                }
            }
            // `post(h1).delete(h2)` — method chain; receiver may itself be a verb call
            syn::Expr::MethodCall(method_call) => {
                if is_mutation_verb(&syn::Path {
                    leading_colon: None,
                    segments: std::iter::once(syn::PathSegment {
                        ident: method_call.method.clone(),
                        arguments: syn::PathArguments::None,
                    })
                    .collect(),
                }) {
                    self.collect_handlers_from_args(&method_call.args);
                }
                // Recurse into the receiver for chained calls.
                self.collect_from_verb_expr(&method_call.receiver);
            }
            _ => {}
        }
    }

    /// Extract the handler ident from call arguments.
    ///
    /// Handles simple `handler` idents and fully-qualified `crate::mod::handler`
    /// paths.
    fn collect_handlers_from_args(
        &mut self,
        args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
    ) {
        for arg in args {
            if let syn::Expr::Path(p) = arg
                && let Some(last) = p.path.segments.last()
            {
                self.handlers.push(last.ident.to_string());
            }
        }
    }
}

// ── Scheduler executor collector ──────────────────────────────────────────────

/// Collect `<TypeName>::run` sites from `impl …Executor for TypeName` blocks
/// that contain a `run` method.
struct ExecutorCollector {
    /// `"<TypeName>::run"` strings found.
    pub run_sites: Vec<String>,
}

impl<'ast> Visit<'ast> for ExecutorCollector {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // Only interested in `impl TraitName for TypeName` (not inherent impls).
        if node.trait_.is_none() {
            syn::visit::visit_item_impl(self, node);
            return;
        }

        // Check the trait name ends with "Executor".
        let trait_ends_with_executor = node.trait_.as_ref().is_some_and(|(_, trait_path, _)| {
            trait_path
                .segments
                .last()
                .is_some_and(|s| s.ident.to_string().ends_with("Executor"))
        });

        if !trait_ends_with_executor {
            syn::visit::visit_item_impl(self, node);
            return;
        }

        // Check if there is a `run` method in this impl block.
        let has_run = node.items.iter().any(|item| {
            if let syn::ImplItem::Fn(m) = item {
                m.sig.ident == "run"
            } else {
                false
            }
        });

        if has_run
            && let syn::Type::Path(type_path) = &*node.self_ty
            && let Some(last) = type_path.path.segments.last()
        {
            self.run_sites.push(format!("{}::run", last.ident));
        }

        syn::visit::visit_item_impl(self, node);
    }
}

// ── `#[audit_required]` collector ────────────────────────────────────────────

/// Collect function names annotated with `#[audit_required]`.
struct AttrRequiredCollector {
    /// Function names from `#[audit_required]` annotations.
    pub sites: Vec<String>,
}

impl<'ast> Visit<'ast> for AttrRequiredCollector {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let has_attr = node.attrs.iter().any(|a| {
            a.path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "audit_required")
        });
        if has_attr {
            self.sites.push(node.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let has_attr = node.attrs.iter().any(|a| {
            a.path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "audit_required")
        });
        if has_attr {
            self.sites.push(node.sig.ident.to_string());
        }
        syn::visit::visit_impl_item_fn(self, node);
    }
}

// ── Main scan ─────────────────────────────────────────────────────────────────

/// Scan the workspace rooted at `root` for audit call-site coverage.
///
/// Cross-references detected call sites with `catalog` and `registry` to
/// populate a [`WalkReport`].
///
/// # Errors
///
/// Returns a descriptive string if the workspace tree cannot be traversed.
pub fn scan(root: &Path, catalog: &Catalog, registry: &Registry) -> Result<WalkReport, String> {
    let files = collect_rust_sources(root);
    let catalog_sites: HashSet<&str> = catalog.entries.iter().map(|e| e.site.as_str()).collect();
    let mut discovered: HashSet<String> = HashSet::new();
    let mut cache: CargoCache = HashMap::new();

    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let file: syn::File = match syn::parse_str(&src) {
            Ok(f) => f,
            Err(_) => continue, // skip unparseable files (build scripts, proc-macro crates)
        };
        let module_path = derive_module_path(root, path, &mut cache);

        let mut utoipa = UtoipaHandlerCollector { handlers: vec![] };
        syn::visit::visit_file(&mut utoipa, &file);
        for h in utoipa.handlers {
            discovered.insert(format!("{module_path}::{h}"));
        }

        let mut verbs = VerbHandlerCollector { handlers: vec![] };
        syn::visit::visit_file(&mut verbs, &file);
        for h in verbs.handlers {
            discovered.insert(format!("{module_path}::{h}"));
        }

        let mut execs = ExecutorCollector { run_sites: vec![] };
        syn::visit::visit_file(&mut execs, &file);
        for r in execs.run_sites {
            discovered.insert(format!("{module_path}::{r}"));
        }

        let mut attrs = AttrRequiredCollector { sites: vec![] };
        syn::visit::visit_file(&mut attrs, &file);
        for s in attrs.sites {
            discovered.insert(format!("{module_path}::{s}"));
        }
    }

    let mut report = WalkReport {
        missing_catalog_entry: vec![],
        unknown_action: vec![],
        stale_skip: vec![],
    };

    for site in &discovered {
        if !catalog_sites.contains(site.as_str()) {
            report.missing_catalog_entry.push(site.clone());
        }
    }
    for e in &catalog.entries {
        if !discovered.contains(&e.site) {
            report.stale_skip.push(e.site.clone());
        }
        if let Some(action) = &e.action
            && !registry.actions.contains_key(action)
        {
            report
                .unknown_action
                .push(format!("{} -> {}", e.site, action));
        }
    }

    Ok(report)
}
