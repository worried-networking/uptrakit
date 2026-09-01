#!/usr/bin/env python3
"""Select plugin crates affected by a git diff range, for the diff-scoped bare-crate clippy sweep.

CI's "Bare-crate clippy sweep (plugin crates)" step (`.github/workflows/ci.yml`) always runs
`cargo clippy --all-targets -p <crate>` for every crate under `crates/plugins/*/*/`, because a
workspace-wide `cargo clippy --all-features` unifies feature flags across crates and can hide a
crate that would fail to compile in isolation (e.g. it imports a feature-gated item from a
dependency without enabling that feature itself). The full sweep is cheap in CI but too slow for a
pre-push hook, so this module narrows it to the plugin crates that could plausibly be affected by
a given commit range: the plugin crate itself changed, one of its transitive workspace-internal
dependencies changed, or a workspace-wide file (`Cargo.toml`/`Cargo.lock`) changed.

Core selection logic (`select_affected_plugin_crates` and its helpers) is pure — it takes an
already-parsed `cargo metadata` dict and a list of changed paths, and returns crate names. It does
not shell out, so tests can inject fixtures without running git or cargo. Only `main()` performs
I/O (git, cargo metadata) and printing.

CLI: `python3 ci/bare_crate_select.py <base> <head>` prints selected plugin crate names, one per
line, sorted and deduped. Empty output (exit 0) when nothing is selected. Non-zero exit with a
stderr message on git/cargo failures.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import PurePosixPath

EXIT_OK = 0
EXIT_USAGE_ERROR = 2
EXIT_TOOL_ERROR = 1

# A "plugin crate" is a workspace member whose manifest lives two directory levels below
# crates/plugins/ (e.g. crates/plugins/infrastructure/core/Cargo.toml). Derived from the real
# layout at select time, never hand-maintained.
PLUGIN_MANIFEST_RE = re.compile(r"^crates/plugins/[^/]+/[^/]+/Cargo\.toml$")

# Workspace-wide files: a change here can affect any plugin crate's build (feature unification,
# dependency versions, lockfile resolution), so every plugin crate is selected.
ROOT_ESCALATION_PATHS = frozenset({"Cargo.toml", "Cargo.lock"})


def _rel_manifest_path(manifest_path: str, workspace_root: str) -> str:
    """POSIX-style manifest path relative to the workspace root."""
    rel = PurePosixPath(manifest_path).relative_to(PurePosixPath(workspace_root))
    return rel.as_posix()


def workspace_member_packages(metadata: dict) -> dict[str, dict]:
    """Map workspace member package id -> package dict, restricted to workspace members.

    `metadata["packages"]` includes every resolved package (workspace members and every
    third-party/transitive dependency); `metadata["workspace_members"]` is the authoritative id
    list for what's actually part of this workspace.
    """
    member_ids = set(metadata["workspace_members"])
    return {pkg["id"]: pkg for pkg in metadata["packages"] if pkg["id"] in member_ids}


def plugin_crate_ids(members: dict[str, dict], workspace_root: str) -> set[str]:
    """Workspace member ids whose manifest matches crates/plugins/*/*/Cargo.toml."""
    return {
        pid
        for pid, pkg in members.items()
        if PLUGIN_MANIFEST_RE.match(_rel_manifest_path(pkg["manifest_path"], workspace_root))
    }


def member_crate_dir(pkg: dict, workspace_root: str) -> str:
    """Crate directory (POSIX, relative to workspace root, no trailing slash) for a member."""
    parent = PurePosixPath(_rel_manifest_path(pkg["manifest_path"], workspace_root)).parent
    return "" if str(parent) == "." else parent.as_posix()


def changed_member_ids(
    changed_paths: list[str], members: dict[str, dict], workspace_root: str
) -> set[str]:
    """Map changed file paths to owning workspace member ids via longest-prefix match.

    A changed path belongs to the member whose crate directory is the longest matching prefix.
    Paths matching no member directory (docs, scripts, a deleted crate's leftover path) are
    ignored, not errors — deletions are handled the same way as any other changed path, since
    this function only ever sees a path string, never touches the filesystem.
    """
    dirs = {pid: member_crate_dir(pkg, workspace_root) for pid, pkg in members.items()}
    result: set[str] = set()
    for raw_path in changed_paths:
        path = PurePosixPath(raw_path).as_posix()
        best_pid: str | None = None
        best_len = -1
        for pid, crate_dir in dirs.items():
            if not crate_dir:
                continue
            if path == crate_dir or path.startswith(crate_dir + "/"):
                if len(crate_dir) > best_len:
                    best_len = len(crate_dir)
                    best_pid = pid
        if best_pid is not None:
            result.add(best_pid)
    return result


def workspace_internal_dependency_closure(
    metadata: dict, members: dict[str, dict]
) -> dict[str, set[str]]:
    """member id -> transitive workspace-internal dependency ids (normal + dev + build kinds).

    Uses `metadata["resolve"]["nodes"]`, which carries the fully resolved dependency graph
    (including dev/build edges) keyed by package id; `node["deps"]` entries are filtered down to
    ids that are themselves workspace members.
    """
    member_ids = set(members)
    nodes_by_id = {node["id"]: node for node in metadata["resolve"]["nodes"]}

    direct: dict[str, set[str]] = {}
    for pid in member_ids:
        node = nodes_by_id.get(pid)
        deps: set[str] = set()
        if node is not None:
            for dep in node.get("deps", []):
                if dep["pkg"] in member_ids:
                    deps.add(dep["pkg"])
        direct[pid] = deps

    closure: dict[str, set[str]] = {}
    for pid in member_ids:
        seen: set[str] = set()
        stack = list(direct[pid])
        while stack:
            current = stack.pop()
            if current in seen:
                continue
            seen.add(current)
            stack.extend(direct.get(current, set()) - seen)
        closure[pid] = seen
    return closure


def select_affected_plugin_crates(metadata: dict, changed_paths: list[str]) -> set[str]:
    """Pure core: (parsed `cargo metadata` dict, changed file paths) -> selected plugin crate names.

    Selection rules, in order:
      1. If `Cargo.toml` or `Cargo.lock` (workspace root) changed, select every plugin crate.
      2. Otherwise, select a plugin crate P iff P itself changed, or any changed workspace member
         is in P's transitive workspace-internal dependency closure (normal + dev + build).
    """
    workspace_root = metadata["workspace_root"]
    members = workspace_member_packages(metadata)
    plugins = plugin_crate_ids(members, workspace_root)
    if not plugins:
        return set()

    normalized_changed = {PurePosixPath(p).as_posix() for p in changed_paths}
    if normalized_changed & ROOT_ESCALATION_PATHS:
        return {members[pid]["name"] for pid in plugins}

    changed_members = changed_member_ids(changed_paths, members, workspace_root)
    closure = workspace_internal_dependency_closure(metadata, members)

    selected_ids = {
        pid
        for pid in plugins
        if pid in changed_members or (closure.get(pid, set()) & changed_members)
    }
    return {members[pid]["name"] for pid in selected_ids}


def _run_git_diff(base: str, head: str) -> list[str]:
    """Changed file paths between two refs, via the two-arg `git diff` form (works with any SHAs)."""
    try:
        result = subprocess.run(
            ["git", "diff", "--name-only", base, head],
            capture_output=True,
            text=True,
        )
    except FileNotFoundError as exc:
        raise RuntimeError(f"git not found: {exc}") from exc
    if result.returncode != 0:
        raise RuntimeError(
            f"git diff --name-only {base} {head} failed: {result.stderr.strip()}"
        )
    return [line for line in result.stdout.split("\n") if line]


def _run_cargo_metadata() -> dict:
    """Full (non `--no-deps`) `cargo metadata`, so the resolved dependency graph is available."""
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            capture_output=True,
            text=True,
        )
    except FileNotFoundError as exc:
        raise RuntimeError(f"cargo not found: {exc}") from exc
    if result.returncode != 0:
        raise RuntimeError(f"cargo metadata --format-version 1 failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {sys.argv[0]} <base> <head>", file=sys.stderr)
        return EXIT_USAGE_ERROR

    base, head = argv
    try:
        changed_paths = _run_git_diff(base, head)
        metadata = _run_cargo_metadata()
    except (RuntimeError, json.JSONDecodeError) as exc:
        print(f"bare_crate_select: {exc}", file=sys.stderr)
        return EXIT_TOOL_ERROR

    selected = select_affected_plugin_crates(metadata, changed_paths)
    for name in sorted(selected):
        print(name)
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
