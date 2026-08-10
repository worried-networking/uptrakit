#!/usr/bin/env python3
"""Verify every tracked .rs file is reachable from a Cargo target root via `mod` resolution.

Failure classes (reported separately; see docs/development/quality-gates.md):
  Class A — tracked .rs file visited by no resolution walk: an orphan module.
            The compiler never sees such a file; `warnings = "deny"` and clippy
            are inert on it, and edits to it silently ship nothing.
  Class B — a `mod name;` declaration that resolves to no file: a resolver gap
            in this gate (or a genuinely broken declaration). Fails loudly so a
            gap can never silently make a live file look orphaned.

Exit codes (matching ci/check_plugin_semantic_boundary.py):
  0 — clean; 1 — Class A violations; 2 — Class B resolver gaps or config errors
  (message prefix distinguishes `resolver gap:` from `environment error:` /
  `config error:`).

Consequence of this gate: module decompositions must land atomically — a new
file and its `mod` declaration belong in the same commit.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:  # Python < 3.11
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        tomllib = None  # type: ignore[assignment]

EXIT_OK = 0
EXIT_VIOLATIONS = 1
EXIT_CONFIG_ERROR = 2

# Module-shape events over sanitized source: `mod name;` (file module),
# `mod name {` (inline module), and bare braces for depth tracking.
# Attributes (including #[cfg(...)]) are deliberately NOT evaluated: the gate
# visits every declared module regardless of feature/platform, which is what
# makes it immune to feature-combination false positives.
EVENT_RE = re.compile(
    r"\bmod[ \t\n]+([A-Za-z_][A-Za-z0-9_]*)[ \t\n]*([;{])|([{}])"
)
# #[path = "..."] immediately preceding a `mod name;`. Matched against the
# RAW source (the sanitizer blanks string interiors, including attribute
# paths), as is INCLUDE_RE; only brace/mod tracking uses sanitized text.
PATH_ATTR_RE = re.compile(
    r"#\[path[ \t]*=[ \t]*\"([^\"]+)\"\][ \t]*\n[ \t]*"
    r"(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;"
)
# include!("literal.rs") — treat the literal as a visit.
INCLUDE_RE = re.compile(r"include!\s*\(\s*\"([^\"]+\.rs)\"\s*\)")


def sanitize(src: str) -> str:
    """Blank out comment and string interiors, length-preserved.

    Keeps braces inside strings/comments from corrupting module-depth
    tracking. Handles //, nested /* */, "..." with escapes, char literals,
    lifetimes, and raw strings r#"..."#.
    """
    out = list(src)
    i, n = 0, len(src)

    def blank(a: int, b: int) -> None:
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j == -1 else j
            blank(i, j)
            i = j
        elif c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == "r" and i + 1 < n and src[i + 1] in "#\"" and (i == 0 or not src[i - 1].isalnum() and src[i - 1] != "_"):
            j = i + 1
            hashes = 0
            while j < n and src[j] == "#":
                hashes, j = hashes + 1, j + 1
            if j < n and src[j] == '"':
                close = '"' + "#" * hashes
                k = src.find(close, j + 1)
                k = n if k == -1 else k + len(close)
                blank(i + 1, k)
                i = k
            else:
                i += 1
        elif c == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                elif src[j] == '"':
                    j += 1
                    break
                else:
                    j += 1
            blank(i + 1, j - 1)
            i = j
        elif c == "'":
            # char literal ('x', '\n') vs lifetime ('a) — a closing quote
            # within 3 chars (or an escape) marks a literal.
            if i + 1 < n and src[i + 1] == "\\":
                j = src.find("'", i + 2)
                i = (n if j == -1 else j + 1)
            elif i + 2 < n and src[i + 2] == "'":
                i += 3
            else:
                i += 1  # lifetime — skip the quote only
        else:
            i += 1
    return "".join(out)


def fail_env(message: str) -> int:
    print(f"no-orphan-modules environment error: {message}", file=sys.stderr)
    return EXIT_CONFIG_ERROR


def load_allowlist(path: Path) -> tuple[set[str], set[str], str | None]:
    """Returns (allowed_paths, allowed_decls, error). Entries need a reason."""
    if not path.exists():
        return set(), set(), None
    if tomllib is None:
        return set(), set(), "Python tomllib is unavailable. Install tomli: pip install tomli (Python < 3.11)"
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:  # type: ignore[union-attr]
        return set(), set(), f"unparseable allowlist {path}: {exc}"
    allowed_paths: set[str] = set()
    allowed_decls: set[str] = set()
    for entry in data.get("allow", []):
        reason = entry.get("reason", "")
        if not isinstance(reason, str) or not reason.strip():
            return set(), set(), f"allowlist entry {entry!r} is missing a non-empty `reason`"
        if "path" in entry:
            allowed_paths.add(entry["path"])
        elif "decl" in entry:
            allowed_decls.add(entry["decl"])
        else:
            return set(), set(), f"allowlist entry {entry!r} needs `path` or `decl`"
    return allowed_paths, allowed_decls, None


def target_roots(root: Path) -> tuple[list[Path], str | None]:
    """All target src_paths from cargo metadata, plus every crate's build.rs."""
    try:
        out = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except FileNotFoundError:
        return [], "cargo not found on PATH"
    except subprocess.CalledProcessError as exc:
        return [], f"cargo metadata failed: {exc.stderr.strip()}"
    meta = json.loads(out)
    roots: list[Path] = []
    for pkg in meta["packages"]:
        for target in pkg["targets"]:
            roots.append(Path(target["src_path"]))
        build_rs = Path(pkg["manifest_path"]).parent / "build.rs"
        if build_rs.is_file():
            roots.append(build_rs)
    return roots, None


def visit(
    file: Path,
    visited: set[Path],
    unresolved: list[tuple[Path, str]],
    roots: frozenset[Path],
) -> None:
    file = file.resolve()
    if file in visited or not file.is_file():
        return
    visited.add(file)
    raw = file.read_text(encoding="utf-8", errors="replace")
    src = sanitize(raw)
    parent = file.parent
    # Crate roots (every cargo target src_path: lib.rs, main.rs, each
    # tests/*.rs, examples/*.rs, benches/*.rs, build.rs) and mod.rs resolve
    # child modules in their own directory; any other `foo.rs` resolves them
    # in `foo/`. Declarations inside an inline `mod a { ... }` resolve one
    # directory deeper per nesting level (`mod a { mod b; }` -> `a/b.rs`).
    is_root = file in roots or file.name in ("lib.rs", "main.rs", "mod.rs")
    base = parent if is_root else parent / file.stem

    explicit = {name: parent / rel for rel, name in PATH_ATTR_RE.findall(raw)}
    for rel in INCLUDE_RE.findall(raw):
        visit(parent / rel, visited, unresolved, roots)

    depth = 0
    inline_stack: list[tuple[int, str]] = []  # (depth at open, mod name)
    for match in EVENT_RE.finditer(src):
        name, shape, brace = match.groups()
        if brace == "{":
            depth += 1
        elif brace == "}":
            depth -= 1
            if inline_stack and inline_stack[-1][0] == depth:
                inline_stack.pop()
        elif shape == "{":
            inline_stack.append((depth, name))
            depth += 1
        else:  # `mod name;`
            if name in explicit:
                visit(explicit[name], visited, unresolved, roots)
                continue
            subdir = base
            for _, seg in inline_stack:
                subdir = subdir / seg
            as_file = subdir / f"{name}.rs"
            as_dir = subdir / name / "mod.rs"
            if as_file.is_file():
                visit(as_file, visited, unresolved, roots)
            elif as_dir.is_file():
                visit(as_dir, visited, unresolved, roots)
            else:
                unresolved.append((file, name))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root (default: cwd)")
    parser.add_argument(
        "--allowlist",
        default="ci/no_orphan_modules_allowlist.toml",
        help="allowlist TOML, relative to --root",
    )
    args = parser.parse_args()
    root = Path(args.root).resolve()

    allowed_paths, allowed_decls, allow_err = load_allowlist(root / args.allowlist)
    if allow_err is not None:
        print(f"no-orphan-modules config error: {allow_err}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    try:
        tracked_out = subprocess.run(
            ["git", "ls-files", "-z", "--", "*.rs"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (FileNotFoundError, subprocess.CalledProcessError) as exc:
        return fail_env(f"git ls-files failed: {exc}")
    tracked = {
        (root / p).resolve()
        for p in tracked_out.split("\0")
        if p and not p.startswith("ci/testdata/")
    }

    tracked_rel = {str(p.relative_to(root)) for p in tracked}
    stale_allow = sorted(allowed_paths - tracked_rel)
    if stale_allow:
        for entry in stale_allow:
            print(
                f"no-orphan-modules config error: allowlist path `{entry}` matches no tracked file — "
                "remove the stale entry (a dead entry silently pre-authorizes a future orphan at that path)",
                file=sys.stderr,
            )
        return EXIT_CONFIG_ERROR

    roots, root_err = target_roots(root)
    if root_err is not None:
        return fail_env(root_err)

    root_set = frozenset(p.resolve() for p in roots)
    visited: set[Path] = set()
    unresolved: list[tuple[Path, str]] = []
    for target_root in roots:
        visit(target_root, visited, unresolved, root_set)

    real_unresolved = [
        (f, name)
        for f, name in unresolved
        if f"{f.relative_to(root)}::{name}" not in allowed_decls
    ]
    orphans = sorted(
        str(p.relative_to(root))
        for p in tracked - visited
        if str(p.relative_to(root)) not in allowed_paths
    )

    if real_unresolved:
        for f, name in real_unresolved:
            print(
                f"resolver gap: `mod {name};` in {f.relative_to(root)} resolves to no file "
                f"(allowlist as decl = \"{f.relative_to(root)}::{name}\" with a reason if intentional)",
                file=sys.stderr,
            )
        return EXIT_CONFIG_ERROR
    if orphans:
        for p in orphans:
            print(
                f"orphan module: {p} is tracked but no `mod` declaration reaches it — "
                "the compiler never sees this file. Declare it (same commit) or delete it.",
                file=sys.stderr,
            )
        return EXIT_VIOLATIONS
    print(f"no-orphan-modules clean ({len(visited & tracked)} tracked files reachable)")
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
