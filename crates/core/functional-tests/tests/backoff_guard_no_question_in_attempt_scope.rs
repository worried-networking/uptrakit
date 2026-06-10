//! Workspace lint: forbid `?` or `select!` inside the lexical scope of a live
//! `AttemptGuard`.
//!
//! # Rule
//!
//! Between `let g = backoff.attempt()` and `g.reset()` / `g.escalate()` /
//! `drop(g)` — together called "resolution" — the following are forbidden:
//!
//! - Any `?` operator (`ExprTry`).
//! - Any `select!` / `tokio::select!` / `futures::select!` macro invocation.
//!
//! The compile-time net (`unused_must_use`, `let_underscore_must_use = "deny"`)
//! catches the common "forgot to resolve" cases. However the realistic refactor
//! footgun:
//!
//! ```text
//! let guard = backoff.attempt();
//! let delay = guard.sample_delay();
//! some_fallible_call()?;   // drops guard unresolved in release!
//! guard.escalate();        // unreachable
//! ```
//!
//! compiles cleanly because the guard is "used" (`.sample_delay()` satisfies
//! `#[must_use]`). In release the unresolved `Drop` produces only a `warn!`
//! log and **no state mutation** — a CI grep nobody alerts on hides the bug.
//!
//! # `tokio::select!` opacity
//!
//! The body of a `select!` macro is opaque to `syn` (inner tokens are not
//! parsed as Rust AST without macro expansion). Consequently the `?` rule
//! cannot be enforced inside a `select!` body. The complementary rule —
//! flagging the `select!` invocation itself — catches the realistic footgun of
//! a future contributor holding a live guard across a `select!` containing
//! `recv().await?` arms. The rule fires if a `select!` macro is encountered
//! while any guard frame is open, **unless** the guard was resolved on the
//! statement immediately preceding the `select!`.
//!
//! In the migrated codebase every `select!` that appears after an `attempt()`
//! call appears only after the guard is already resolved (e.g.
//! `guard.escalate()` then `tokio::select! { ... }`). The visitor closes the
//! frame at the resolution call, so those sites do not trigger.
//!
//! # Escape hatch
//!
//! Add a comment on the violating line **or the line immediately above** it:
//!
//! ```text
//! // uptrakit-backoff: allow ? in attempt scope — <reason>
//! ```
//!
//! Both the `?` and the `select!` rules honour this suppression comment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cargo_metadata::MetadataCommand;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    Expr, ExprCall, ExprMacro, ExprMethodCall, ExprTry, File, Ident, Local, LocalInit, Pat,
    PatIdent,
};

const ALLOW_COMMENT: &str = "uptrakit-backoff: allow ? in attempt scope";

// ---------------------------------------------------------------------------
// Frame tracking
// ---------------------------------------------------------------------------

/// One open guard scope: the binding ident and the block depth at which it was
/// created. The frame is closed (popped) when:
/// - The visitor encounters `<ident>.reset()` or `<ident>.escalate()`.
/// - The visitor encounters `drop(<ident>)`.
/// - The block that created the frame exits.
#[derive(Debug)]
struct GuardFrame {
    /// The local-variable ident string (e.g. `"guard"`).
    ident: String,
    /// Block depth at the point the `Local` was seen.
    block_depth: usize,
}

// ---------------------------------------------------------------------------
// Visitor
// ---------------------------------------------------------------------------

struct BackoffVisitor<'src> {
    file_path: String,
    /// All source lines, zero-indexed. Used for allowlist lookup.
    lines: Vec<&'src str>,
    /// Accumulated violations: (file_path, 1-based line, message).
    violations: Vec<(String, u32, String)>,
    /// Stack of open guard frames.
    frames: Vec<GuardFrame>,
    /// Current nesting depth of `Block`s.
    block_depth: usize,
}

impl<'src> BackoffVisitor<'src> {
    fn new(file_path: impl Into<String>, source: &'src str) -> Self {
        Self {
            file_path: file_path.into(),
            lines: source.lines().collect(),
            violations: Vec::new(),
            frames: Vec::new(),
            block_depth: 0,
        }
    }

    /// Returns `true` if line `line` (1-based) or the line immediately above
    /// it contains the allowlist comment.
    fn is_suppressed(&self, line: u32) -> bool {
        let check = |ln: u32| -> bool {
            if ln == 0 {
                return false;
            }
            let idx = (ln - 1) as usize;
            self.lines
                .get(idx)
                .is_some_and(|l| l.contains(ALLOW_COMMENT))
        };
        check(line) || check(line.saturating_sub(1))
    }

    fn record_violation(&mut self, line: u32, kind: &str) {
        if !self.is_suppressed(line) {
            self.violations
                .push((self.file_path.clone(), line, kind.to_string()));
        }
    }

    /// Pop the frame for `name` if one is open.
    fn close_frame(&mut self, name: &str) {
        if let Some(pos) = self.frames.iter().rposition(|f| f.ident == name) {
            self.frames.remove(pos);
        }
    }
}

// ---------------------------------------------------------------------------
// Expression ident extraction helpers
// ---------------------------------------------------------------------------

/// Try to extract a simple single-segment path ident string from an expression
/// (e.g. `guard`, `drop`). Returns `None` for qualified paths or multi-segment
/// paths.
fn expr_path_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(ep) if ep.qself.is_none() && ep.path.segments.len() == 1 => {
            ep.path.segments.first().map(|s| s.ident.to_string())
        }
        _ => None,
    }
}

/// Extract the bound ident from a pattern if it is a simple `PatIdent` with
/// no sub-pattern (e.g. `let guard = ...`; but not `let (a, b) = ...`).
fn pat_simple_ident(pat: &Pat) -> Option<&Ident> {
    if let Pat::Ident(PatIdent {
        ident,
        subpat: None,
        ..
    }) = pat
    {
        Some(ident)
    } else {
        None
    }
}

/// True if `expr` is `<receiver>.attempt()` (no args, no generic turbofish).
fn is_attempt_call(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::MethodCall(ExprMethodCall { method, args, .. })
            if method == "attempt" && args.is_empty()
    )
}

// ---------------------------------------------------------------------------
// syn::visit::Visit impl
// ---------------------------------------------------------------------------

impl<'ast, 'src> Visit<'ast> for BackoffVisitor<'src> {
    // ------------------------------------------------------------------
    // Block: manage frame lifetime by block depth.
    // ------------------------------------------------------------------
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.block_depth += 1;
        let depth = self.block_depth;
        // Recurse into all statements inside the block via the default visitor.
        syn::visit::visit_block(self, block);
        // Pop any frames that were opened inside this block depth — they are
        // now out of scope regardless of whether they were explicitly resolved.
        self.frames.retain(|f| f.block_depth != depth);
        self.block_depth -= 1;
    }

    // ------------------------------------------------------------------
    // Local (let binding): detect `let <ident> = <expr>.attempt()`.
    // ------------------------------------------------------------------
    fn visit_local(&mut self, local: &'ast Local) {
        // Check before recursing so the frame is registered when any
        // sub-expressions of the init are visited.
        let should_push =
            matches!(&local.init, Some(LocalInit { expr, .. }) if is_attempt_call(expr));
        if should_push && let Some(ident) = pat_simple_ident(&local.pat) {
            self.frames.push(GuardFrame {
                ident: ident.to_string(),
                block_depth: self.block_depth,
            });
        }
        // Recurse into the rest of the local declaration.
        syn::visit::visit_local(self, local);
    }

    // ------------------------------------------------------------------
    // Expr: detect violations and resolutions.
    // ------------------------------------------------------------------
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            // ---------------------------------------------------------
            // `?` operator — violation if any guard is open.
            // ---------------------------------------------------------
            Expr::Try(ExprTry { expr: inner, .. }) => {
                if !self.frames.is_empty() {
                    // Use the span of the inner expression for the line number.
                    let line = inner.span().start().line as u32;
                    self.record_violation(line, "? operator inside open AttemptGuard scope");
                }
                // Recurse into the inner expression.
                syn::visit::visit_expr(self, expr);
            }

            // ---------------------------------------------------------
            // Method call — detect resolution: `<ident>.reset()` or
            // `<ident>.escalate()`.
            // ---------------------------------------------------------
            Expr::MethodCall(ExprMethodCall {
                receiver, method, ..
            }) => {
                if (method == "reset" || method == "escalate")
                    && !self.frames.is_empty()
                    && let Some(name) = expr_path_ident(receiver)
                {
                    // Close the frame before recursing into the call.
                    self.close_frame(&name);
                }
                // Always recurse — the resolution is already handled above.
                syn::visit::visit_expr(self, expr);
            }

            // ---------------------------------------------------------
            // Function call — detect `drop(<ident>)`.
            // ---------------------------------------------------------
            Expr::Call(ExprCall { func, args, .. }) => {
                if args.len() == 1
                    && expr_path_ident(func).as_deref() == Some("drop")
                    && let Some(arg_name) = expr_path_ident(&args[0])
                {
                    self.close_frame(&arg_name);
                }
                syn::visit::visit_expr(self, expr);
            }

            // ---------------------------------------------------------
            // Macro invocation — detect `select!` while a guard is open.
            // The macro body is opaque (not expanded by syn), so we flag
            // the presence of a `select!` as a potential violation when
            // any guard frame is still live.
            // ---------------------------------------------------------
            Expr::Macro(ExprMacro { mac, .. }) => {
                if !self.frames.is_empty() {
                    let last_seg = mac.path.segments.last().map(|s| s.ident.to_string());
                    if last_seg.as_deref() == Some("select") {
                        let line: u32 = mac
                            .path
                            .segments
                            .last()
                            .map(|s| s.ident.span().start().line as u32)
                            .unwrap_or(0);
                        self.record_violation(
                            line,
                            "select! macro inside open AttemptGuard scope \
                             (body is opaque to syn; ? inside select! arms cannot be checked — \
                             resolve the guard before the select!)",
                        );
                    }
                }
                syn::visit::visit_expr(self, expr);
            }

            // ---------------------------------------------------------
            // All other expressions — recurse normally.
            // ---------------------------------------------------------
            _ => {
                syn::visit::visit_expr(self, expr);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

fn collect_rs_files_inner(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files_inner(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Collect all `*.rs` files under `<member_root>/src/` and
/// `<member_root>/tests/`, skipping non-existent directories silently.
fn member_rs_files(member_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for sub in ["src", "tests"] {
        let dir = member_root.join(sub);
        if dir.exists() {
            collect_rs_files_inner(&dir, &mut files);
        }
    }
    files
}

// ---------------------------------------------------------------------------
// Per-file lint runner
// ---------------------------------------------------------------------------

fn lint_file(path: &Path) -> Vec<(String, u32, String)> {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    // Quick pre-filter: skip files that don't reference `.attempt()` at all.
    if !source.contains(".attempt()") {
        return vec![];
    }
    let ast: File = match syn::parse_file(&source) {
        Ok(f) => f,
        // Skip files that fail to parse (e.g. macro-generated, generated code).
        Err(_) => return vec![],
    };
    let path_str = path.to_string_lossy().into_owned();
    let mut visitor = BackoffVisitor::new(path_str, &source);
    visitor.visit_file(&ast);
    visitor.violations
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test]
fn backoff_guard_no_question_in_attempt_scope() {
    // Locate workspace root: functional-tests lives at
    // <workspace>/crates/core/functional-tests → pop three levels.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_workspace_root = {
        let mut p = manifest_dir.clone();
        for _ in 0..3 {
            p.pop();
        }
        p
    };

    // Enumerate workspace member crate roots via cargo_metadata.
    // Use the manifest path derived from CARGO_MANIFEST_DIR to anchor the
    // metadata command. The `no_deps()` call skips external dep resolution.
    let metadata = MetadataCommand::new()
        .manifest_path(manifest_workspace_root.join("Cargo.toml"))
        .no_deps()
        .exec()
        .expect("cargo metadata failed");

    // Use cargo_metadata's canonical workspace root and target directory.
    let workspace_root = metadata.workspace_root.as_std_path().to_path_buf();
    let target_dir = metadata.target_directory.as_std_path().to_path_buf();

    let mut violations: Vec<(String, u32, String)> = Vec::new();
    let mut file_count = 0usize;

    // Keyed by package name to avoid duplicates (workspace can list the same
    // manifest from multiple virtual paths in rare setups).
    let mut paths_by_member: HashMap<String, Vec<PathBuf>> = HashMap::new();

    // `workspace_packages()` returns only true workspace members (excludes
    // path-dep non-members and external registry deps).
    for package in metadata.workspace_packages() {
        let member_root = package
            .manifest_path
            .parent()
            .expect("package manifest path has no parent")
            .as_std_path()
            .to_path_buf();

        // Skip anything inside the target/ directory. We do NOT exclude
        // ".worktrees/" paths here — when running from inside a git worktree,
        // all member paths legitimately start with ".worktrees/" relative to
        // the main workspace root. `workspace_packages()` already filters to
        // the current workspace's members, so no cross-worktree contamination.
        if member_root.starts_with(&target_dir)
            || member_root
                .strip_prefix(&workspace_root)
                .is_ok_and(|rel| rel.starts_with("target"))
        {
            continue;
        }

        paths_by_member
            .entry(package.name.to_string())
            .or_insert_with(|| member_rs_files(&member_root));
    }

    for files in paths_by_member.values() {
        for path in files {
            // Belt-and-suspenders: skip anything inside target/.
            // Note: we do NOT exclude paths containing ".worktrees" here —
            // when running from inside a git worktree (as this crate does),
            // ALL source paths legitimately contain ".worktrees/" in their
            // absolute path. The package filter above already limits us to
            // the current workspace's members.
            if path.to_string_lossy().contains("/target/") {
                continue;
            }
            file_count += 1;
            violations.extend(lint_file(path));
        }
    }

    assert!(
        file_count > 0,
        "lint walked zero files — cargo_metadata or path logic is broken"
    );

    if !violations.is_empty() {
        let mut sorted = violations.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let msg = sorted
            .iter()
            .map(|(file, line, kind)| format!("  {file}:{line}: {kind}"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "backoff guard lint: {} violation(s) found — \
             resolve the AttemptGuard (via .reset() or .escalate()) \
             before any ? or select! in the same scope.\n\
             Escape hatch: add `// uptrakit-backoff: allow ? in attempt scope — <reason>` \
             on the offending line or the line immediately above.\n\
             Violations:\n{msg}",
            sorted.len(),
        );
    }
}
