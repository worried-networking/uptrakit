use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::sync_sdk::{collect_module, run_check, write_output};

pub fn run(workspace_root: &Path, check: bool) -> Result<()> {
    let surfaces_src = workspace_root.join("crates/shared/surfaces/src");
    let wire_src = workspace_root.join("crates/shared/wire/src");
    let shared_types_src = workspace_root.join("crates/shared/types/src");
    let web_api_types_src = workspace_root.join("crates/shared/web-api-types/src");
    let out_root = workspace_root.join("crates/shared/openapi-client/src/generated");

    let mut output: Vec<(PathBuf, String)> = Vec::new();

    // 1. Surfaces — rewrite self-references only
    collect_module(
        &surfaces_src,
        &out_root.join("surfaces"),
        &[(&["crate"], &["crate", "generated", "surfaces"])],
        &mut output,
    )?;

    // 2. Wire — self-references first, then external deps (order matters: crate:: rule
    //    must run first so it doesn't corrupt paths produced by later rules)
    collect_module(
        &wire_src,
        &out_root.join("wire"),
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

    // 3. Shared-types — rewrite self-references only (no internal path deps)
    collect_module(
        &shared_types_src,
        &out_root.join("shared_types"),
        &[(&["crate"], &["crate", "generated", "shared_types"])],
        &mut output,
    )?;

    // 4. Web-api-types — self-references first, then external deps (order matters: crate::
    //    rule must run first so it doesn't corrupt paths produced by external dep rules)
    collect_module(
        &web_api_types_src,
        &out_root.join("types"),
        &[
            (&["crate"], &["crate", "generated", "types"]),
            (&["uptrakit_wire"], &["crate", "generated", "wire"]),
            (
                &["uptrakit_shared_types"],
                &["crate", "generated", "shared_types"],
            ),
            (&["uptrakit_surfaces"], &["crate", "generated", "surfaces"]),
        ],
        &mut output,
    )?;

    // 5. Top-level generated/mod.rs
    output.push((
        out_root.join("mod.rs"),
        "pub mod shared_types;\npub mod surfaces;\npub mod types;\npub mod wire;\n".to_string(),
    ));

    // 6. Fix serde default string literals (syn cannot touch string attributes)
    for (_, content) in &mut output {
        *content = content
            .replace(
                "\"crate::default_enabled\"",
                "\"crate::generated::types::default_enabled\"",
            )
            .replace(
                "\"crate::default_featured\"",
                "\"crate::generated::types::default_featured\"",
            );
    }

    // 7. Strip uptrakit_shared_macros use-imports (macro available via #[macro_use])
    for (_, content) in &mut output {
        let filtered: Vec<&str> = content
            .lines()
            .filter(|l| !l.contains("uptrakit_shared_macros"))
            .collect();
        *content = filtered.join("\n");
        if !content.ends_with('\n') {
            content.push('\n');
        }
    }

    // 8. Post-process generated files for compilation compatibility.
    for (path, content) in &mut output {
        // wire_validate_impls.rs has `_ =>` arms on #[non_exhaustive] enums that
        // are exhaustively matched here (same crate), triggering unreachable_patterns.
        if path.ends_with("wire_validate_impls.rs") {
            *content = format!("#![allow(unreachable_patterns)]\n{content}");
        }

        // Replace runnable fenced code blocks in doc comments with ```ignore.
        // Generated doc comments reference old crate names (uptrakit_web_api_types etc.)
        // that no longer exist in this crate — they'd fail as doctests.
        // Rustdoc compiles both "```" and "```rust" as Rust doctests.
        *content = content
            .replace("//! ```rust\n", "//! ```ignore\n")
            .replace("/// ```rust\n", "/// ```ignore\n")
            .replace("//! ``` rust\n", "//! ```ignore\n")
            .replace("/// ``` rust\n", "/// ```ignore\n")
            // Plain ``` (no language tag) is also compiled as Rust by rustdoc.
            .replace("//! ```\n", "//! ```ignore\n")
            .replace("/// ```\n", "/// ```ignore\n");
    }

    if check {
        run_check(&output)
    } else {
        write_output(&out_root, &output)
    }
}
