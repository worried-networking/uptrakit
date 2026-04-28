use anyhow::{Context, Result, bail};
use proc_macro2::Span;
use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{File, Ident, Path as SynPath, visit_mut::VisitMut};
use walkdir::WalkDir;

pub fn run(workspace_root: &Path, check: bool, commit: bool) -> Result<()> {
    let surfaces_src = workspace_root.join("crates/shared/surfaces/src");
    let wire_src = workspace_root.join("crates/shared/wire/src");
    let shared_types_src = workspace_root.join("crates/shared/types/src");
    let sdk_generated = workspace_root.join("crates/shared/service-sdk/src/generated");

    // Collect output files: path → content
    let mut output: Vec<(PathBuf, String)> = Vec::new();

    // 1. Surfaces — no path rewriting needed (no internal deps)
    collect_module(
        &surfaces_src,
        &sdk_generated.join("surfaces"),
        &[],
        &mut output,
    )?;

    // 2. Shared-types — rewrite self-references only (no internal path deps)
    collect_module(
        &shared_types_src,
        &sdk_generated.join("shared_types"),
        &[(&["crate"], &["crate", "generated", "shared_types"])],
        &mut output,
    )?;

    // 3. Wire — rewrite uptrakit_surfaces and uptrakit_shared_types
    //    (wire has workspace path deps on both)
    collect_module(
        &wire_src,
        &sdk_generated.join("wire"),
        &[
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
        let dst_path = dst_dir.join(rel);
        let src_content = fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;

        let content = rewrite_paths(&src_content, rewrites)
            .with_context(|| format!("rewriting {}", entry.path().display()))?;

        output.push((dst_path, content));
    }
    Ok(())
}

/// Parse `src`, apply all path segment rewrites, return formatted Rust source.
fn rewrite_paths(src: &str, rewrites: &[(&[&str], &[&str])]) -> Result<String> {
    let mut file: File = syn::parse_str(src).context("parsing source file")?;

    for (from, to) in rewrites {
        let mut rewriter = PathRewriter { from, to };
        rewriter.visit_file_mut(&mut file);
    }

    let tokens = file.to_token_stream();
    let formatted = prettyplease::unparse(&syn::parse2(tokens)?);
    Ok(formatted)
}

struct PathRewriter<'a> {
    from: &'a [&'a str],
    to: &'a [&'a str],
}

impl VisitMut for PathRewriter<'_> {
    fn visit_path_mut(&mut self, path: &mut SynPath) {
        // Recurse into nested paths first
        syn::visit_mut::visit_path_mut(self, path);

        let seg_idents: Vec<String> = path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();

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
}

pub(crate) fn write_output(generated_dir: &Path, output: &[(PathBuf, String)]) -> Result<()> {
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
        bail!(
            "Generated files are stale. Run `cargo xtask sync-sdk` to regenerate, then commit."
        );
    }
    println!("sync-sdk: all generated files up to date");
    Ok(())
}
