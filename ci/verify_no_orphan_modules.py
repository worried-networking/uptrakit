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
# #[path = "..."] preceding a `mod name;`, tolerating arbitrary whitespace
# (including newlines) and any run of further attributes in between (e.g. a
# `#[cfg(...)]` interleaved between `#[path]` and `mod`). Matched against
# comment-blanked, string-preserving sanitized source (see `attrs` in
# `visit()`), as is INCLUDE_RE; only brace/mod tracking uses fully sanitized
# text.
PATH_ATTR_RE = re.compile(
    r"#\[path[ \t]*=[ \t]*\"([^\"]+)\"\][ \t\r\n]*"
    r"(?:#\[(?:\"[^\"]*\"|[^\[\]]|\[[^\]]*\])*\][ \t\r\n]*)*"
    r"(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t\r\n]+([A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*;"
)
# include!("literal.rs") — treat the literal as a visit.
INCLUDE_RE = re.compile(r"include!\s*\(\s*\"([^\"]+\.rs)\"\s*\)")


def raw_string_prefix_len(src: str, i: int) -> int:
    """Length of the raw-string prefix opening at `i`, or 0 if none opens there.

    Covers `r"…"` / `r#"…"#` and their prefixed forms `br"…"` (byte) and
    `cr"…"` (C string) — all four are raw, so the `\\` escape handling of the
    ordinary string branch would run past their real terminator. `i` points at
    the first prefix character. The character before the prefix must not be an
    identifier character, or an identifier ending in `r` (`for`, `bar`)
    immediately followed by a string would be misread as a raw string.
    """
    n = len(src)
    j = i
    if src[j] in "bc":
        j += 1
        if j >= n or src[j] != "r":
            return 0
    elif src[j] != "r":
        return 0
    j += 1  # past the `r`
    if j >= n or src[j] not in "#\"":
        return 0
    prev = src[i - 1] if i > 0 else ""
    if prev.isalnum() or prev == "_":
        return 0
    return j - i


def sanitize(src: str, blank_strings: bool = True) -> str:
    """Blank out comment interiors (and, by default, string interiors),
    length-preserved.

    Keeps braces inside strings/comments from corrupting module-depth
    tracking. Handles //, nested /* */, "..." with escapes, char literals,
    lifetimes, and raw strings r#"..."# (plus the br/cr prefixed forms).
    Comments are always blanked; pass
    `blank_strings=False` to keep string-literal content intact (e.g. so
    `PATH_ATTR_RE`/`INCLUDE_RE` can still read a path/filename out of a
    string) while still blanking out any comment content, including a
    comment that itself looks like a `"..."` string.
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
        elif c in "brc" and (prefix := raw_string_prefix_len(src, i)):
            j = i + prefix
            hashes = 0
            while j < n and src[j] == "#":
                hashes, j = hashes + 1, j + 1
            if j < n and src[j] == '"':
                close = '"' + "#" * hashes
                k = src.find(close, j + 1)
                k = n if k == -1 else k + len(close)
                if blank_strings:
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
            if blank_strings:
                blank(i + 1, j - 1)
            i = j
        elif c == "'":
            # char literal ('x', '\n') vs lifetime ('a) — a closing quote
            # within 3 chars (or an escape) marks a literal. Always blank
            # the interior (regardless of blank_strings): an unblanked
            # brace inside a char literal — e.g. '{' or b'{' — corrupts
            # brace/mod depth tracking in `visit()`, and the interior is
            # never meaningful to PATH_ATTR_RE/INCLUDE_RE.
            if i + 1 < n and src[i + 1] == "\\":
                # Start the search at i+3: in `'\''` the escaped quote sits at
                # i+2, and finding it would leave the real closing quote
                # unconsumed — a stray `'` that then mis-consumes the text
                # after it as another char literal.
                j = src.find("'", i + 3)
                if j == -1:
                    i = n
                else:
                    blank(i + 1, j)
                    i = j + 1
            elif i + 2 < n and src[i + 2] == "'":
                blank(i + 1, i + 2)
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
        has_path = "path" in entry
        has_decl = "decl" in entry
        if has_path and has_decl:
            return (
                set(),
                set(),
                f"allowlist entry {entry!r} has both `path` and `decl` — use two separate entries",
            )
        key = "path" if has_path else "decl" if has_decl else None
        if key is not None and not isinstance(entry[key], str):
            return (
                set(),
                set(),
                f"allowlist entry {entry!r} has a non-string `{key}`",
            )
        if has_path:
            allowed_paths.add(entry["path"])
        elif has_decl:
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
    # Comments blanked but string interiors intact, so a commented-out
    # `include!("dead.rs")` or `#[path = "dead.rs"]` cannot mask a real
    # orphan, while a genuine path/filename string is still readable.
    attrs = sanitize(raw, blank_strings=False)
    parent = file.parent
    # Crate roots (every cargo target src_path: lib.rs, main.rs, each
    # tests/*.rs, examples/*.rs, benches/*.rs, build.rs) and mod.rs resolve
    # child modules in their own directory; any other `foo.rs` resolves them
    # in `foo/`. Declarations inside an inline `mod a { ... }` resolve one
    # directory deeper per nesting level (`mod a { mod b; }` -> `a/b.rs`).
    is_root = file in roots or file.name in ("lib.rs", "main.rs", "mod.rs")
    base = parent if is_root else parent / file.stem

    explicit = {name: parent / rel for rel, name in PATH_ATTR_RE.findall(attrs)}
    for rel in INCLUDE_RE.findall(attrs):
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
    stale_decls = sorted(
        decl for decl in allowed_decls if decl.rsplit("::", 1)[0] not in tracked_rel
    )
    if stale_allow or stale_decls:
        for entry in stale_allow:
            print(
                f"no-orphan-modules config error: allowlist path `{entry}` matches no tracked file — "
                "remove the stale entry (a dead entry silently pre-authorizes a future orphan at that path)",
                file=sys.stderr,
            )
        for entry in stale_decls:
            print(
                f"no-orphan-modules config error: allowlist decl `{entry}` file half matches no tracked "
                "file — remove the stale entry (a dead entry silently pre-authorizes a future resolver "
                "gap at that path)",
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
