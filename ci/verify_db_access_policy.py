#!/usr/bin/env python3
"""Verify db_access_policy.toml against actual handler signatures in routes/.

Exit codes:
  0 — all handlers listed and classifications match actual signatures
  1 — at least one violation (unlisted handler, classification mismatch,
      or stale TOML entry)

Scope: crates/ui/web-api/src/routes/ only. Middleware is NOT checked.

Run from the repository root:
    python3 ci/verify_db_access_policy.py
"""
import sys
import re
import pathlib
import typing

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        print(
            "ERROR: tomllib not available. "
            "Install tomli: pip install tomli (Python < 3.11)",
            file=sys.stderr,
        )
        sys.exit(1)

POLICY_PATH = pathlib.Path("crates/ui/web-api/db_access_policy.toml")
ROUTES_DIR = pathlib.Path("crates/ui/web-api/src/routes")

FOCUSED_SUBSTATES = ["DbState", "AuthState", "BroadcastState", "CertState", "OidcState"]
FOCUSED_STATE_PATTERN = "|".join(f"State<{s}>" for s in FOCUSED_SUBSTATES)


class _LexState:
    def __init__(self) -> None:
        self.block_comment_depth = 0
        self.in_string = False
        self.string_escape = False
        self.in_char = False
        self.char_escape = False
        self.raw_string_hashes: int | None = None

    def is_plain_code(self) -> bool:
        return (
            self.block_comment_depth == 0
            and not self.in_string
            and not self.in_char
            and self.raw_string_hashes is None
        )


def _is_trivia_line(line: str) -> bool:
    stripped = line.lstrip()
    return (
        not stripped.strip()
        or stripped.startswith("//")
        or stripped.startswith("#[")
        or stripped.startswith("#![")
    )


def _starts_raw_string(line: str, pos: int) -> tuple[bool, int, int]:
    cursor = pos
    prefix_len = 0
    if cursor < len(line) and line[cursor] == "b":
        cursor += 1
        prefix_len += 1
    if cursor >= len(line) or line[cursor] != "r":
        return False, 0, 0
    cursor += 1
    prefix_len += 1
    hashes = 0
    while cursor < len(line) and line[cursor] == "#":
        hashes += 1
        cursor += 1
    if cursor < len(line) and line[cursor] == '"':
        return True, hashes, prefix_len + hashes + 1
    return False, 0, 0


def _starts_char_literal(line: str, pos: int) -> bool:
    if pos + 2 >= len(line):
        return False
    if line[pos] != "'":
        return False
    cursor = pos + 1
    if line[cursor] == "\\":
        cursor += 2
    else:
        cursor += 1
    return cursor < len(line) and line[cursor] == "'"


def _scan_item_line(line: str, state: _LexState) -> tuple[int, bool]:
    depth_delta = 0
    saw_item_delimiter = False
    cursor = 0

    while cursor < len(line):
        if state.raw_string_hashes is not None:
            if line[cursor] == '"' and line[cursor + 1 :].startswith(
                "#" * state.raw_string_hashes
            ):
                cursor += 1 + state.raw_string_hashes
                state.raw_string_hashes = None
                continue
            cursor += 1
            continue

        if state.in_string:
            ch = line[cursor]
            if state.string_escape:
                state.string_escape = False
            elif ch == "\\":
                state.string_escape = True
            elif ch == '"':
                state.in_string = False
            cursor += 1
            continue

        if state.in_char:
            ch = line[cursor]
            if state.char_escape:
                state.char_escape = False
            elif ch == "\\":
                state.char_escape = True
            elif ch == "'":
                state.in_char = False
            cursor += 1
            continue

        if state.block_comment_depth > 0:
            if line.startswith("/*", cursor):
                state.block_comment_depth += 1
                cursor += 2
                continue
            if line.startswith("*/", cursor):
                state.block_comment_depth -= 1
                cursor += 2
                continue
            cursor += 1
            continue

        if line.startswith("//", cursor):
            break
        if line.startswith("/*", cursor):
            state.block_comment_depth += 1
            cursor += 2
            continue

        starts_raw, raw_hashes, raw_prefix_len = _starts_raw_string(line, cursor)
        if starts_raw:
            state.raw_string_hashes = raw_hashes
            cursor += raw_prefix_len
            continue

        if line[cursor] == '"':
            state.in_string = True
            cursor += 1
            continue
        if _starts_char_literal(line, cursor):
            state.in_char = True
            cursor += 1
            continue

        if line[cursor] == "{":
            depth_delta += 1
            saw_item_delimiter = True
        elif line[cursor] == "}":
            depth_delta -= 1
        elif line[cursor] == ";":
            saw_item_delimiter = True

        cursor += 1

    return depth_delta, saw_item_delimiter


def _split_top_level_items(text: str) -> list[str]:
    """Split Rust source into top-level items with leading trivia attached."""
    lines = text.splitlines(keepends=True)
    items: list[str] = []
    pending_trivia: list[str] = []
    idx = 0
    scan_state = _LexState()

    while idx < len(lines):
        while idx < len(lines) and _is_trivia_line(lines[idx]):
            pending_trivia.append(lines[idx])
            idx += 1

        if idx >= len(lines):
            break

        item_lines = pending_trivia
        pending_trivia = []
        item_depth = 0
        saw_item_delimiter = False

        while idx < len(lines):
            line = lines[idx]
            item_lines.append(line)
            depth_delta, saw_delimiter = _scan_item_line(line, scan_state)
            item_depth += depth_delta
            saw_item_delimiter = saw_item_delimiter or saw_delimiter
            idx += 1
            if saw_item_delimiter and item_depth <= 0 and scan_state.is_plain_code():
                break

        items.append("".join(item_lines))

    return items


def _scan_balanced_attribute(text: str, start: int) -> tuple[str, int]:
    """Return a full `#[...]` / `#![...]` attribute and the next cursor."""
    cursor = start
    if text[cursor] != "#":
        raise ValueError("attribute must start with '#'")
    cursor += 1
    if cursor < len(text) and text[cursor] == "!":
        cursor += 1
    if cursor >= len(text) or text[cursor] != "[":
        raise ValueError("attribute missing '['")

    attr_start = start
    cursor += 1
    bracket_depth = 1
    state = _LexState()

    while cursor < len(text) and bracket_depth > 0:
        if state.raw_string_hashes is not None:
            if text[cursor] == '"' and text[cursor + 1 :].startswith(
                "#" * state.raw_string_hashes
            ):
                cursor += 1 + state.raw_string_hashes
                state.raw_string_hashes = None
                continue
            cursor += 1
            continue

        if state.in_string:
            ch = text[cursor]
            if state.string_escape:
                state.string_escape = False
            elif ch == "\\":
                state.string_escape = True
            elif ch == '"':
                state.in_string = False
            cursor += 1
            continue

        if state.in_char:
            ch = text[cursor]
            if state.char_escape:
                state.char_escape = False
            elif ch == "\\":
                state.char_escape = True
            elif ch == "'":
                state.in_char = False
            cursor += 1
            continue

        if state.block_comment_depth > 0:
            if text.startswith("/*", cursor):
                state.block_comment_depth += 1
                cursor += 2
                continue
            if text.startswith("*/", cursor):
                state.block_comment_depth -= 1
                cursor += 2
                continue
            cursor += 1
            continue

        if text.startswith("//", cursor):
            newline = text.find("\n", cursor)
            if newline == -1:
                cursor = len(text)
                continue
            cursor = newline + 1
            continue
        if text.startswith("/*", cursor):
            state.block_comment_depth += 1
            cursor += 2
            continue

        starts_raw, raw_hashes, raw_prefix_len = _starts_raw_string(text, cursor)
        if starts_raw:
            state.raw_string_hashes = raw_hashes
            cursor += raw_prefix_len
            continue

        if text[cursor] == '"':
            state.in_string = True
            cursor += 1
            continue
        if _starts_char_literal(text, cursor):
            state.in_char = True
            cursor += 1
            continue

        if text[cursor] == "[":
            bracket_depth += 1
        elif text[cursor] == "]":
            bracket_depth -= 1
        cursor += 1

    if bracket_depth != 0:
        raise ValueError("unterminated outer attribute")
    return text[attr_start:cursor], cursor


def _scan_leading_outer_attributes(item_text: str) -> tuple[list[str], int]:
    """Return leading outer attributes and the cursor after leading trivia."""
    attrs: list[str] = []
    cursor = 0
    block_comment_depth = 0

    while cursor < len(item_text):
        if block_comment_depth > 0:
            if item_text.startswith("/*", cursor):
                block_comment_depth += 1
                cursor += 2
                continue
            if item_text.startswith("*/", cursor):
                block_comment_depth -= 1
                cursor += 2
                continue
            cursor += 1
            continue

        if item_text[cursor].isspace():
            cursor += 1
            continue
        if item_text.startswith("//", cursor):
            newline = item_text.find("\n", cursor)
            if newline == -1:
                break
            cursor = newline + 1
            continue
        if item_text.startswith("/*", cursor):
            block_comment_depth += 1
            cursor += 2
            continue
        if item_text.startswith("#[", cursor) or item_text.startswith("#![", cursor):
            attr, cursor = _scan_balanced_attribute(item_text, cursor)
            attrs.append(attr)
            continue
        break

    return attrs, cursor


def _leading_outer_attributes(item_text: str) -> list[str]:
    attrs, _ = _scan_leading_outer_attributes(item_text)
    return attrs


def _strip_leading_trivia(item_text: str) -> str:
    """Return item text starting at the first non-trivia token."""
    _, cursor = _scan_leading_outer_attributes(item_text)
    return item_text[cursor:].lstrip()


def _top_level_async_fn_name(item_text: str) -> str | None:
    """Return the name for a top-level async fn item, if this item is one."""
    stripped = _strip_leading_trivia(item_text)
    match = re.match(r"^\s*(?:pub(?:\([^)]*\))?\s+)?async\s+fn\s+(\w+)", stripped)
    if match:
        return match.group(1)
    return None


class _CfgParser:
    def __init__(self, text: str):
        self.text = text
        self.pos = 0

    def _skip_ws(self) -> None:
        while self.pos < len(self.text) and self.text[self.pos].isspace():
            self.pos += 1

    def _peek(self) -> str | None:
        self._skip_ws()
        if self.pos >= len(self.text):
            return None
        return self.text[self.pos]

    def _consume(self, expected: str) -> None:
        self._skip_ws()
        if self.pos >= len(self.text) or self.text[self.pos] != expected:
            raise ValueError(f"expected '{expected}' in cfg expression")
        self.pos += 1

    def _consume_ident(self) -> str:
        self._skip_ws()
        start = self.pos
        while self.pos < len(self.text) and (
            self.text[self.pos].isalnum()
            or self.text[self.pos] in {"_", "-", ":", "."}
        ):
            self.pos += 1
        if start == self.pos:
            raise ValueError("expected identifier in cfg expression")
        return self.text[start:self.pos]

    def _consume_literal(self) -> str:
        self._skip_ws()
        if self.pos < len(self.text) and self.text[self.pos] == '"':
            start = self.pos
            self.pos += 1
            escaped = False
            while self.pos < len(self.text):
                ch = self.text[self.pos]
                self.pos += 1
                if escaped:
                    escaped = False
                    continue
                if ch == "\\":
                    escaped = True
                    continue
                if ch == '"':
                    break
            return self.text[start:self.pos]

        start = self.pos
        while self.pos < len(self.text) and self.text[self.pos] not in {",", ")"}:
            self.pos += 1
        literal = self.text[start:self.pos].strip()
        if not literal:
            raise ValueError("expected literal in cfg expression")
        return literal

    def parse_expr(self) -> tuple[str, object]:
        ident = self._consume_ident()
        if self._peek() == "(" and ident in {"all", "any", "not"}:
            self._consume("(")
            if ident == "not":
                inner = self.parse_expr()
                if self._peek() == ",":
                    self._consume(",")
                self._consume(")")
                return ("not", inner)

            children: list[tuple[str, object]] = []
            while True:
                children.append(self.parse_expr())
                if self._peek() == ",":
                    self._consume(",")
                    if self._peek() == ")":
                        break
                    continue
                break
            self._consume(")")
            return (ident, children)

        if self._peek() == "=":
            self._consume("=")
            literal = self._consume_literal()
            ident = f"{ident}={literal}"

        return ("atom", ident)


def _extract_cfg_expressions(attrs_text: str) -> list[str]:
    expressions: list[str] = []
    for attr in attrs_text:
        body_start = 3 if attr.startswith("#![") else 2
        body = attr[body_start:-1].strip()
        attr_name = body.split("(", 1)[0].strip()
        if attr_name != "cfg":
            continue
        open_paren = body.find("(")
        if open_paren == -1:
            continue
        inner = body[open_paren + 1 :].rstrip()
        if inner.endswith(")"):
            inner = inner[:-1]
        expressions.append(inner)
    return expressions


def _parse_cfg_expr(expr: str) -> tuple[str, object]:
    parser = _CfgParser(expr)
    parsed = parser.parse_expr()
    parser._skip_ws()
    if parser.pos != len(parser.text):
        raise ValueError(f"unexpected trailing cfg tokens: {parser.text[parser.pos:]!r}")
    return parsed

    
def _cfg_can_be_true_without_test_node(node: tuple[str, object]) -> bool:
    def simplify(node: tuple[str, object]) -> tuple[str, object]:
        kind, payload = node
        if kind == "atom":
            if payload == "test":
                return ("const", False)
            return node
        if kind == "not":
            child = simplify(payload)  # type: ignore[arg-type]
            if child[0] == "const":
                return ("const", not child[1])  # type: ignore[index]
            if child[0] == "atom":
                return ("not", child)
            if child[0] == "not":
                return simplify(child[1])  # type: ignore[index]
            if child[0] == "all":
                return simplify(
                    ("any", [("not", grandchild) for grandchild in child[1]])  # type: ignore[index]
                )
            if child[0] == "any":
                return simplify(
                    ("all", [("not", grandchild) for grandchild in child[1]])  # type: ignore[index]
                )
            return ("not", child)
        children = [simplify(child) for child in payload]  # type: ignore[arg-type]
        if kind == "all":
            if any(child == ("const", False) for child in children):
                return ("const", False)
            children = [child for child in children if child != ("const", True)]
            if not children:
                return ("const", True)
            return ("all", children)
        if any(child == ("const", True) for child in children):
            return ("const", True)
        children = [child for child in children if child != ("const", False)]
        if not children:
            return ("const", False)
        return ("any", children)

    def gather_requirements(
        node: tuple[str, object],
    ) -> list[dict[str, bool]] | None:
        kind, payload = node
        if kind == "const":
            return [{}] if payload else []
        if kind == "atom":
            return [{typing.cast(str, payload): True}]
        if kind == "not":
            child_kind, child_payload = typing.cast(tuple[str, object], payload)
            if child_kind != "atom":
                raise ValueError("unexpected non-atom after cfg simplification")
            return [{typing.cast(str, child_payload): False}]
        if kind == "any":
            combined: list[dict[str, bool]] = []
            for child in typing.cast(list[tuple[str, object]], payload):
                child_requirements = gather_requirements(child)
                if child_requirements is None:
                    return None
                combined.extend(child_requirements)
            return combined
        combinations: list[dict[str, bool]] = [{}]
        for child in typing.cast(list[tuple[str, object]], payload):
            child_requirements = gather_requirements(child)
            if child_requirements is None:
                return None
            next_combinations: list[dict[str, bool]] = []
            for base in combinations:
                for child_req in child_requirements:
                    merged = dict(base)
                    conflict = False
                    for atom, value in child_req.items():
                        if atom in merged and merged[atom] != value:
                            conflict = True
                            break
                        merged[atom] = value
                    if not conflict:
                        next_combinations.append(merged)
            combinations = next_combinations
            if not combinations:
                return []
        return combinations

    simplified = simplify(node)
    requirements = gather_requirements(simplified)
    return bool(requirements)


def _item_is_test_only(item_text: str) -> bool:
    attrs = _leading_outer_attributes(item_text)
    cfg_expressions = _extract_cfg_expressions(attrs)
    if not cfg_expressions:
        return False
    combined = ("all", [_parse_cfg_expr(expr) for expr in cfg_expressions])
    return not _cfg_can_be_true_without_test_node(combined)


def extract_handlers(rs_file: pathlib.Path) -> tuple[dict[str, str], set[str]]:
    """Return runtime async fn signatures and all source fn names."""
    items = _split_top_level_items(rs_file.read_text())
    runtime_text = "".join(item for item in items if not _item_is_test_only(item))

    handlers: dict[str, str] = {}
    for m in re.finditer(r"(?:pub\s+)?async\s+fn\s+(\w+)", runtime_text):
        name = m.group(1)
        start = m.start()
        brace = runtime_text.find("{", start)
        sig = (
            runtime_text[start:brace]
            if brace != -1
            else runtime_text[start : start + 500]
        )
        sig = " ".join(sig.split())  # normalize whitespace
        handlers[name] = sig

    all_function_names: set[str] = set()
    for item in items:
        top_level_name = _top_level_async_fn_name(item)
        if top_level_name is not None:
            all_function_names.add(top_level_name)
    return handlers, all_function_names


def check_classification(
    fn_name: str, sig: str, classification: str
) -> list[str]:
    """Return list of error strings (empty = pass)."""
    errors: list[str] = []

    has_db_state = "State<DbState>" in sig
    has_tenant_db = "TenantDb" in sig
    has_full_state = "State<Arc<AppState>>" in sig
    has_sub_state = any(f"State<{s}>" in sig for s in FOCUSED_SUBSTATES)

    if classification == "ignore":
        return []

    if classification == "tenant-agnostic":
        if not has_db_state:
            errors.append(
                f"  {fn_name}: classified 'tenant-agnostic' but missing State<DbState>"
            )
        if has_tenant_db:
            errors.append(
                f"  {fn_name}: classified 'tenant-agnostic' but has TenantDb"
            )
        if has_full_state:
            errors.append(
                f"  {fn_name}: classified 'tenant-agnostic' but has State<Arc<AppState>>"
            )

    elif classification == "tenant-scoped":
        if not has_tenant_db:
            errors.append(
                f"  {fn_name}: classified 'tenant-scoped' but missing TenantDb"
            )
        if has_db_state:
            errors.append(
                f"  {fn_name}: classified 'tenant-scoped' but has State<DbState>"
            )
        if has_full_state:
            errors.append(
                f"  {fn_name}: classified 'tenant-scoped' but has State<Arc<AppState>>"
            )

    elif classification == "no-db":
        if has_db_state:
            errors.append(f"  {fn_name}: classified 'no-db' but has State<DbState>")
        if has_tenant_db:
            errors.append(f"  {fn_name}: classified 'no-db' but has TenantDb")
        if has_full_state:
            errors.append(
                f"  {fn_name}: classified 'no-db' but has State<Arc<AppState>>"
            )

    elif classification == "full-state":
        if not has_full_state:
            errors.append(
                f"  {fn_name}: classified 'full-state' but missing State<Arc<AppState>>"
            )
        if has_sub_state:
            errors.append(
                f"  {fn_name}: classified 'full-state' but has focused State<SubState>"
            )

    else:
        errors.append(
            f"  {fn_name}: unknown classification '{classification}' "
            f"(valid: tenant-agnostic, tenant-scoped, no-db, full-state, ignore)"
        )

    return errors


def _policy_entry_exists(
    fn_name: str,
    classification: str,
    handlers: dict[str, str],
    source_function_names: set[str],
) -> bool:
    """Return whether a policy entry still maps to a live async item.

    Stale checks intentionally use the same runtime async item set as the
    unlisted-handler pass, so policy entries cannot stay alive via test-only
    or otherwise non-runtime async functions.
    """
    return fn_name in handlers


def main() -> None:
    if not POLICY_PATH.exists():
        print(f"ERROR: Policy file not found: {POLICY_PATH}", file=sys.stderr)
        print(
            "Run: python3 ci/seed_db_access_policy.py  to generate the initial file.",
            file=sys.stderr,
        )
        sys.exit(1)

    policy = tomllib.loads(POLICY_PATH.read_text())
    routes_policy: dict[str, dict[str, str]] = policy.get("routes", {})

    exit_code = 0
    all_errors: list[str] = []

    for rs_file in sorted(ROUTES_DIR.rglob("*.rs")):
        rel = str(rs_file.relative_to(ROUTES_DIR))
        handlers, source_function_names = extract_handlers(rs_file)
        file_policy = routes_policy.get(rel, {})

        # Check for unlisted handlers (in code but not in policy).
        for fn_name in handlers:
            if fn_name not in file_policy:
                all_errors.append(
                    f"{rel}: handler '{fn_name}' is not listed in db_access_policy.toml"
                )
                exit_code = 1

        # Check for stale entries (in policy but not in code).
        for fn_name, classification in file_policy.items():
            if not _policy_entry_exists(
                fn_name,
                classification,
                handlers,
                source_function_names,
            ):
                all_errors.append(
                    f"{rel}: policy entry '{fn_name}' not found in source file (stale)"
                )
                exit_code = 1

        # Check classification correctness.
        for fn_name, classification in file_policy.items():
            if fn_name not in handlers:
                continue  # stale already reported above
            sig = handlers[fn_name]
            errors = check_classification(fn_name, sig, classification)
            if errors:
                all_errors.append(f"{rel}:")
                all_errors.extend(errors)
                exit_code = 1

    if all_errors:
        print("ERROR: db_access_policy.toml violations found:", file=sys.stderr)
        for line in all_errors:
            print(line, file=sys.stderr)
    else:
        print("OK: db_access_policy.toml is consistent with routes/ signatures.")

    sys.exit(exit_code)


if __name__ == "__main__":
    main()
