#!/usr/bin/env python3
"""Assert converted operations' oauth2 scope lists match their handlers'
action extractors (M1.4a). Rules:

  R1  non-empty ("oauth2" = [scopes]) <=> the handler's action-extractor
      action set equals exactly those scopes (both directions).
  R2  empty ("oauth2" = []) => handler has NO action extractor.
  R3  no operation carries BOTH x-required-permission and an oauth2
      requirement (mixed worlds).
  R4  every oauth2 requirement is paired with ("developer_token" = []).
  R5  multiple ("oauth2" = [...]) groups on one operation encode OR
      alternatives (inline authorize_any enforcement): the handler must use
      NO action extractor but must take Extension<AccessAuthority> (and the
      file must call authorize_any); each group carries exactly one scope;
      no duplicate alternatives; and every declared scope (single- or
      multi-group) must exist in the built-in catalog map. Operations
      carrying x-action-dynamic must declare exactly one empty oauth2 group
      (their requirement is registration data, enforced at runtime).
      The catalog check covers the closed built-in catalog only — dynamic
      plugin/surface scopes never appear in route declarations.

Unconverted operations (bearer_token + x-required-permission) are ignored
except by R3 — transition tolerance for the M1.4b window. Non-vacuity:
empty extractor map, empty catalog map, or zero converted operations is a
hard error. Parsing follows ci/verify_db_access_policy.py's
balanced-attribute-scan pattern (standalone copy, no cross-import).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ACTION_RS = ROOT / "crates/ui/web-api/src/middleware/action.rs"
CATALOG_RS = ROOT / "crates/shared/types/src/access/catalog.rs"
ROUTES_DIR = ROOT / "crates/ui/web-api/src/routes"

UTOIPA_ATTR = "#[utoipa::path("


def _capture_balanced(text: str, start: int) -> str:
    """Return the text of a (...) group starting at the '(' at `start`,
    inclusive, honouring double-quoted strings with escapes."""
    depth = 0
    i = start
    in_str = False
    while i < len(text):
        ch = text[i]
        if in_str:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_str = False
        elif ch == '"':
            in_str = True
        elif ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
        i += 1
    raise ValueError(f"unbalanced parentheses at offset {start}")


def _capture_braced(text: str, start: int) -> str:
    """Like _capture_balanced but for a `{...}` group starting at `start`."""
    depth = 0
    i = start
    in_str = False
    while i < len(text):
        ch = text[i]
        if in_str:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_str = False
        elif ch == '"':
            in_str = True
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
        i += 1
    raise ValueError(f"unbalanced braces at offset {start}")


def _parse_extractor_map(action_rs: str) -> dict[str, str]:
    """`CanReadHosts => actions::HOSTS_READ` pairs from action_extractor!.

    Bounded to the macro invocation's own brace group — an unbounded
    to-end-of-file scan would absorb any later `Ident => actions::CONST`
    shaped code into the map.
    """
    marker = "action_extractor! {"
    body_start = action_rs.find(marker)
    if body_start == -1:
        return {}
    body = _capture_braced(action_rs, body_start + len(marker) - 1)
    return dict(re.findall(r"(\w+)\s*=>\s*actions::(\w+)", body))


def _parse_catalog_map(catalog_rs: str) -> dict[str, str]:
    """CONST ident -> "resource:verb" from the access_catalog! invocation.

    Per-resource blocks are captured by balanced braces and verb tuples
    matched with a whitespace-tolerant (multi-line-safe) regex — a plain
    line-based regex silently DROPS any tuple rustfmt ever wraps.
    """
    marker = "access_catalog! {"
    inv_start = catalog_rs.find(marker)
    if inv_start == -1:
        return {}
    body = _capture_braced(catalog_rs, inv_start + len(marker) - 1)
    out: dict[str, str] = {}
    for res_m in re.finditer(r"\w+,\s*\"([^\"]+)\",\s*\{", body):
        resource = res_m.group(1)
        block = _capture_braced(body, res_m.end() - 1)
        for verb_m in re.finditer(
            r"\w+\s*=>\s*\(\s*\"([^\"]+)\"\s*,\s*\w+\s*,\s*(\w+)\s*,", block
        ):
            out[verb_m.group(2)] = f"{resource}:{verb_m.group(1)}"
    return out


FN_RE = re.compile(r"(?:pub(?:\([^)]*\))?\s+)?async\s+fn\s+\w+")


def _iter_operations(source: str):
    """Yield (attr_text, fn_signature_text) per #[utoipa::path(...)] item.

    Between the utoipa attr and its handler only further attributes appear
    (dominant repo shape: `#[tracing::instrument(skip_all)]` in between), so
    the FIRST `async fn` after the attr IS the decorated handler — no
    attribute-by-attribute skipping needed.
    """
    idx = 0
    while True:
        idx = source.find(UTOIPA_ATTR, idx)
        if idx == -1:
            return
        paren = idx + len(UTOIPA_ATTR) - 1
        attr = _capture_balanced(source, paren)
        cursor = paren + len(attr)
        fn_m = FN_RE.search(source, cursor)
        if not fn_m:
            return
        sig_open = source.index("(", fn_m.end())
        signature = fn_m.group(0) + _capture_balanced(source, sig_open)
        yield attr, signature
        idx = cursor


def _oauth2_groups(attr: str) -> list[list[str]] | None:
    """One scope list per ("oauth2" = [...]) requirement group, in
    declaration order; None when the attr declares no oauth2 requirement
    at all. Multiple groups encode OR alternatives (rule R5)."""
    groups = [
        re.findall(r"\"([^\"]+)\"", m.group(1))
        for m in re.finditer(r"\"oauth2\"\s*=\s*\[([^\]]*)\]", attr)
    ]
    return groups if groups else None


def _action_module_imports(source: str) -> set[str]:
    """Names actually imported (or fully-qualified-referenced) from
    `middleware::action` in this file.

    Handles both spellings that appear in this codebase:
      - a braced, optionally-aliased `use ...::action::{A, B as C, ...};`
        list — each entry is recorded under its PRE-alias name (the
        identifier `extractor_actions` keys off; if a name is aliased, the
        literal text appearing at its use sites is the alias instead, so
        the pre-alias name simply won't be found there by the `\\bname\\b`
        signature scan below — recording it here is harmless either way);
      - an unbraced `use ...::action::Name;` import, or a fully-qualified
        `middleware::action::Name` call/type site with no `use` at all —
        both look identical after the `middleware::action::` marker, so
        one branch covers them.
    """
    imported: set[str] = set()
    for m in re.finditer(r"middleware::action::(\{[^}]*\}|\w+)", source):
        group = m.group(1)
        if group.startswith("{"):
            for entry in group[1:-1].split(","):
                name = entry.strip().split(" as ")[0].strip()
                if name:
                    imported.add(name)
        else:
            imported.add(group)
    return imported


def _check_file(
    path: str,
    source: str,
    extractor_actions: dict[str, str],
    catalog_actions: set[str],
) -> tuple[list[str], int]:
    """Return (violations, converted_operation_count) for one route file.

    Extractor names are attributed ONLY when that SAME name is actually
    imported from `middleware::action` in this file (see
    `_action_module_imports`) — the legacy `middleware::permission` module
    defines same-named extractors (`CanManageCommands`, `CanUpdateHosts`,
    `CanTriggerChecks`) still referenced by callers of that module. A
    file cannot import the same unqualified name from both modules (compile
    error), but it CAN import *different* names from each module in the
    same file — e.g. `action::{AccessAuthority, authorize_any}` alongside
    `permission::CanManageCommands` — which a bare `"middleware::action" in
    source` file-level flag mis-set as "uses the action module" for every
    bare-name match, misattributing the legacy extractor's action to the
    file. Keying attribution on the actual per-name import list closes that
    hole while still attributing every genuinely converted extractor.
    """
    imported = _action_module_imports(source)
    uses_action_module = bool(imported)
    violations: list[str] = []
    converted = 0
    for attr, signature in _iter_operations(source):
        if not uses_action_module:
            if "x-required-permission" in attr and _oauth2_groups(attr) is not None:
                violations.append(
                    f"{path}: R3 operation mixes x-required-permission with oauth2"
                )
            continue
        groups = _oauth2_groups(attr)
        has_ext = "x-required-permission" in attr
        dynamic = '"x-action-dynamic"' in attr
        used = sorted(
            {
                extractor_actions[name]
                for name in extractor_actions
                if name in imported and re.search(rf"\b{name}\b", signature)
            }
        )
        if groups is not None and has_ext:
            violations.append(f"{path}: R3 operation mixes x-required-permission with oauth2")
        if dynamic and groups != [[]]:
            violations.append(
                f"{path}: R5 x-action-dynamic operation must declare exactly one empty oauth2 group"
            )
        if groups is None:
            if used:
                violations.append(
                    f"{path}: R1 handler uses action extractor(s) {used} but declares no oauth2 requirement"
                )
            continue
        converted += 1
        if '"developer_token"' not in attr:
            violations.append(f"{path}: R4 oauth2 requirement without developer_token pairing")
        for scope in (scope for group in groups for scope in group):
            if scope not in catalog_actions:
                violations.append(f"{path}: R5 declared scope {scope!r} not in the action catalog")
        if len(groups) == 1:
            scopes = groups[0]
            if scopes:
                if sorted(scopes) != used:
                    violations.append(
                        f"{path}: R1 oauth2 scopes {sorted(scopes)} != extractor actions {used}"
                    )
            elif used:
                violations.append(
                    f"{path}: R2 empty-scope operation must not use action extractors ({used})"
                )
        else:
            if used:
                violations.append(
                    f"{path}: R5 OR-declared operation must not use action extractors ({used})"
                )
            if len({tuple(group) for group in groups}) != len(groups):
                violations.append(f"{path}: R5 duplicate OR alternatives declared")
            for group in groups:
                if len(group) != 1:
                    violations.append(
                        f"{path}: R5 each OR alternative must carry exactly one scope (got {group})"
                    )
            if not re.search(r"\bAccessAuthority\b", signature):
                violations.append(
                    f"{path}: R5 OR-declared operation must take Extension<AccessAuthority> for inline enforcement"
                )
            if "authorize_any" not in source:
                violations.append(
                    f"{path}: R5 file declares OR operations but never calls authorize_any"
                )
    return violations, converted


def main() -> int:
    action_src = ACTION_RS.read_text(encoding="utf-8")
    catalog_src = CATALOG_RS.read_text(encoding="utf-8")
    name_to_const = _parse_extractor_map(action_src)
    const_to_action = _parse_catalog_map(catalog_src)
    if not name_to_const:
        print("verify_action_security_declarations: extractor map parsed EMPTY", file=sys.stderr)
        return 1
    if not const_to_action:
        print("verify_action_security_declarations: catalog map parsed EMPTY", file=sys.stderr)
        return 1
    unknown = {c for c in name_to_const.values() if c not in const_to_action}
    if unknown:
        print(f"verify_action_security_declarations: extractor consts not in catalog: {sorted(unknown)}", file=sys.stderr)
        return 1
    extractor_actions = {n: const_to_action[c] for n, c in name_to_const.items()}
    catalog_actions = set(const_to_action.values())
    violations: list[str] = []
    converted_total = 0
    for rs in sorted(ROUTES_DIR.rglob("*.rs")):
        vs, converted = _check_file(
            str(rs.relative_to(ROOT)), rs.read_text(encoding="utf-8"), extractor_actions, catalog_actions
        )
        violations.extend(vs)
        converted_total += converted
    if converted_total == 0:
        print("verify_action_security_declarations: zero converted operations found (vacuous run)", file=sys.stderr)
        return 1
    if violations:
        for v in violations:
            print(v, file=sys.stderr)
        print(f"verify_action_security_declarations: {len(violations)} violation(s)", file=sys.stderr)
        return 1
    print(f"verify_action_security_declarations: OK ({converted_total} converted operations checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
