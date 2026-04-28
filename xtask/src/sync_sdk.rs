use anyhow::{Context, Result, bail};
use proc_macro2::Span;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{Attribute, File, Ident, Item, Path as SynPath, UseTree, visit_mut::VisitMut};
use walkdir::WalkDir;

pub fn run(workspace_root: &Path, check: bool, commit: bool) -> Result<()> {
    let surfaces_src = workspace_root.join("crates/shared/surfaces/src");
    let wire_src = workspace_root.join("crates/shared/wire/src");
    let shared_types_src = workspace_root.join("crates/shared/types/src");
    let sdk_generated = workspace_root.join("crates/shared/service-sdk/src/generated");

    // Collect output files: path → content
    let mut output: Vec<(PathBuf, String)> = Vec::new();

    // 1. Surfaces — rewrite intra-crate `crate::` references
    collect_module(
        &surfaces_src,
        &sdk_generated.join("surfaces"),
        &[(&["crate"], &["crate", "generated", "surfaces"])],
        &mut output,
    )?;

    // 2. Shared-types — rewrite intra-crate `crate::` references
    collect_module(
        &shared_types_src,
        &sdk_generated.join("shared_types"),
        &[(&["crate"], &["crate", "generated", "shared_types"])],
        &mut output,
    )?;

    // 3. Wire — rewrite intra-crate `crate::`, uptrakit_surfaces, uptrakit_shared_types
    collect_module(
        &wire_src,
        &sdk_generated.join("wire"),
        &[
            (&["crate"], &["crate", "generated", "wire"]),
            (&["uptrakit_surfaces"], &["crate", "generated", "surfaces"]),
            (
                &["uptrakit_shared_types"],
                &["crate", "generated", "shared_types"],
            ),
        ],
        &mut output,
    )?;

    // 4. Top-level generated/mod.rs
    let mod_rs = sdk_generated.join("mod.rs");
    let mod_content = "pub mod shared_types;\npub mod surfaces;\npub mod wire;\n".to_string();
    output.push((mod_rs, mod_content));

    if check {
        run_check(&output)
    } else {
        write_output(&sdk_generated, &output)?;
        if commit {
            let status = std::process::Command::new("git")
                .args(["add", "crates/shared/service-sdk/src/generated/"])
                .status()?;
            anyhow::ensure!(status.success(), "git add failed");
            let status = std::process::Command::new("git")
                .args([
                    "commit",
                    "-m",
                    "chore(generated): regenerate service-sdk wire/surfaces types",
                ])
                .status()?;
            anyhow::ensure!(status.success(), "git commit failed");
        }
        Ok(())
    }
}

/// Walk `src_dir`, parse each .rs file, apply rewrites, collect into `output`.
/// `dst_dir` is the target directory path (used to build output paths).
pub(crate) fn collect_module(
    src_dir: &Path,
    dst_dir: &Path,
    rewrites: &[(&[&str], &[&str])],
    output: &mut Vec<(PathBuf, String)>,
) -> Result<()> {
    for entry in WalkDir::new(src_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "rs"))
    {
        let rel = entry.path().strip_prefix(src_dir)?;

        // Skip dedicated test files — they have external deps (serde_yaml_ng, etc.)
        // and are not meaningful in the generated SDK context.
        if rel.file_name().map_or(false, |f| f == "tests.rs") {
            continue;
        }

        // Rename lib.rs -> mod.rs so Rust module resolution works for inline dirs
        let rel = if rel == std::path::Path::new("lib.rs") {
            std::path::PathBuf::from("mod.rs")
        } else {
            rel.to_path_buf()
        };
        let dst_path = dst_dir.join(&rel);
        let src_content = fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;

        let content = rewrite_paths(&src_content, rewrites)
            .with_context(|| format!("rewriting {}", entry.path().display()))?;

        output.push((dst_path, content));
    }
    Ok(())
}

/// Returns true if the item has a `#[cfg(test)]` attribute.
fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        // Check if the cfg argument is exactly `test`
        attr.parse_args::<syn::Ident>()
            .map_or(false, |id| id == "test")
    })
}

/// Strip all top-level `#[cfg(test)]` items (mod blocks, fns, impls, etc.) from a file.
fn strip_test_items(file: &mut File) {
    file.items.retain(|item| {
        let attrs = match item {
            Item::Mod(m) => &m.attrs,
            Item::Fn(f) => &f.attrs,
            Item::Impl(i) => &i.attrs,
            Item::Use(u) => &u.attrs,
            Item::Struct(s) => &s.attrs,
            Item::Enum(e) => &e.attrs,
            Item::Const(c) => &c.attrs,
            Item::Static(s) => &s.attrs,
            Item::Trait(t) => &t.attrs,
            Item::Type(t) => &t.attrs,
            _ => return true,
        };
        !is_cfg_test(attrs)
    });
}

/// Parse `src`, apply all path segment rewrites, return formatted Rust source.
fn rewrite_paths(src: &str, rewrites: &[(&[&str], &[&str])]) -> Result<String> {
    let mut file: File = syn::parse_str(src).context("parsing source file (syn::parse_str)")?;

    // Remove #[cfg(test)] items — tests from source crates cannot compile in the
    // generated SDK context (wrong paths, missing deps like serde_yaml_ng, etc.).
    strip_test_items(&mut file);

    for (from, to) in rewrites {
        let mut rewriter = PathRewriter { from, to };
        rewriter.visit_file_mut(&mut file);
    }

    let formatted = prettyplease::unparse(&file);

    // Rewrite crate references inside doc-comment code blocks (/// ``` ... ```).
    // `syn::VisitMut` does not touch string literals / doc comments, so we do a
    // targeted text pass here.  In doctests, `crate::` is invalid — must use the
    // published crate name `uptrakit_service_sdk`.
    let formatted = formatted
        .replace(
            "use uptrakit_shared_types::",
            "use uptrakit_service_sdk::generated::shared_types::",
        )
        .replace(
            "use uptrakit_surfaces::",
            "use uptrakit_service_sdk::generated::surfaces::",
        )
        .replace(
            "use uptrakit_wire::",
            "use uptrakit_service_sdk::generated::wire::",
        );

    // Prepend lint-allow header so generated files compile cleanly under workspace lint policy.
    // `unreachable_patterns` triggers for `#[non_exhaustive]` catch-all arms when all variants
    // are matched explicitly; this is intentional for forward-compat.
    let output = format!(
        "// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.\n\
         #![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]\n\
         {formatted}"
    );

    // Run the output through rustfmt so on-disk files match what `cargo fmt` would produce.
    // This keeps the pre-commit rustfmt check green without a separate `cargo fmt` step.
    rustfmt_string(output)
}

/// Pipe `src` through `rustfmt --edition 2021` and return the formatted output.
fn rustfmt_string(src: String) -> Result<String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawning rustfmt")?;

    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(src.as_bytes())
        .context("writing to rustfmt stdin")?;

    let output = child.wait_with_output().context("waiting for rustfmt")?;
    anyhow::ensure!(output.status.success(), "rustfmt exited with error");
    String::from_utf8(output.stdout).context("rustfmt output is not valid UTF-8")
}

struct PathRewriter<'a> {
    from: &'a [&'a str],
    to: &'a [&'a str],
}

impl VisitMut for PathRewriter<'_> {
    fn visit_path_mut(&mut self, path: &mut SynPath) {
        // Recurse into nested paths first
        syn::visit_mut::visit_path_mut(self, path);

        let seg_idents: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        let n = self.from.len();
        if seg_idents.len() >= n && seg_idents[..n] == *self.from {
            // Collect the tail segments (after the matched prefix)
            let tail: Vec<_> = path.segments.iter().skip(n).cloned().collect();

            path.segments.clear();
            path.leading_colon = None;

            for seg_name in self.to {
                path.segments.push(syn::PathSegment {
                    ident: Ident::new(seg_name, Span::call_site()),
                    arguments: syn::PathArguments::None,
                });
            }
            path.segments.extend(tail);
        }
    }

    fn visit_use_tree_mut(&mut self, tree: &mut UseTree) {
        // Recurse first
        syn::visit_mut::visit_use_tree_mut(self, tree);
        rewrite_use_tree(tree, self.from, self.to);
    }
}

/// Rewrite a `UseTree` when its leading idents match `from`.
///
/// `use crate::foo::Bar` is represented as:
///   `UseTree::Path { ident: "crate", tree: UseTree::Path { ident: "foo", tree: ... } }`
///
/// We need to check if the chain of idents matches `from` and replace with `to`.
fn rewrite_use_tree(tree: &mut UseTree, from: &[&str], to: &[&str]) {
    if from.is_empty() || to.is_empty() {
        return;
    }

    // Collect the leading ident chain and find where the `from` prefix ends
    let idents = collect_use_tree_idents(tree);
    if idents.len() < from.len() {
        return;
    }
    if &idents[..from.len()] != from {
        return;
    }

    // Rebuild: prepend `to` segments, then attach the remaining original subtree
    // after stripping the `from` prefix
    let remaining = strip_use_tree_prefix(tree, from.len());

    // Build new tree: to[0]::to[1]::...::to[n-1]::<remaining>
    let new_tree = build_use_tree(to, remaining);
    *tree = new_tree;
}

/// Collect the leading idents from a UseTree chain (stops at Glob, Name, Rename, Group).
fn collect_use_tree_idents(tree: &UseTree) -> Vec<String> {
    let mut idents = Vec::new();
    let mut current = tree;
    loop {
        match current {
            UseTree::Path(p) => {
                idents.push(p.ident.to_string());
                current = &p.tree;
            }
            UseTree::Name(n) => {
                idents.push(n.ident.to_string());
                break;
            }
            UseTree::Rename(r) => {
                idents.push(r.ident.to_string());
                break;
            }
            UseTree::Glob(_) | UseTree::Group(_) => break,
        }
    }
    idents
}

/// Strip `n` leading Path segments from a UseTree, returning the subtree.
fn strip_use_tree_prefix(tree: &UseTree, n: usize) -> UseTree {
    let mut current = tree.clone();
    for _ in 0..n {
        match current {
            UseTree::Path(p) => current = *p.tree,
            // If we run out of path segments, just return the current node
            other => return other,
        }
    }
    current
}

/// Build a UseTree chain from `segments` with `tail` as the final leaf.
fn build_use_tree(segments: &[&str], tail: UseTree) -> UseTree {
    if segments.is_empty() {
        return tail;
    }
    // Build from the end
    let mut tree = tail;
    for &seg in segments.iter().rev() {
        tree = UseTree::Path(syn::UsePath {
            ident: Ident::new(seg, Span::call_site()),
            colon2_token: syn::token::PathSep::default(),
            tree: Box::new(tree),
        });
    }
    tree
}

pub(crate) fn write_output(_generated_dir: &Path, output: &[(PathBuf, String)]) -> Result<()> {
    for (path, content) in output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

pub(crate) fn run_check(output: &[(PathBuf, String)]) -> Result<()> {
    let mut dirty = false;
    for (path, expected) in output {
        let actual = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("MISSING: {}", path.display());
                dirty = true;
                continue;
            }
        };
        if actual != *expected {
            eprintln!("STALE:   {}", path.display());
            dirty = true;
        }
    }
    if dirty {
        bail!("Generated files are stale. Run `cargo xtask sync-sdk` to regenerate, then commit.");
    }
    println!("sync-sdk: all generated files up to date");
    Ok(())
}
