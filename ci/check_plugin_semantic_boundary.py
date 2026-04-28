#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ModuleNotFoundError:
        tomllib = None


EXIT_OK = 0
EXIT_VIOLATIONS = 1
EXIT_CONFIG_ERROR = 2


RULE_PLUGIN_CORE_IMPORT = "plugin-core-import"
RULE_CONCRETE_PLUGIN_IMPORT = "concrete-plugin-import"
RULE_PLUGIN_IDS_REFERENCE = "plugin-ids-reference"
RULE_FORBIDDEN_PLUGIN_HELPER = "forbidden-plugin-helper"
RULE_HARDCODED_PLUGIN_TYPE_LITERAL = "hardcoded-plugin-type-literal"
RULE_MANIFEST_PLUGIN_DEPENDENCY = "manifest-plugin-dependency"
# Migration-only parity rule for the legacy shell checker surface.
# Intentionally excluded from KNOWN_RULE_IDS so allowlists remain spec-canonical only.
RULE_LEGACY_DASHBOARD_BESPOKE_SURFACE = "legacy-dashboard-bespoke-surface"

KNOWN_RULE_IDS = {
    RULE_PLUGIN_CORE_IMPORT,
    RULE_CONCRETE_PLUGIN_IMPORT,
    RULE_PLUGIN_IDS_REFERENCE,
    RULE_FORBIDDEN_PLUGIN_HELPER,
    RULE_HARDCODED_PLUGIN_TYPE_LITERAL,
    RULE_MANIFEST_PLUGIN_DEPENDENCY,
}

RULE_MATCH_KINDS: dict[str, set[str]] = {
    RULE_PLUGIN_CORE_IMPORT: {"import_path"},
    RULE_CONCRETE_PLUGIN_IMPORT: {"crate_name"},
    RULE_PLUGIN_IDS_REFERENCE: {"module_token"},
    RULE_FORBIDDEN_PLUGIN_HELPER: {"api_name"},
    RULE_HARDCODED_PLUGIN_TYPE_LITERAL: {"literal_string"},
    RULE_MANIFEST_PLUGIN_DEPENDENCY: {"manifest_dependency"},
}

ALLOWED_MATCH_KINDS = {kind for kinds in RULE_MATCH_KINDS.values() for kind in kinds}

PLUGIN_TYPE_ID_REL_PATH = "crates/shared/types/src/plugin_type_id.rs"
PLUGIN_TYPE_ID_GENERATED_MIRROR_REL_PATHS = (
    "crates/shared/openapi-client/src/generated/shared_types/plugin_type_id.rs",
    "crates/shared/service-sdk/src/generated/shared_types/plugin_type_id.rs",
)

CORE_IMPORT_TOKEN_RE = re.compile(
    r"\buptrakit_plugin_infrastructure_core(?:::[A-Za-z0-9_]+)?\b"
)
CONCRETE_IMPORT_TOKEN_RE = re.compile(
    r"\b(?:uptrakit_plugin_[a-z0-9_]+|uptrakit_notification_plugin_[a-z0-9_]+)(?:::[A-Za-z0-9_]+)?\b"
)
HELPER_DEF_RE = re.compile(r"\bfn\s+(is_package_manager|display_name)\s*\(")
PLUGIN_TYPE_ID_ANY_ASSOC_CALL_RE = re.compile(
    r"\b(?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*([A-Za-z_][A-Za-z0-9_]*)\s*::\s*(is_package_manager|display_name)\s*\("
)
PLUGIN_TYPE_ID_ASSOC_FUNCTION_ITEM_RE = re.compile(
    r"\b(?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*([A-Za-z_][A-Za-z0-9_]*)\s*::\s*(is_package_manager|display_name)\b(?!\s*\()"
)
PLUGIN_TYPE_ID_METHOD_CALL_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*(is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_SELF_FIELD_METHOD_CALL_RE = re.compile(
    r"\bself\s*\.\s*(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\s*\.\s*(?P<api>is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_COMPLEX_RECEIVER_METHOD_CALL_RE = re.compile(
    r"\.\s*(is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_MULTILINE_METHOD_CALL_RE = re.compile(
    r"\b(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\s*\n\s*\.\s*(?P<api>is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_SELF_FIELD_MULTILINE_METHOD_CALL_RE = re.compile(
    r"\bself\s*\.\s*(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\s*\n\s*\.\s*(?P<api>is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_MULTILINE_ASSOC_CALL_RE = re.compile(
    r"\b(?:[A-Za-z_][A-Za-z0-9_]*::)*(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\s*\n\s*::\s*(?P<api>is_package_manager|display_name)\s*\("
)
PLUGIN_TYPE_ID_MULTILINE_COMPLEX_RECEIVER_METHOD_CALL_RE = re.compile(
    r"(?P<receiver>[^\n]*[)\]}])\s*\n\s*\.\s*(?P<api>is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_ALIAS_RE = re.compile(
    r"\b(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\b"
)
PLUGIN_TYPE_ID_IMPORT_ALIAS_PAIR_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
PLUGIN_IDS_DIRECT_IMPORT_ALIAS_RE = re.compile(
    r"\bplugin_ids\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)
PLUGIN_IDS_DIRECT_IMPORT_RE = re.compile(r"\bplugin_ids\s*;")
PLUGIN_IDS_DIRECT_WILDCARD_IMPORT_RE = re.compile(
    r"\bplugin_ids\s*::\s*\*\s*;"
)
PLUGIN_IDS_BRACED_IMPORT_RE = re.compile(
    r"\bplugin_ids\s*::\s*\{(?P<body>[^}]*)\}\s*;"
)
PLUGIN_IDS_GROUPED_PARENT_IMPORT_RE = re.compile(
    r"\{(?P<body>[^}]*)\}\s*;"
)
PLUGIN_IDS_DIRECT_CONST_IMPORT_RE = re.compile(
    r"\bplugin_ids\s*::\s*([A-Za-z_][A-Za-z0-9_]*)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;"
)
PLUGIN_IDS_SELF_ALIAS_RE = re.compile(
    r"self(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?"
)
PLUGIN_TYPE_ID_TYPED_BINDING_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(?P<type>(?:&\s*)?(?:mut\s+)?[^=,;)\{\n]+)"
)
PLUGIN_TYPE_ID_LET_TYPED_BINDING_RE = re.compile(
    r"\blet\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(?P<type>[^=;\n]+)"
)
PLUGIN_TYPE_ID_LET_INFERRED_BINDING_RE = re.compile(
    r"\blet\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<expr>[^;\n]+)"
)
PLUGIN_TYPE_ID_DIRECT_CONSTRUCTOR_BINDING_RE = re.compile(
    r"\blet\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*PluginTypeId::"
)
PLUGIN_TYPE_ID_INDEXED_METHOD_CALL_RE = re.compile(
    r"\b(?P<prefix>(?:[A-Za-z_][A-Za-z0-9_]*\s*\.\s*)*)(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\s*\[[^\]\n]+\]\s*\.\s*(?P<api>is_package_manager|display_name)\s*\(\s*\)"
)
PLUGIN_TYPE_ID_RETURNING_FN_RE = re.compile(
    r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>\n]*>)?\s*\([^)\n]*\)\s*->\s*(?P<type>[^{;\n]+)"
)
TRAILING_FN_CALL_RE = re.compile(
    r"(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\([^()\n]*\)\s*$"
)
TYPE_TOKEN_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
PLUGIN_CRATE_NAME_RE = re.compile(
    r"^(?:uptrakit-plugin|uptrakit-notification-plugin)-[a-z0-9-]+$"
)
DASHBOARD_BESPOKE_LITERAL_RE = re.compile(
    r"settings_dashboard_icons|dashboard_icons\.enabled"
)
IDENTITY_SPECIFIC_HELPER_RE = re.compile(
    r"\b(is_[a-z0-9_]*dashboard|has_[a-z0-9_]*dashboard|is_dashboard_icons|has_dashboard_icons)\b"
)

CONST_DEF_RE = re.compile(
    r"pub\s+const\s+([A-Z0-9_]+)\s*:\s*PluginTypeId\s*=\s*PluginTypeId::from_static\(\"([a-z0-9_]+)\"\)\s*;",
    re.DOTALL,
)
ALL_LIST_RE = re.compile(
    r"pub\s+const\s+ALL\s*:\s*&\[\s*PluginTypeId\s*\]\s*=\s*&\[(.*?)\];",
    re.DOTALL,
)
ALL_ENTRY_RE = re.compile(r"\b([A-Z][A-Z0-9_]*)\b")

RUST_RAW_STRING_RE = re.compile(
    r'(?:br|r)(?P<hashes>#{0,16})"(?P<body>.*?)"(?P=hashes)'
)
RUST_STRING_RE = re.compile(r'"(?P<body>(?:\\.|[^"\\])*)"')
RUST_RAW_STRING_MULTILINE_RE = re.compile(
    r'(?:br|r)(?P<hashes>#{0,16})"(?P<body>.*?)"(?P=hashes)',
    re.DOTALL,
)
RUST_STRING_MULTILINE_RE = re.compile(
    r'(?:b)?"(?P<body>(?:\\.|[^"\\])*)"',
    re.DOTALL,
)
TS_DOUBLE_QUOTED_RE = re.compile(r'"(?P<body>(?:\\.|[^"\\])*)"')
TS_SINGLE_QUOTED_RE = re.compile(r"'(?P<body>(?:\\.|[^'\\])*)'")
TS_TEMPLATE_RE = re.compile(r"`(?P<body>(?:\\.|[^`\\])*)`")

IDENTITY_CONTEXT_HINTS = (
    "plugin_type",
    "plugintype",
    "plugin-type",
    "plugin-types",
    "plugin_types",
    "channel_type",
    "channeltype",
    "plugin_type_id",
    "plugintypeid",
    "plugintypeid::from_static",
    "plugintypeid::new",
)
IDENTITY_CONTEXT_HINT_RES = tuple(
    re.compile(rf"(?<![a-z0-9_]){re.escape(hint)}(?![a-z0-9_])")
    for hint in IDENTITY_CONTEXT_HINTS
)
IDENTITY_SAME_LINE_HINT_RES = tuple(
    re.compile(rf"(?<![a-z0-9]){re.escape(hint)}(?![a-z0-9_])")
    for hint in IDENTITY_CONTEXT_HINTS
)
IDENTITY_LITERAL_KEY_VALUE_HINTS = (
    "plugin_type",
    "plugintype",
    "plugin-type",
    "plugin-types",
    "plugin_types",
    "channel_type",
    "channeltype",
    "channel-type",
    "plugin_type_id",
    "plugintypeid",
    "plugin-type-id",
)
IDENTITY_LITERAL_KEY_VALUE_PATTERN = "|".join(
    re.escape(hint) for hint in IDENTITY_LITERAL_KEY_VALUE_HINTS
)
IDENTITY_LITERAL_KEY_VALUE_RE = re.compile(
    rf"(?is)(?<![a-z0-9_])[\"']?(?:{IDENTITY_LITERAL_KEY_VALUE_PATTERN})[\"']?\s*(?::|=)\s*[\"']?(?P<value>[a-z0-9_]+)(?![a-z0-9_])"
)
PLUGIN_TYPES_ROUTE_RE = re.compile(r"\bplugin[-_]types\b")

BLOCK_GLOB_RE = re.compile(r"[\*\?\[\]\{\}]")
REGEX_LIKE_MATCH_VALUE_RE = re.compile(r"[\\\^\$\*\+\?\{\}\[\]\(\)\|]")
FRONTEND_TEST_STORY_FIXTURE_FILE_RE = re.compile(
    r"\.(test|story|stories|fixture|fixtures)\.[^/]+$"
)
RUST_CFG_ATTRIBUTE_RE = re.compile(
    r"^\s*#\s*\[\s*cfg\s*\((?P<expr>.*?)\)\s*\]\s*(?://.*)?$"
)
RUST_CFG_ATTRIBUTE_PREFIX_RE = re.compile(
    r"^\s*#\s*\[\s*cfg\s*\((?P<expr>.*?)\)\s*\](?P<suffix>.*)$"
)
RUST_TEST_ATOM_RE = re.compile(r"^\s*test\s*$")
RUST_TEST_ATTRIBUTE_RE = re.compile(
    r"^\s*#\s*\[\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*test(?:\s*\([^]]*\))?\s*\]\s*(?://.*)?$"
)
RUST_FN_NAME_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b")
RUST_RAW_STRING_START_RE = re.compile(r'(?:br|r)(?P<hashes>#{0,16})"')
RUST_TEST_ONLY_PATH_PARTS = frozenset({"tests", "integration_tests"})
RUST_TEST_ONLY_FILENAMES = frozenset({"tests.rs", "test.rs"})
OUT_OF_SCOPE_ROOT_PARTS = frozenset({"docs", "examples", "migrations"})
MANIFEST_ALLOWED_PLUGIN_DEPENDENCIES = frozenset(
    {
        "uptrakit-plugin-infrastructure-registry",
        "uptrakit-plugin-infrastructure-catalog",
        "uptrakit-plugin-infrastructure-catalogue",
    }
)
ALLOWED_REGISTRY_CATALOGUE_IMPORT_CRATES = frozenset(
    {
        "uptrakit_plugin_infrastructure_core",
        "uptrakit_plugin_infrastructure_registry",
        "uptrakit_plugin_infrastructure_catalog",
        "uptrakit_plugin_infrastructure_catalogue",
    }
)


@dataclass(frozen=True, order=True)
class Finding:
    rule_id: str
    path: str
    line: int
    match_kind: str
    match_value: str
    excerpt: str


@dataclass(frozen=True)
class AllowlistEntry:
    path: str
    rule_id: str
    match_kind: str
    match_value: str
    reason: str


@dataclass(frozen=True)
class CanonicalPluginIds:
    ids: frozenset[str]
    constant_names: frozenset[str]
    exempt_lines_by_path: dict[str, frozenset[int]]


@dataclass(frozen=True)
class RustScopeIndex:
    parent_by_scope: dict[int, int | None]
    active_scope_ids_by_line: dict[int, tuple[int, ...]]


@dataclass
class RustBraceScanState:
    block_comment_depth: int = 0
    in_string: bool = False
    string_is_raw: bool = False
    raw_hashes: str = ""
    escape_next: bool = False


class ConfigError(RuntimeError):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Check plugin semantic boundary rules")
    parser.add_argument(
        "--root",
        default=".",
        help="Repository or fixture root to scan (defaults to current working directory)",
    )
    parser.add_argument(
        "--allowlist",
        help="Path to allowlist TOML (defaults to <root>/ci/plugin_semantic_boundary_allowlist.toml when present)",
    )
    parser.add_argument(
        "--format",
        choices=("text", "json"),
        default="text",
        help="Output format",
    )
    return parser.parse_args(argv)


def posix_rel(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def _is_frontend_excluded(normalized: str) -> bool:
    if not normalized.startswith("frontend/src/"):
        return False
    if (
        "/__tests__/" in normalized
        or "/__fixtures__/" in normalized
        or "/fixtures/" in normalized
        or "/stories/" in normalized
        or "/__stories__/" in normalized
    ):
        return True
    filename = normalized.rsplit("/", 1)[-1]
    return FRONTEND_TEST_STORY_FIXTURE_FILE_RE.search(filename) is not None


def _has_test_only_path_component(normalized: str) -> bool:
    parts = set(normalized.split("/"))
    return not RUST_TEST_ONLY_PATH_PARTS.isdisjoint(parts)


def _is_rust_test_module_filename(normalized: str) -> bool:
    filename = normalized.rsplit("/", 1)[-1]
    return filename in RUST_TEST_ONLY_FILENAMES


def _is_out_of_scope_root_path(normalized: str) -> bool:
    parts = normalized.split("/")
    if not parts:
        return False

    try:
        src_index = parts.index("src")
    except ValueError:
        src_index = len(parts)

    for idx, part in enumerate(parts):
        if part in OUT_OF_SCOPE_ROOT_PARTS and idx < src_index:
            return True
    return False


def looks_like_production_code(rel: str) -> bool:
    normalized = rel.replace("\\", "/")
    if not (
        normalized.startswith("crates/")
        or normalized.startswith("frontend/src/")
    ):
        return False

    # Track C intentionally excludes plugin implementation trees from the gate,
    # including registry/catalogue implementation crates. Only the production
    # consumer surface is scanned here.
    if normalized.startswith("crates/plugins/"):
        return False
    if _is_frontend_excluded(normalized):
        return False
    if _is_out_of_scope_root_path(normalized):
        return False
    if _has_test_only_path_component(normalized):
        return False
    if normalized.startswith("crates/") and normalized.endswith(".rs"):
        if _is_rust_test_module_filename(normalized):
            return False
    return True


def should_scan_manifest(rel: str) -> bool:
    normalized = rel.replace("\\", "/")
    if not normalized.endswith("Cargo.toml"):
        return False
    if not normalized.startswith("crates/"):
        return False
    if normalized.startswith("crates/plugins/"):
        return False
    if _is_out_of_scope_root_path(normalized):
        return False
    if _has_test_only_path_component(normalized):
        return False
    return True


def _split_cfg_args(raw: str) -> list[str] | None:
    parts: list[str] = []
    depth = 0
    start = 0
    quote_char: str | None = None
    i = 0

    while i < len(raw):
        ch = raw[i]
        if quote_char is not None:
            if ch == "\\":
                i += 2
                continue
            if ch == quote_char:
                quote_char = None
            i += 1
            continue

        if ch in {'"', "'"}:
            quote_char = ch
            i += 1
            continue

        if ch == "(":
            depth += 1
            i += 1
            continue
        if ch == ")":
            depth -= 1
            if depth < 0:
                return None
            i += 1
            continue
        if ch == "," and depth == 0:
            parts.append(raw[start:i].strip())
            start = i + 1
        i += 1

    if depth != 0 or quote_char is not None:
        return None
    parts.append(raw[start:].strip())
    if any(part == "" for part in parts):
        return None
    return parts


def _parse_cfg_call(expr: str) -> tuple[str, list[str]] | None:
    call_match = re.match(r"^(all|any|not)\s*\(", expr)
    if not call_match:
        return None

    operator = call_match.group(1)
    open_idx = call_match.end() - 1
    depth = 0
    quote_char: str | None = None
    close_idx = -1
    i = open_idx
    while i < len(expr):
        ch = expr[i]
        if quote_char is not None:
            if ch == "\\":
                i += 2
                continue
            if ch == quote_char:
                quote_char = None
            i += 1
            continue

        if ch in {'"', "'"}:
            quote_char = ch
            i += 1
            continue

        if ch == "(":
            depth += 1
            i += 1
            continue
        if ch == ")":
            depth -= 1
            if depth == 0:
                close_idx = i
                break
            i += 1
            continue
        i += 1

    if close_idx < 0:
        return None
    if expr[close_idx + 1 :].strip():
        return None

    body = expr[open_idx + 1 : close_idx]
    args = _split_cfg_args(body)
    if args is None:
        return None
    return operator, args


def _cfg_possibility(expr: str, *, test_enabled: bool) -> tuple[bool, bool]:
    normalized = expr.strip()
    if not normalized:
        return True, True

    parsed_call = _parse_cfg_call(normalized)
    if parsed_call is not None:
        operator, args = parsed_call
        child_possibilities = [
            _cfg_possibility(arg, test_enabled=test_enabled) for arg in args
        ]
        if operator == "all":
            can_be_true = all(can_true for can_true, _ in child_possibilities)
            can_be_false = any(can_false for _, can_false in child_possibilities)
            return can_be_true, can_be_false
        if operator == "any":
            can_be_true = any(can_true for can_true, _ in child_possibilities)
            can_be_false = all(can_false for _, can_false in child_possibilities)
            return can_be_true, can_be_false
        if len(child_possibilities) != 1:
            return True, True
        child_true, child_false = child_possibilities[0]
        return child_false, child_true

    if RUST_TEST_ATOM_RE.fullmatch(normalized):
        return test_enabled, (not test_enabled)
    return True, True


def _cfg_expression_requires_test(cfg_expr: str) -> bool:
    can_true_with_test, _ = _cfg_possibility(cfg_expr, test_enabled=True)
    can_true_without_test, _ = _cfg_possibility(cfg_expr, test_enabled=False)
    return can_true_with_test and (not can_true_without_test)


def _is_cfg_test_attribute_line(line: str) -> bool:
    match = RUST_CFG_ATTRIBUTE_RE.match(line)
    if not match:
        return False
    return _cfg_expression_requires_test(match.group("expr"))


def _cfg_test_attribute_suffix(line: str) -> str | None:
    match = RUST_CFG_ATTRIBUTE_PREFIX_RE.match(line)
    if not match:
        return None
    if not _cfg_expression_requires_test(match.group("expr")):
        return None
    return match.group("suffix")


def _iter_rust_brace_events(
    line: str,
    state: RustBraceScanState,
):
    i = 0

    while i < len(line):
        if state.block_comment_depth > 0:
            if line.startswith("/*", i):
                state.block_comment_depth += 1
                i += 2
                continue
            if line.startswith("*/", i):
                state.block_comment_depth -= 1
                i += 2
                continue
            i += 1
            continue

        if state.in_string:
            if state.string_is_raw:
                if line[i] == '"' and line.startswith(state.raw_hashes, i + 1):
                    i += 1 + len(state.raw_hashes)
                    state.in_string = False
                    state.string_is_raw = False
                    state.raw_hashes = ""
                    continue
                i += 1
                continue

            if state.escape_next:
                state.escape_next = False
                i += 1
                continue
            if line[i] == "\\":
                state.escape_next = True
                i += 1
                continue
            if line[i] == '"':
                state.in_string = False
            i += 1
            continue

        if line.startswith("//", i):
            break
        if line.startswith("/*", i):
            state.block_comment_depth += 1
            i += 2
            continue

        raw_string_match = RUST_RAW_STRING_START_RE.match(line, i)
        if raw_string_match:
            state.in_string = True
            state.string_is_raw = True
            state.raw_hashes = raw_string_match.group("hashes")
            i = raw_string_match.end()
            continue

        char_literal_end = _consume_rust_char_literal(line, i)
        if char_literal_end is not None:
            i = char_literal_end
            continue

        if line.startswith('b"', i):
            state.in_string = True
            state.string_is_raw = False
            state.escape_next = False
            i += 2
            continue
        if line[i] == '"':
            state.in_string = True
            state.string_is_raw = False
            state.escape_next = False
            i += 1
            continue

        if line[i] in {"{", "}"}:
            yield line[i]
        i += 1


def _skip_cfg_test_item(
    lines: list[str],
    start_index: int,
    *,
    first_line: str | None = None,
) -> int:
    return _skip_rust_item(lines, start_index, first_line=first_line)


def _consume_rust_char_literal(line: str, start: int) -> int | None:
    if line[start] != "'" or start + 2 >= len(line):
        return None

    idx = start + 1
    if line[idx] == "\\":
        idx += 1
        if idx >= len(line):
            return None

        escaped = line[idx]
        if escaped == "x":
            if idx + 2 >= len(line):
                return None
            hex_digits = line[idx + 1 : idx + 3]
            if any(ch not in "0123456789abcdefABCDEF" for ch in hex_digits):
                return None
            idx += 3
        elif escaped == "u":
            if idx + 1 >= len(line) or line[idx + 1] != "{":
                return None
            idx += 2
            hex_start = idx
            while idx < len(line) and line[idx] != "}":
                if line[idx] not in "0123456789abcdefABCDEF_":
                    return None
                idx += 1
            if idx == hex_start or idx >= len(line) or line[idx] != "}":
                return None
            idx += 1
        elif escaped in {'\\', "'", '"', "n", "r", "t", "0"}:
            idx += 1
        else:
            return None
    else:
        if line[idx] in {"'", "\n"}:
            return None
        idx += 1

    if idx >= len(line) or line[idx] != "'":
        return None
    return idx + 1


def _count_rust_braces(line: str, state: RustBraceScanState) -> tuple[int, int]:
    open_count = 0
    close_count = 0
    for brace in _iter_rust_brace_events(line, state):
        if brace == "{":
            open_count += 1
        else:
            close_count += 1

    return open_count, close_count


def _skip_rust_item(
    lines: list[str],
    start_index: int,
    *,
    first_line: str | None = None,
) -> int:
    i = start_index
    depth = 0
    saw_open_brace = False
    brace_state = RustBraceScanState()

    while i < len(lines):
        line = first_line if i == start_index and first_line is not None else lines[i]
        open_count, close_count = _count_rust_braces(line, brace_state)
        if open_count > 0:
            saw_open_brace = True
        depth += open_count - close_count
        i += 1

        if saw_open_brace and depth <= 0:
            return i
        if not saw_open_brace and ";" in line and depth <= 0:
            return i

    return i


def strip_cfg_test_items(text: str) -> str:
    lines = text.splitlines()
    kept: list[str] = []
    i = 0
    pending_cfg_test = False

    while i < len(lines):
        line = lines[i]
        cfg_test_suffix = _cfg_test_attribute_suffix(line)
        if cfg_test_suffix is not None:
            kept.append("")
            stripped_suffix = cfg_test_suffix.strip()
            if (
                stripped_suffix
                and not cfg_test_suffix.lstrip().startswith("//")
                and not cfg_test_suffix.lstrip().startswith("#[")
            ):
                skipped_to = _skip_cfg_test_item(lines, i, first_line=cfg_test_suffix)
                kept.extend("" for _ in range(skipped_to - i - 1))
                i = skipped_to
                pending_cfg_test = False
                continue
            pending_cfg_test = True
            i += 1
            continue

        if pending_cfg_test:
            if re.match(r"^\s*$", line) or re.match(r"^\s*#\[", line):
                kept.append("")
                i += 1
                continue

            skipped_to = _skip_cfg_test_item(lines, i)
            kept.extend("" for _ in range(skipped_to - i))
            i = skipped_to
            pending_cfg_test = False
            continue

        kept.append(line)
        i += 1

    return "\n".join(kept)


def strip_test_functions(text: str) -> str:
    lines = text.splitlines()
    kept: list[str] = []
    i = 0

    while i < len(lines):
        line = lines[i]
        if RUST_TEST_ATTRIBUTE_RE.match(line):
            kept.append("")
            i += 1
            while i < len(lines) and (
                re.match(r"^\s*$", lines[i]) or re.match(r"^\s*#\s*\[", lines[i])
            ):
                kept.append("")
                i += 1
            if i >= len(lines):
                break
            if re.search(r"\bfn\b", lines[i]):
                skipped_to = _skip_rust_item(lines, i)
                kept.extend("" for _ in range(skipped_to - i))
                i = skipped_to
                continue
            continue
        kept.append(line)
        i += 1

    return "\n".join(kept)


def preprocess_rust(text: str) -> str:
    return strip_test_functions(strip_cfg_test_items(text))


def _replace_comment_with_whitespace(match: re.Match[str]) -> str:
    block = match.group(0)
    return "".join("\n" if ch == "\n" else " " for ch in block)


def _strip_nested_rust_block_comments(text: str) -> str:
    stripped: list[str] = []
    i = 0
    comment_depth = 0
    in_string = False
    string_is_raw = False
    raw_hashes = ""
    escape_next = False

    while i < len(text):
        if comment_depth > 0:
            if text.startswith("/*", i):
                comment_depth += 1
                stripped.append("  ")
                i += 2
                continue
            if text.startswith("*/", i):
                comment_depth -= 1
                stripped.append("  ")
                i += 2
                continue

            stripped.append("\n" if text[i] == "\n" else " ")
            i += 1
            continue

        if in_string:
            if string_is_raw:
                if text[i] == '"' and text.startswith(raw_hashes, i + 1):
                    stripped.append('"' + raw_hashes)
                    i += 1 + len(raw_hashes)
                    in_string = False
                    string_is_raw = False
                    raw_hashes = ""
                    continue
                stripped.append(text[i])
                i += 1
                continue

            if escape_next:
                stripped.append(text[i])
                escape_next = False
                i += 1
                continue
            if text[i] == "\\":
                stripped.append(text[i])
                escape_next = True
                i += 1
                continue
            if text[i] == '"':
                in_string = False
            stripped.append(text[i])
            i += 1
            continue

        if text.startswith("/*", i):
            comment_depth = 1
            stripped.append("  ")
            i += 2
            continue

        raw_string_match = RUST_RAW_STRING_START_RE.match(text, i)
        if raw_string_match:
            stripped.append(raw_string_match.group(0))
            in_string = True
            string_is_raw = True
            raw_hashes = raw_string_match.group("hashes")
            i = raw_string_match.end()
            continue

        if text.startswith('b"', i):
            stripped.append('b"')
            in_string = True
            string_is_raw = False
            escape_next = False
            i += 2
            continue

        if text[i] == '"':
            stripped.append('"')
            in_string = True
            string_is_raw = False
            escape_next = False
            i += 1
            continue

        stripped.append(text[i])
        i += 1

    return "".join(stripped)


def strip_block_comments(text: str, suffix: str) -> str:
    if suffix == ".rs":
        stripped = _strip_nested_rust_block_comments(text)
    else:
        stripped = re.sub(r"/\*.*?\*/", _replace_comment_with_whitespace, text, flags=re.DOTALL)
    if suffix == ".svelte":
        stripped = re.sub(
            r"<!--.*?-->",
            _replace_comment_with_whitespace,
            stripped,
            flags=re.DOTALL,
        )
    return stripped


def _literal_span_regexes_for_suffix(suffix: str) -> tuple[re.Pattern[str], ...]:
    if suffix == ".rs":
        return (RUST_RAW_STRING_RE, RUST_STRING_RE)
    return (TS_DOUBLE_QUOTED_RE, TS_SINGLE_QUOTED_RE, TS_TEMPLATE_RE)


def _literal_span_regexes_for_text(suffix: str) -> tuple[re.Pattern[str], ...]:
    if suffix == ".rs":
        return (RUST_RAW_STRING_MULTILINE_RE, RUST_STRING_MULTILINE_RE)
    return (TS_DOUBLE_QUOTED_RE, TS_SINGLE_QUOTED_RE, TS_TEMPLATE_RE)


def _extract_literal_spans(line: str, suffix: str) -> list[tuple[int, int]]:
    spans: list[tuple[int, int]] = []
    for regex in _literal_span_regexes_for_suffix(suffix):
        for match in regex.finditer(line):
            spans.append((match.start(), match.end()))
    spans.sort(key=lambda span: (span[0], span[1]))
    return spans


def _extract_literal_spans_from_text(text: str, suffix: str) -> list[tuple[int, int]]:
    spans: list[tuple[int, int]] = []
    for regex in _literal_span_regexes_for_text(suffix):
        for match in regex.finditer(text):
            spans.append((match.start(), match.end()))
    spans.sort(key=lambda span: (span[0], span[1]))
    return spans


def _is_offset_inside_spans(offset: int, spans: list[tuple[int, int]]) -> bool:
    for start, end in spans:
        if start <= offset < end:
            return True
        if offset < start:
            return False
    return False


def strip_inline_line_comments(text: str, suffix: str) -> str:
    spans = _extract_literal_spans_from_text(text, suffix)
    stripped: list[str] = []
    index = 0
    span_index = 0

    while index < len(text):
        while span_index < len(spans) and spans[span_index][1] <= index:
            span_index += 1

        if span_index < len(spans):
            span_start, span_end = spans[span_index]
            if span_start <= index < span_end:
                stripped.append(text[index:span_end])
                index = span_end
                continue

        if text.startswith("//", index):
            comment_end = index
            while comment_end < len(text) and text[comment_end] != "\n":
                comment_end += 1
            stripped.append(" " * (comment_end - index))
            index = comment_end
            continue

        stripped.append(text[index])
        index += 1

    return "".join(stripped)


def strip_rust_string_literals(text: str) -> str:
    without_raw_strings = re.sub(
        RUST_RAW_STRING_MULTILINE_RE,
        _replace_comment_with_whitespace,
        text,
    )
    return re.sub(
        RUST_STRING_MULTILINE_RE,
        _replace_comment_with_whitespace,
        without_raw_strings,
    )


def _line_is_comment(line: str) -> bool:
    stripped = line.lstrip()
    return stripped.startswith("//")


def _line_no_for_offset(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def discover_canonical_plugin_ids(root: Path) -> CanonicalPluginIds:
    plugin_type_id_path = root / PLUGIN_TYPE_ID_REL_PATH
    if not plugin_type_id_path.exists():
        raise ConfigError(
            "target-set misconfiguration: missing canonical plugin id source "
            f"{PLUGIN_TYPE_ID_REL_PATH}"
        )

    text = plugin_type_id_path.read_text(encoding="utf-8")
    const_defs: dict[str, tuple[str, int, int]] = {}
    for match in CONST_DEF_RE.finditer(text):
        name = match.group(1)
        plugin_id = match.group(2)
        start_line = _line_no_for_offset(text, match.start())
        end_line = _line_no_for_offset(text, match.end())
        const_defs[name] = (plugin_id, start_line, end_line)

    if not const_defs:
        raise ConfigError(
            "target-set misconfiguration: canonical plugin id constants were not found"
        )

    all_match = ALL_LIST_RE.search(text)
    if not all_match:
        raise ConfigError("target-set misconfiguration: plugin_ids::ALL was not found")

    all_body = all_match.group(1)
    all_names = ALL_ENTRY_RE.findall(all_body)
    unresolved_names = sorted({name for name in all_names if name not in const_defs})
    if unresolved_names:
        raise ConfigError(
            "target-set misconfiguration: plugin_ids::ALL references unknown canonical "
            f"plugin id constants: {', '.join(unresolved_names)}"
        )
    all_names = [name for name in all_names if name in const_defs]
    if not all_names:
        raise ConfigError(
            "target-set misconfiguration: plugin_ids::ALL does not reference known constants"
        )

    ids: set[str] = set()
    exempt_lines: set[int] = set()
    for name in all_names:
        plugin_id, start_line, end_line = const_defs[name]
        ids.add(plugin_id)
        exempt_lines.update(range(start_line, end_line + 1))

    exempt_lines_by_path: dict[str, frozenset[int]] = {
        PLUGIN_TYPE_ID_REL_PATH: frozenset(exempt_lines),
    }
    for mirror_rel in PLUGIN_TYPE_ID_GENERATED_MIRROR_REL_PATHS:
        mirror_path = root / mirror_rel
        if not mirror_path.exists():
            continue
        mirror_text = mirror_path.read_text(encoding="utf-8")
        mirror_exempt: set[int] = set()
        for match in CONST_DEF_RE.finditer(mirror_text):
            start_line = _line_no_for_offset(mirror_text, match.start())
            end_line = _line_no_for_offset(mirror_text, match.end())
            mirror_exempt.update(range(start_line, end_line + 1))
        if mirror_exempt:
            exempt_lines_by_path[mirror_rel] = frozenset(mirror_exempt)

    return CanonicalPluginIds(
        ids=frozenset(sorted(ids)),
        constant_names=frozenset(sorted(all_names)),
        exempt_lines_by_path=exempt_lines_by_path,
    )


def add_regex_findings(
    findings: set[Finding],
    *,
    rule_id: str,
    rel_path: str,
    text: str,
    regex: re.Pattern[str],
    match_kind: str,
    match_value_fn: callable | None = None,
    skip_match: callable | None = None,
) -> None:
    for line_no, line in enumerate(text.splitlines(), start=1):
        if _line_is_comment(line):
            continue
        for match in regex.finditer(line):
            raw_value = match.group(0)
            if skip_match is not None and skip_match(raw_value):
                continue
            value = match_value_fn(match) if match_value_fn is not None else raw_value
            findings.add(
                Finding(
                    rule_id=rule_id,
                    path=rel_path,
                    line=line_no,
                    match_kind=match_kind,
                    match_value=value,
                    excerpt=line.strip(),
                )
            )


def _is_concrete_plugin_import(value: str) -> bool:
    crate_name = value.split("::", 1)[0]
    return crate_name not in ALLOWED_REGISTRY_CATALOGUE_IMPORT_CRATES


def _extract_import_crate_name(match: re.Match[str]) -> str:
    return match.group(0).split("::", 1)[0]


def _extract_first_non_empty_group(match: re.Match[str]) -> str:
    for group in match.groups():
        if group:
            return group
    return match.group(0)


def _collect_plugin_type_id_aliases(text: str) -> set[str]:
    alias_pairs: list[tuple[str, str]] = []
    for line in text.splitlines():
        if _line_is_comment(line):
            continue
        for match in PLUGIN_TYPE_ID_ALIAS_RE.finditer(line):
            alias_pairs.append((match.group(1), match.group(2)))
        if "use " not in line:
            continue
        for match in PLUGIN_TYPE_ID_IMPORT_ALIAS_PAIR_RE.finditer(line):
            alias_pairs.append((match.group(2), match.group(1)))

    aliases: set[str] = set()
    changed = True
    while changed:
        changed = False
        for alias, target in alias_pairs:
            if alias in aliases:
                continue
            if target == "PluginTypeId" or target in aliases:
                aliases.add(alias)
                changed = True
    return aliases


def _iter_rust_use_statements(text: str) -> list[str]:
    statements: list[str] = []
    pending: list[str] = []
    for line in text.splitlines():
        if _line_is_comment(line):
            continue
        stripped = line.strip()
        if not stripped:
            continue
        if pending:
            pending.append(stripped)
            if ";" in stripped:
                statements.append(" ".join(pending))
                pending.clear()
            continue
        if not stripped.startswith("use "):
            continue
        pending.append(stripped)
        if ";" in stripped:
            statements.append(" ".join(pending))
            pending.clear()
    return statements


def _split_top_level_use_items(text: str) -> list[str]:
    items: list[str] = []
    start = 0
    depth = 0

    for idx, ch in enumerate(text):
        if ch == "{":
            depth += 1
            continue
        if ch == "}":
            depth = max(0, depth - 1)
            continue
        if ch == "," and depth == 0:
            items.append(text[start:idx])
            start = idx + 1

    items.append(text[start:])
    return items


def _find_top_level_char(text: str, target: str) -> int:
    depth = 0
    for idx, ch in enumerate(text):
        if ch == "{":
            if depth == 0 and target == "{":
                return idx
            depth += 1
            continue
        if ch == "}":
            depth = max(0, depth - 1)
            continue
        if ch == target and depth == 0:
            return idx
    return -1


def _find_matching_brace(text: str, start: int) -> int:
    depth = 0
    for idx in range(start, len(text)):
        if text[idx] == "{":
            depth += 1
            continue
        if text[idx] == "}":
            depth -= 1
            if depth == 0:
                return idx
    raise ValueError(f"unmatched brace in use tree: {text!r}")


def _normalize_use_path(text: str) -> str:
    return re.sub(r"\s*::\s*", "::", text.strip())


def _split_use_alias(text: str) -> tuple[str, str | None]:
    alias_idx = _find_top_level_char(text, " ")
    if alias_idx == -1:
        return text, None

    alias_match = re.fullmatch(r"(?P<path>.+?)\s+as\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*)", text)
    if not alias_match:
        return text, None
    return alias_match.group("path"), alias_match.group("alias")


def _path_parts(text: str) -> list[str]:
    normalized = _normalize_use_path(text)
    if not normalized:
        return []
    return [part for part in normalized.split("::") if part]


def _collect_plugin_ids_leaf_binding(
    item: str,
    *,
    prefix_parts: list[str],
    namespace_aliases: set[str],
    imported_constants: set[str],
) -> bool:
    normalized_item = item.strip()
    if not normalized_item:
        return False

    path_text, alias = _split_use_alias(normalized_item)
    normalized_path = _normalize_use_path(path_text)
    if not normalized_path:
        return False

    roots_plugin_ids_namespace = bool(prefix_parts) and prefix_parts[0] in namespace_aliases

    if normalized_path == "self":
        if prefix_parts and (prefix_parts[-1] == "plugin_ids" or roots_plugin_ids_namespace):
            namespace_aliases.add(alias or prefix_parts[-1])
        return False

    parts = prefix_parts + _path_parts(normalized_path)
    if not parts:
        return False

    roots_plugin_ids_namespace = parts[0] in namespace_aliases

    if parts[-1] == "plugin_ids" or (len(parts) == 1 and roots_plugin_ids_namespace):
        namespace_aliases.add(alias or "plugin_ids")
        return False

    if parts[-1] == "*" and (
        (len(parts) >= 2 and parts[-2] == "plugin_ids")
        or (len(parts) >= 2 and roots_plugin_ids_namespace)
    ):
        return True

    if (
        len(parts) >= 2
        and TYPE_TOKEN_RE.fullmatch(parts[-1])
        and ((parts[-2] == "plugin_ids") or roots_plugin_ids_namespace)
    ):
        imported_constants.add(alias or parts[-1])

    return False


def _collect_plugin_ids_bindings_from_use_tree(
    tree: str,
    *,
    prefix_parts: list[str],
    namespace_aliases: set[str],
    imported_constants: set[str],
) -> bool:
    wildcard_imported = False

    for item in _split_top_level_use_items(tree):
        stripped_item = item.strip()
        if not stripped_item:
            continue

        group_start = _find_top_level_char(stripped_item, "{")
        if group_start != -1:
            group_end = _find_matching_brace(stripped_item, group_start)
            head = stripped_item[:group_start].strip()
            if head.endswith("::"):
                head = head[:-2]
            nested_prefix = prefix_parts + _path_parts(head)
            wildcard_imported = _collect_plugin_ids_bindings_from_use_tree(
                stripped_item[group_start + 1 : group_end],
                prefix_parts=nested_prefix,
                namespace_aliases=namespace_aliases,
                imported_constants=imported_constants,
            ) or wildcard_imported
            continue

        wildcard_imported = _collect_plugin_ids_leaf_binding(
            stripped_item,
            prefix_parts=prefix_parts,
            namespace_aliases=namespace_aliases,
            imported_constants=imported_constants,
        ) or wildcard_imported

    return wildcard_imported


def _collect_plugin_ids_import_bindings(text: str) -> tuple[set[str], set[str], bool]:
    namespace_aliases: set[str] = {"plugin_ids"}
    imported_constants: set[str] = set()
    wildcard_imported = False

    use_trees: list[str] = []
    for statement in _iter_rust_use_statements(text):
        use_tree = statement.strip()
        if use_tree.startswith("use "):
            use_tree = use_tree[4:]
        if use_tree.endswith(";"):
            use_tree = use_tree[:-1]
        use_trees.append(use_tree)

    changed = True
    while changed:
        changed = False
        for use_tree in use_trees:
            before_alias_count = len(namespace_aliases)
            before_constant_count = len(imported_constants)
            saw_wildcard = wildcard_imported

            wildcard_imported = _collect_plugin_ids_bindings_from_use_tree(
                use_tree,
                prefix_parts=[],
                namespace_aliases=namespace_aliases,
                imported_constants=imported_constants,
            ) or wildcard_imported

            if (
                len(namespace_aliases) != before_alias_count
                or len(imported_constants) != before_constant_count
                or wildcard_imported != saw_wildcard
            ):
                changed = True

    return namespace_aliases, imported_constants, wildcard_imported


def add_plugin_ids_reference_findings(
    findings: set[Finding],
    *,
    rel_path: str,
    text: str,
    canonical_constant_names: frozenset[str],
) -> None:
    try:
        namespace_aliases, imported_constants, wildcard_imported = (
            _collect_plugin_ids_import_bindings(text)
        )
    except ValueError as exc:
        raise ConfigError(f"{rel_path}: malformed use tree: {exc}") from exc
    if wildcard_imported:
        imported_constants.update(canonical_constant_names)
    module_regexes = {
        alias: re.compile(rf"\b{re.escape(alias)}\s*::\s*[A-Za-z0-9_]+\b")
        for alias in namespace_aliases
    }
    imported_constant_regexes = {
        name: re.compile(rf"\b{re.escape(name)}\b")
        for name in imported_constants
    }

    for line_no, line in enumerate(text.splitlines(), start=1):
        if _line_is_comment(line):
            continue

        for regex in module_regexes.values():
            for match in regex.finditer(line):
                findings.add(
                    Finding(
                        rule_id=RULE_PLUGIN_IDS_REFERENCE,
                        path=rel_path,
                        line=line_no,
                        match_kind="module_token",
                        match_value=match.group(0),
                        excerpt=line.strip(),
                    )
                )

        if line.lstrip().startswith("use "):
            continue

        for regex in imported_constant_regexes.values():
            for match in regex.finditer(line):
                if re.search(r"::\s*$", line[: match.start()]):
                    continue
                findings.add(
                    Finding(
                        rule_id=RULE_PLUGIN_IDS_REFERENCE,
                        path=rel_path,
                        line=line_no,
                        match_kind="module_token",
                        match_value=match.group(0),
                        excerpt=line.strip(),
                    )
                )


def add_plugin_type_id_helper_definition_findings(
    findings: set[Finding],
    *,
    rel_path: str,
    text: str,
) -> None:
    if rel_path != PLUGIN_TYPE_ID_REL_PATH:
        return
    add_regex_findings(
        findings,
        rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
        rel_path=rel_path,
        text=text,
        regex=HELPER_DEF_RE,
        match_kind="api_name",
        match_value_fn=_extract_first_non_empty_group,
    )


def _type_annotation_references_plugin_type_id(
    annotation: str,
    plugin_type_names: set[str],
) -> bool:
    tokens = set(TYPE_TOKEN_RE.findall(annotation))
    return not tokens.isdisjoint(plugin_type_names)


def _is_direct_plugin_type_annotation(
    annotation: str,
    plugin_type_names: set[str],
) -> bool:
    match = re.fullmatch(
        r"(?:&\s*)?(?:mut\s+)?(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)",
        annotation.strip(),
    )
    return bool(match and match.group(1) in plugin_type_names)


def _collect_plugin_type_id_bindings(text: str) -> tuple[set[str], set[str]]:
    aliases = _collect_plugin_type_id_aliases(text)
    plugin_type_names = {"PluginTypeId"} | aliases
    typed_identifiers: set[str] = set()
    collection_identifiers: set[str] = set()

    for line in text.splitlines():
        if _line_is_comment(line):
            continue

        for regex in (PLUGIN_TYPE_ID_TYPED_BINDING_RE, PLUGIN_TYPE_ID_LET_TYPED_BINDING_RE):
            for match in regex.finditer(line):
                identifier = match.group(1)
                annotation = match.group("type")
                if not _type_annotation_references_plugin_type_id(
                    annotation, plugin_type_names
                ):
                    continue
                typed_identifiers.add(identifier)
                if not _is_direct_plugin_type_annotation(annotation, plugin_type_names):
                    collection_identifiers.add(identifier)

        for match in PLUGIN_TYPE_ID_DIRECT_CONSTRUCTOR_BINDING_RE.finditer(line):
            typed_identifiers.add(match.group(1))

    return typed_identifiers, collection_identifiers


def _iter_rust_function_headers(text: str) -> list[str]:
    headers: list[str] = []
    pending: list[str] = []
    paren_depth = 0

    for line in text.splitlines():
        if _line_is_comment(line):
            continue
        if not pending:
            if not re.search(r"\bfn\b", line):
                continue
            pending.append(line.rstrip())
            paren_depth += line.count("(") - line.count(")")
            if paren_depth <= 0 and ("{" in line or ";" in line):
                headers.append("\n".join(pending))
                pending.clear()
                paren_depth = 0
            continue

        pending.append(line.rstrip())
        paren_depth += line.count("(") - line.count(")")
        if paren_depth <= 0 and ("{" in line or ";" in line):
            headers.append("\n".join(pending))
            pending.clear()
            paren_depth = 0

    return headers


def _collect_plugin_type_id_returning_functions(text: str) -> set[str]:
    aliases = _collect_plugin_type_id_aliases(text)
    plugin_type_names = {"PluginTypeId"} | aliases
    functions: set[str] = set()

    for header in _iter_rust_function_headers(text):
        name_match = RUST_FN_NAME_RE.search(header)
        if not name_match or "->" not in header:
            continue
        return_type = header.split("->", 1)[1]
        return_type = re.split(r"\bwhere\b|[{;]", return_type, maxsplit=1)[0]
        if _type_annotation_references_plugin_type_id(
            return_type, plugin_type_names
        ):
            functions.add(name_match.group(1))

    return functions


def _iter_rust_function_headers_with_end_lines(text: str) -> list[tuple[str, int]]:
    headers: list[tuple[str, int]] = []
    pending: list[str] = []
    paren_depth = 0
    end_line = 0

    for line_no, line in enumerate(text.splitlines(), start=1):
        if _line_is_comment(line):
            continue
        if not pending:
            if not re.search(r"\bfn\b", line):
                continue
            pending.append(line.rstrip())
            paren_depth += line.count("(") - line.count(")")
            end_line = line_no
            if paren_depth <= 0 and ("{" in line or ";" in line):
                headers.append(("\n".join(pending), end_line))
                pending.clear()
                paren_depth = 0
            continue

        pending.append(line.rstrip())
        paren_depth += line.count("(") - line.count(")")
        end_line = line_no
        if paren_depth <= 0 and ("{" in line or ";" in line):
            headers.append(("\n".join(pending), end_line))
            pending.clear()
            paren_depth = 0

    return headers


def _build_rust_scope_index(text: str) -> RustScopeIndex:
    lines = text.splitlines()
    parent_by_scope: dict[int, int | None] = {0: None}
    start_line_by_scope: dict[int, int] = {0: 1}
    active_scope_ids_by_line: dict[int, tuple[int, ...]] = {}
    stack = [0]
    next_scope_id = 1
    brace_state = RustBraceScanState()

    for line_no, line in enumerate(lines, start=1):
        line_active_ids = list(stack)
        for brace in _iter_rust_brace_events(line, brace_state):
            if brace == "{":
                scope_id = next_scope_id
                next_scope_id += 1
                parent_by_scope[scope_id] = stack[-1]
                start_line_by_scope[scope_id] = line_no
                stack.append(scope_id)
                line_active_ids.append(scope_id)
                continue
            if len(stack) > 1:
                stack.pop()

        active_scope_ids_by_line[line_no] = tuple(dict.fromkeys(line_active_ids))

    return RustScopeIndex(
        parent_by_scope=parent_by_scope,
        active_scope_ids_by_line=active_scope_ids_by_line,
    )


def _line_scope_ids(scope_index: RustScopeIndex, line_no: int) -> tuple[int, ...]:
    return scope_index.active_scope_ids_by_line.get(line_no, (0,))


def _innermost_scope_id(scope_index: RustScopeIndex, line_no: int) -> int:
    return _line_scope_ids(scope_index, line_no)[-1]


def _visible_scope_bindings(
    line_no: int,
    *,
    scope_index: RustScopeIndex,
    bindings_by_scope: dict[int, set[str]],
) -> set[str]:
    visible: set[str] = set()
    for scope_id in _line_scope_ids(scope_index, line_no):
        visible.update(bindings_by_scope.get(scope_id, ()))
    return visible


def _collect_scope_bound_plugin_type_id_bindings(
    text: str,
) -> tuple[RustScopeIndex, dict[int, set[str]], dict[int, set[str]]]:
    scope_index = _build_rust_scope_index(text)
    aliases = _collect_plugin_type_id_aliases(text)
    plugin_type_names = {"PluginTypeId"} | aliases
    typed_by_scope: dict[int, set[str]] = {}
    collection_by_scope: dict[int, set[str]] = {}
    function_header_line_numbers: set[int] = set()

    def register_binding(line_no: int, identifier: str, annotation: str) -> None:
        if not _type_annotation_references_plugin_type_id(annotation, plugin_type_names):
            return
        scope_id = _innermost_scope_id(scope_index, line_no)
        typed_by_scope.setdefault(scope_id, set()).add(identifier)
        if not _is_direct_plugin_type_annotation(annotation, plugin_type_names):
            collection_by_scope.setdefault(scope_id, set()).add(identifier)

    for header, end_line in _iter_rust_function_headers_with_end_lines(text):
        header_lines = header.splitlines()
        function_header_line_numbers.update(
            range(end_line - len(header_lines) + 1, end_line + 1)
        )
        if "{" not in header:
            continue
        for line in header_lines:
            if _line_is_comment(line):
                continue
            for regex in (PLUGIN_TYPE_ID_TYPED_BINDING_RE, PLUGIN_TYPE_ID_LET_TYPED_BINDING_RE):
                for match in regex.finditer(line):
                    register_binding(end_line, match.group(1), match.group("type"))

    for line_no, line in enumerate(text.splitlines(), start=1):
        if _line_is_comment(line):
            continue
        if line_no in function_header_line_numbers:
            continue

        for regex in (PLUGIN_TYPE_ID_TYPED_BINDING_RE, PLUGIN_TYPE_ID_LET_TYPED_BINDING_RE):
            for match in regex.finditer(line):
                register_binding(line_no, match.group(1), match.group("type"))

        for match in PLUGIN_TYPE_ID_DIRECT_CONSTRUCTOR_BINDING_RE.finditer(line):
            scope_id = _innermost_scope_id(scope_index, line_no)
            typed_by_scope.setdefault(scope_id, set()).add(match.group(1))

    return scope_index, typed_by_scope, collection_by_scope


def _collect_plugin_type_id_fields(
    text: str,
    *,
    plugin_type_names: set[str],
) -> tuple[set[str], set[str]]:
    direct_fields: set[str] = set()
    collection_fields: set[str] = set()

    for line in text.splitlines():
        if _line_is_comment(line):
            continue
        stripped = line.lstrip()
        if stripped.startswith("let ") or re.search(r"\bfn\b", line):
            continue
        for match in PLUGIN_TYPE_ID_TYPED_BINDING_RE.finditer(line):
            annotation = match.group("type")
            if not _type_annotation_references_plugin_type_id(
                annotation,
                plugin_type_names,
            ):
                continue
            if _is_direct_plugin_type_annotation(annotation, plugin_type_names):
                direct_fields.add(match.group(1))
                continue
            collection_fields.add(match.group(1))

    return direct_fields, collection_fields


def _collect_scope_bound_inferred_plugin_type_id_bindings(
    text: str,
    *,
    scope_index: RustScopeIndex,
    typed_by_scope: dict[int, set[str]],
    collection_by_scope: dict[int, set[str]],
    plugin_type_names: set[str],
    plugin_type_returning_functions: set[str],
) -> None:
    changed = True
    while changed:
        changed = False
        for line_no, line in enumerate(text.splitlines(), start=1):
            if _line_is_comment(line):
                continue
            scope_id = _innermost_scope_id(scope_index, line_no)
            visible_typed = _visible_scope_bindings(
                line_no,
                scope_index=scope_index,
                bindings_by_scope=typed_by_scope,
            )
            visible_collections = _visible_scope_bindings(
                line_no,
                scope_index=scope_index,
                bindings_by_scope=collection_by_scope,
            )
            candidate_identifiers = (
                visible_typed | visible_collections | plugin_type_names
            )

            for match in PLUGIN_TYPE_ID_LET_INFERRED_BINDING_RE.finditer(line):
                identifier = match.group(1)
                if identifier in visible_typed or identifier in visible_collections:
                    continue

                expression = match.group("expr")
                expression_mentions_plugin_type = (
                    _expression_mentions_plugin_type_identifier(
                        expression,
                        candidate_identifiers,
                    )
                )
                expression_returns_plugin_type = (
                    _expression_ends_with_plugin_type_returning_call(
                        expression,
                        plugin_type_returning_functions,
                    )
                )
                if not (
                    expression_mentions_plugin_type or expression_returns_plugin_type
                ):
                    continue

                typed_by_scope.setdefault(scope_id, set()).add(identifier)
                changed = True


def _expression_mentions_plugin_type_identifier(
    expression: str,
    plugin_type_identifiers: set[str],
) -> bool:
    tokens = set(TYPE_TOKEN_RE.findall(expression))
    return not tokens.isdisjoint(plugin_type_identifiers)


def _expression_ends_with_plugin_type_returning_call(
    expression: str,
    plugin_type_returning_functions: set[str],
) -> bool:
    expression = expression.rstrip()
    if not expression.endswith(")"):
        return False

    depth = 0
    opening_index = -1
    for index in range(len(expression) - 1, -1, -1):
        char = expression[index]
        if char == ")":
            depth += 1
            continue
        if char != "(":
            continue
        depth -= 1
        if depth == 0:
            opening_index = index
            break
    if opening_index == -1:
        return False

    cursor = opening_index - 1
    while cursor >= 0 and expression[cursor].isspace():
        cursor -= 1
    while cursor >= 0 and expression[cursor] == ">":
        generic_depth = 1
        cursor -= 1
        while cursor >= 0 and generic_depth > 0:
            if expression[cursor] == ">":
                generic_depth += 1
            elif expression[cursor] == "<":
                generic_depth -= 1
            cursor -= 1
    while cursor >= 0 and expression[cursor].isspace():
        cursor -= 1
    while cursor >= 0 and expression[cursor] == ":":
        cursor -= 1
    end = cursor + 1
    while cursor >= 0 and (
        expression[cursor].isalnum() or expression[cursor] in {"_", ":"}
    ):
        cursor -= 1
    if end <= cursor + 1:
        return False
    prefix_cursor = cursor
    while prefix_cursor >= 0 and expression[prefix_cursor].isspace():
        prefix_cursor -= 1
    if prefix_cursor >= 0 and expression[prefix_cursor] == ".":
        return False

    call_path = expression[cursor + 1 : end]
    function_name = call_path.split("::")[-1]
    return function_name in plugin_type_returning_functions


def _collect_inferred_plugin_type_id_bindings(
    text: str,
    *,
    typed_identifiers: set[str],
    collection_identifiers: set[str],
    plugin_type_names: set[str],
    plugin_type_returning_functions: set[str],
) -> set[str]:
    inferred_identifiers: set[str] = set()
    candidate_identifiers = typed_identifiers | collection_identifiers | plugin_type_names

    changed = True
    while changed:
        changed = False
        for line in text.splitlines():
            if _line_is_comment(line):
                continue
            for match in PLUGIN_TYPE_ID_LET_INFERRED_BINDING_RE.finditer(line):
                identifier = match.group(1)
                if (
                    identifier in typed_identifiers
                    or identifier in collection_identifiers
                    or identifier in inferred_identifiers
                ):
                    continue

                expression = match.group("expr")
                expression_mentions_plugin_type = _expression_mentions_plugin_type_identifier(
                    expression,
                    candidate_identifiers,
                )
                expression_returns_plugin_type = (
                    _expression_ends_with_plugin_type_returning_call(
                        expression,
                        plugin_type_returning_functions,
                    )
                )
                if not (
                    expression_mentions_plugin_type or expression_returns_plugin_type
                ):
                    continue

                inferred_identifiers.add(identifier)
                candidate_identifiers.add(identifier)
                changed = True
    return inferred_identifiers


def add_plugin_type_id_helper_callsite_findings(
    findings: set[Finding],
    *,
    rel_path: str,
    text: str,
) -> None:
    lines = text.splitlines()

    def _line_excerpt_for_offset(offset: int) -> tuple[int, str]:
        line_no = _line_no_for_offset(text, offset)
        if 0 < line_no <= len(lines):
            return line_no, lines[line_no - 1].strip()
        return line_no, ""

    aliases = _collect_plugin_type_id_aliases(text)
    plugin_type_names = {"PluginTypeId"} | aliases
    plugin_type_returning_functions = _collect_plugin_type_id_returning_functions(text)
    direct_field_identifiers, collection_field_identifiers = _collect_plugin_type_id_fields(
        text,
        plugin_type_names=plugin_type_names,
    )
    scope_index, typed_by_scope, collection_by_scope = (
        _collect_scope_bound_plugin_type_id_bindings(text)
    )
    _collect_scope_bound_inferred_plugin_type_id_bindings(
        text,
        scope_index=scope_index,
        typed_by_scope=typed_by_scope,
        collection_by_scope=collection_by_scope,
        plugin_type_names=plugin_type_names,
        plugin_type_returning_functions=plugin_type_returning_functions,
    )

    for line_no, line in enumerate(text.splitlines(), start=1):
        if _line_is_comment(line):
            continue

        typed_identifiers = _visible_scope_bindings(
            line_no,
            scope_index=scope_index,
            bindings_by_scope=typed_by_scope,
        )
        if rel_path == PLUGIN_TYPE_ID_REL_PATH:
            typed_identifiers.add("self")
        collection_identifiers = _visible_scope_bindings(
            line_no,
            scope_index=scope_index,
            bindings_by_scope=collection_by_scope,
        )
        plugin_type_identifiers = typed_identifiers | collection_identifiers

        for match in PLUGIN_TYPE_ID_ANY_ASSOC_CALL_RE.finditer(line):
            if match.group(1) not in plugin_type_names:
                continue
            findings.add(
                Finding(
                    rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                    path=rel_path,
                    line=line_no,
                    match_kind="api_name",
                    match_value=match.group(2),
                    excerpt=line.strip(),
                )
            )

        for match in PLUGIN_TYPE_ID_ASSOC_FUNCTION_ITEM_RE.finditer(line):
            if match.group(1) not in plugin_type_names:
                continue
            findings.add(
                Finding(
                    rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                    path=rel_path,
                    line=line_no,
                    match_kind="api_name",
                    match_value=match.group(2),
                    excerpt=line.strip(),
                )
            )

        for match in PLUGIN_TYPE_ID_METHOD_CALL_RE.finditer(line):
            receiver = match.group(1)
            if receiver not in typed_identifiers:
                continue
            findings.add(
                Finding(
                    rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                    path=rel_path,
                    line=line_no,
                    match_kind="api_name",
                    match_value=match.group(2),
                    excerpt=line.strip(),
                )
            )

        for match in PLUGIN_TYPE_ID_SELF_FIELD_METHOD_CALL_RE.finditer(line):
            if match.group("receiver") not in direct_field_identifiers:
                continue
            findings.add(
                Finding(
                    rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                    path=rel_path,
                    line=line_no,
                    match_kind="api_name",
                    match_value=match.group("api"),
                    excerpt=line.strip(),
                )
            )

        for match in PLUGIN_TYPE_ID_COMPLEX_RECEIVER_METHOD_CALL_RE.finditer(line):
            receiver_expression = line[: match.start()].rstrip()
            if not receiver_expression:
                continue
            if receiver_expression[-1] not in ")}]":
                continue
            expression_mentions_plugin_identifier = (
                _expression_mentions_plugin_type_identifier(
                    receiver_expression, plugin_type_identifiers
                )
            )
            expression_returns_plugin_type = (
                _expression_ends_with_plugin_type_returning_call(
                    receiver_expression, plugin_type_returning_functions
                )
            )
            if not (
                expression_mentions_plugin_identifier
                or expression_returns_plugin_type
            ):
                continue
            findings.add(
                Finding(
                    rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                    path=rel_path,
                    line=line_no,
                    match_kind="api_name",
                    match_value=match.group(1),
                    excerpt=line.strip(),
                )
            )

        for match in PLUGIN_TYPE_ID_INDEXED_METHOD_CALL_RE.finditer(line):
            receiver = match.group("receiver")
            prefix = match.group("prefix")
            is_self_field_receiver = "self" in TYPE_TOKEN_RE.findall(prefix)
            if (
                receiver not in collection_identifiers
                and not (
                    is_self_field_receiver and receiver in collection_field_identifiers
                )
            ):
                continue
            findings.add(
                Finding(
                    rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                    path=rel_path,
                    line=line_no,
                    match_kind="api_name",
                    match_value=match.group("api"),
                    excerpt=line.strip(),
                )
            )

    for match in PLUGIN_TYPE_ID_MULTILINE_ASSOC_CALL_RE.finditer(text):
        if match.group("receiver") not in plugin_type_names:
            continue
        line_no, excerpt = _line_excerpt_for_offset(match.start("api"))
        findings.add(
            Finding(
                rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                path=rel_path,
                line=line_no,
                match_kind="api_name",
                match_value=match.group("api"),
                excerpt=excerpt,
            )
        )

    for match in PLUGIN_TYPE_ID_MULTILINE_METHOD_CALL_RE.finditer(text):
        line_no, excerpt = _line_excerpt_for_offset(match.start("api"))
        typed_identifiers = _visible_scope_bindings(
            line_no,
            scope_index=scope_index,
            bindings_by_scope=typed_by_scope,
        )
        if rel_path == PLUGIN_TYPE_ID_REL_PATH:
            typed_identifiers.add("self")
        if match.group("receiver") not in typed_identifiers:
            continue
        findings.add(
            Finding(
                rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                path=rel_path,
                line=line_no,
                match_kind="api_name",
                match_value=match.group("api"),
                excerpt=excerpt,
            )
        )

    for match in PLUGIN_TYPE_ID_SELF_FIELD_MULTILINE_METHOD_CALL_RE.finditer(text):
        if match.group("receiver") not in direct_field_identifiers:
            continue
        line_no, excerpt = _line_excerpt_for_offset(match.start("api"))
        findings.add(
            Finding(
                rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                path=rel_path,
                line=line_no,
                match_kind="api_name",
                match_value=match.group("api"),
                excerpt=excerpt,
            )
        )

    for match in PLUGIN_TYPE_ID_MULTILINE_COMPLEX_RECEIVER_METHOD_CALL_RE.finditer(text):
        receiver_expression = match.group("receiver").rstrip()
        line_no, excerpt = _line_excerpt_for_offset(match.start("api"))
        typed_identifiers = _visible_scope_bindings(
            line_no,
            scope_index=scope_index,
            bindings_by_scope=typed_by_scope,
        )
        if rel_path == PLUGIN_TYPE_ID_REL_PATH:
            typed_identifiers.add("self")
        collection_identifiers = _visible_scope_bindings(
            line_no,
            scope_index=scope_index,
            bindings_by_scope=collection_by_scope,
        )
        plugin_type_identifiers = typed_identifiers | collection_identifiers
        expression_mentions_plugin_identifier = (
            _expression_mentions_plugin_type_identifier(
                receiver_expression, plugin_type_identifiers
            )
        )
        expression_returns_plugin_type = (
            _expression_ends_with_plugin_type_returning_call(
                receiver_expression, plugin_type_returning_functions
            )
        )
        if not (
            expression_mentions_plugin_identifier or expression_returns_plugin_type
        ):
            continue
        findings.add(
            Finding(
                rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                path=rel_path,
                line=line_no,
                match_kind="api_name",
                match_value=match.group("api"),
                excerpt=excerpt,
            )
        )


def _contains_plugin_crate_name(name: str) -> bool:
    return bool(PLUGIN_CRATE_NAME_RE.match(name))


def _resolve_workspace_dependency_plugin_name(dep: object) -> str | None:
    if isinstance(dep, str):
        if _contains_plugin_crate_name(dep):
            return dep
        return None
    if not isinstance(dep, dict):
        return None

    package = dep.get("package")
    if isinstance(package, str) and _contains_plugin_crate_name(package):
        return package
    return None


def load_workspace_dependency_map(root: Path) -> dict[str, str]:
    if tomllib is None:
        raise ConfigError("Python tomllib is unavailable")
    workspace_manifest = root / "Cargo.toml"
    if not workspace_manifest.exists():
        return {}

    data = tomllib.loads(workspace_manifest.read_text(encoding="utf-8"))
    workspace = data.get("workspace", {})
    if not isinstance(workspace, dict):
        return {}
    workspace_deps = workspace.get("dependencies", {})
    if not isinstance(workspace_deps, dict):
        return {}

    resolved: dict[str, str] = {}
    for dep_key, dep_spec in workspace_deps.items():
        if not isinstance(dep_key, str):
            continue
        plugin_name = _resolve_workspace_dependency_plugin_name(dep_spec)
        if plugin_name is None and _contains_plugin_crate_name(dep_key):
            plugin_name = dep_key
        if plugin_name is not None:
            resolved[dep_key] = plugin_name
    return resolved


def _is_test_target_name(target_name: str) -> bool:
    match = re.fullmatch(r"\s*cfg\s*\((?P<expr>.*)\)\s*", target_name)
    if not match:
        return False
    return _cfg_expression_requires_test(match.group("expr"))


def _iter_manifest_dependency_tables(data: dict[str, object]) -> list[dict[str, object]]:
    tables: list[dict[str, object]] = []
    for section in ("dependencies", "build-dependencies"):
        section_value = data.get(section)
        if isinstance(section_value, dict):
            tables.append(section_value)

    target = data.get("target")
    if not isinstance(target, dict):
        return tables

    for target_name, target_table in target.items():
        if not isinstance(target_name, str) or not isinstance(target_table, dict):
            continue
        if _is_test_target_name(target_name):
            continue
        for section in ("dependencies", "build-dependencies"):
            section_value = target_table.get(section)
            if isinstance(section_value, dict):
                tables.append(section_value)
    return tables


def _resolve_manifest_dep_plugin_name(
    dep_key: str,
    dep_spec: object,
    *,
    workspace_deps: dict[str, str],
) -> str | None:
    if _contains_plugin_crate_name(dep_key):
        return dep_key

    if isinstance(dep_spec, str):
        return None
    if not isinstance(dep_spec, dict):
        return None

    package_name = dep_spec.get("package")
    if isinstance(package_name, str) and _contains_plugin_crate_name(package_name):
        return package_name

    if dep_spec.get("workspace") is True:
        return workspace_deps.get(dep_key)
    return None


def _find_manifest_location(text: str, dep_key: str, plugin_name: str) -> tuple[int, str]:
    key_pattern = re.compile(rf"^\s*{re.escape(dep_key)}\s*=", re.MULTILINE)
    package_pattern = re.compile(rf'package\s*=\s*"{re.escape(plugin_name)}"')

    lines = text.splitlines()
    for line_no, line in enumerate(lines, start=1):
        if key_pattern.search(line):
            return line_no, line.strip()
    for line_no, line in enumerate(lines, start=1):
        if package_pattern.search(line):
            return line_no, line.strip()
    return 1, lines[0].strip() if lines else ""


def collect_manifest_findings(
    *,
    findings: set[Finding],
    path: Path,
    rel_path: str,
    workspace_deps: dict[str, str],
) -> None:
    if tomllib is None:
        raise ConfigError("Python tomllib is unavailable")

    text = path.read_text(encoding="utf-8")
    data = tomllib.loads(text)

    for dep_table in _iter_manifest_dependency_tables(data):
        for dep_key, dep_spec in dep_table.items():
            if not isinstance(dep_key, str):
                continue
            plugin_name = _resolve_manifest_dep_plugin_name(
                dep_key,
                dep_spec,
                workspace_deps=workspace_deps,
            )
            if plugin_name is None:
                continue
            if plugin_name in MANIFEST_ALLOWED_PLUGIN_DEPENDENCIES:
                continue
            line, excerpt = _find_manifest_location(text, dep_key, plugin_name)
            findings.add(
                Finding(
                    rule_id=RULE_MANIFEST_PLUGIN_DEPENDENCY,
                    path=rel_path,
                    line=line,
                    match_kind="manifest_dependency",
                    match_value=plugin_name,
                    excerpt=excerpt,
                )
            )


def _extract_literal_matches_from_line(line: str, suffix: str) -> list[tuple[int, int, str]]:
    matches: list[tuple[int, int, str]] = []

    if suffix == ".rs":
        for match in RUST_RAW_STRING_RE.finditer(line):
            matches.append((match.start(), match.end(), match.group("body")))
        for match in RUST_STRING_RE.finditer(line):
            matches.append((match.start(), match.end(), match.group("body")))
    else:
        for regex in (TS_DOUBLE_QUOTED_RE, TS_SINGLE_QUOTED_RE, TS_TEMPLATE_RE):
            for match in regex.finditer(line):
                if regex is TS_TEMPLATE_RE and _template_body_has_interpolation(
                    match.group("body")
                ):
                    continue
                matches.append((match.start(), match.end(), match.group("body")))

    matches.sort(key=lambda item: (item[0], item[1]))
    filtered_matches: list[tuple[int, int, str]] = []
    last_end = -1
    for start, end, value in matches:
        if start < last_end:
            continue
        filtered_matches.append((start, end, value))
        last_end = end
    return filtered_matches


def _extract_literals_from_line(line: str, suffix: str) -> list[str]:
    matches = _extract_literal_matches_from_line(line, suffix)
    literals: list[str] = []
    for _, _, value in matches:
        literals.append(value)
    return literals


def _extract_literal_entries(text: str, suffix: str) -> list[tuple[int, str]]:
    literal_regexes: tuple[re.Pattern[str], ...]
    if suffix == ".rs":
        literal_regexes = (RUST_RAW_STRING_MULTILINE_RE, RUST_STRING_MULTILINE_RE)
    else:
        literal_regexes = (TS_DOUBLE_QUOTED_RE, TS_SINGLE_QUOTED_RE, TS_TEMPLATE_RE)

    matches: list[tuple[int, int, str]] = []
    for regex in literal_regexes:
        for match in regex.finditer(text):
            if suffix != ".rs" and regex is TS_TEMPLATE_RE and _template_body_has_interpolation(
                match.group("body")
            ):
                continue
            matches.append((match.start(), match.end(), match.group("body")))

    matches.sort(key=lambda item: (item[0], item[1]))
    entries: list[tuple[int, str]] = []
    last_end = -1
    for start, end, literal in matches:
        if start < last_end:
            continue
        entries.append((_line_no_for_offset(text, start), literal))
        last_end = end
    return entries


def _is_plugin_identity_context(
    context_text: str,
    current_line: str,
    literal_value: str,
) -> bool:
    context_lower = context_text.lower()
    current_line_lower = current_line.lower()
    literal_lower = literal_value.lower()
    has_same_line_hint = any(
        regex.search(current_line_lower) for regex in IDENTITY_SAME_LINE_HINT_RES
    )
    has_context_hint = any(regex.search(context_lower) for regex in IDENTITY_CONTEXT_HINT_RES)
    has_route_hint = bool(PLUGIN_TYPES_ROUTE_RE.search(context_lower)) or bool(
        PLUGIN_TYPES_ROUTE_RE.search(literal_lower)
    )
    return has_same_line_hint or has_context_hint or has_route_hint


def _template_body_has_interpolation(template_body: str) -> bool:
    cursor = 0
    while True:
        idx = template_body.find("${", cursor)
        if idx == -1:
            return False
        backslash_count = 0
        j = idx - 1
        while j >= 0 and template_body[j] == "\\":
            backslash_count += 1
            j -= 1
        if backslash_count % 2 == 0:
            return True
        cursor = idx + 2


def _contains_plugin_id_as_token(plugin_id: str, literal_value: str) -> bool:
    token_re = re.compile(rf"(?<![a-z0-9_]){re.escape(plugin_id)}(?![a-z0-9_])")
    return bool(token_re.search(literal_value))


def _literal_has_identity_key_value_for_plugin_id(
    literal_value: str,
    plugin_id: str,
) -> bool:
    plugin_id_lower = plugin_id.lower()
    for match in IDENTITY_LITERAL_KEY_VALUE_RE.finditer(literal_value):
        if match.group("value").lower() == plugin_id_lower:
            return True
    return False


def _is_identity_context_boundary(line: str) -> bool:
    stripped = line.strip()
    if not stripped:
        return False
    if stripped in {"{", "}"}:
        return True
    return stripped.endswith(";")


def _build_identity_context(
    lines: list[str],
    line_no: int,
    *,
    center_line_override: str | None = None,
) -> str:
    if not lines:
        return ""

    center = max(0, min(len(lines) - 1, line_no - 1))
    context_indices: set[int] = {center}

    significant_before = 0
    idx = center - 1
    while idx >= 0 and significant_before < 8:
        if center - idx > 40:
            break
        if lines[idx].strip():
            if _is_identity_context_boundary(lines[idx]):
                break
            context_indices.add(idx)
            significant_before += 1
        idx -= 1

    significant_after = 0
    idx = center + 1
    while idx < len(lines) and significant_after < 4:
        if idx - center > 20:
            break
        if lines[idx].strip():
            if _is_identity_context_boundary(lines[idx]):
                break
            context_indices.add(idx)
            significant_after += 1
        idx += 1

    context_lines: list[str] = []
    for idx in sorted(context_indices):
        if center_line_override is not None and idx == center:
            context_lines.append(center_line_override)
            continue
        context_lines.append(lines[idx])
    return "\n".join(context_lines)


def _mask_matching_literal_occurrences(
    line: str,
    *,
    suffix: str,
    literal_value: str,
) -> str:
    masked = line
    literal_matches = _extract_literal_matches_from_line(line, suffix)
    for start, end, body in reversed(literal_matches):
        if body != literal_value:
            continue
        masked = masked[:start] + (" " * (end - start)) + masked[end:]
    return masked


def add_literal_findings(
    findings: set[Finding],
    *,
    rel_path: str,
    text: str,
    suffix: str,
    canonical_plugin_ids: frozenset[str],
    exempt_lines: frozenset[int],
) -> None:
    if not canonical_plugin_ids:
        return

    lines = text.splitlines()
    literal_entries = _extract_literal_entries(text, suffix)
    seen_at_location: set[tuple[int, str]] = set()

    for line_no, literal in literal_entries:
        if line_no in exempt_lines:
            continue

        line_text = lines[line_no - 1] if 0 < line_no <= len(lines) else ""
        masked_line = _mask_matching_literal_occurrences(
            line_text,
            suffix=suffix,
            literal_value=literal,
        )
        context = _build_identity_context(
            lines,
            line_no,
            center_line_override=masked_line,
        )

        has_identity_context = _is_plugin_identity_context(context, masked_line, literal)

        for plugin_id in canonical_plugin_ids:
            if not _contains_plugin_id_as_token(plugin_id, literal):
                continue
            if not has_identity_context and not _literal_has_identity_key_value_for_plugin_id(
                literal,
                plugin_id,
            ):
                continue

            location = (line_no, plugin_id)
            if location in seen_at_location:
                continue
            seen_at_location.add(location)
            findings.add(
                Finding(
                    rule_id=RULE_HARDCODED_PLUGIN_TYPE_LITERAL,
                    path=rel_path,
                    line=line_no,
                    match_kind="literal_string",
                    match_value=plugin_id,
                    excerpt=line_text.strip(),
                )
            )


def _is_legacy_dashboard_bespoke_surface(rel_path: str) -> bool:
    normalized = rel_path.replace("\\", "/")
    if normalized == "crates/ui/web-api/src/router.rs":
        return True
    if normalized == "crates/ui/web-api/db_access_policy.toml":
        return True
    if normalized.startswith("crates/ui/web-api/src/routes/") and normalized.endswith(".rs"):
        return True
    if normalized.startswith("crates/ui/web-api-auth/src/") and normalized.endswith(".rs"):
        return True
    if normalized.startswith("crates/shared/web-api-types/src/") and normalized.endswith(".rs"):
        return True
    return False


def validate_target_sets(root: Path) -> None:
    rust_production_count = 0
    frontend_production_count = 0
    manifest_count = 0

    for path in root.rglob("*"):
        if not path.is_file():
            continue
        rel = posix_rel(path, root)
        normalized = rel.replace("\\", "/")
        suffix = path.suffix.lower()
        if should_scan_manifest(rel):
            manifest_count += 1
        if not looks_like_production_code(rel):
            continue
        if normalized.startswith("crates/") and suffix == ".rs":
            rust_production_count += 1
            continue
        if normalized.startswith("frontend/src/") and suffix in {".ts", ".js", ".svelte"}:
            frontend_production_count += 1

    missing_slices: list[str] = []
    if rust_production_count == 0:
        missing_slices.append("rust production target set matched 0 files")
    if frontend_production_count == 0:
        missing_slices.append("frontend production target set matched 0 files")
    if manifest_count == 0:
        missing_slices.append("manifest target set matched 0 files")
    if missing_slices:
        raise ConfigError(
            "target-set misconfiguration: " + "; ".join(missing_slices)
        )


def collect_findings(root: Path) -> list[Finding]:
    findings: set[Finding] = set()
    validate_target_sets(root)
    canonical = discover_canonical_plugin_ids(root)
    workspace_deps = load_workspace_dependency_map(root)

    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue

        rel = posix_rel(path, root)
        suffix = path.suffix.lower()
        if _is_legacy_dashboard_bespoke_surface(rel) and suffix == ".toml":
            text = path.read_text(encoding="utf-8")
            add_regex_findings(
                findings,
                rule_id=RULE_LEGACY_DASHBOARD_BESPOKE_SURFACE,
                rel_path=rel,
                text=text,
                regex=DASHBOARD_BESPOKE_LITERAL_RE,
                match_kind="literal_string",
            )
            continue

        if should_scan_manifest(rel):
            collect_manifest_findings(
                findings=findings,
                path=path,
                rel_path=rel,
                workspace_deps=workspace_deps,
            )
            continue

        if not looks_like_production_code(rel):
            continue
        if suffix not in {".rs", ".ts", ".js", ".svelte"}:
            continue

        text = path.read_text(encoding="utf-8")
        if suffix == ".rs":
            text = preprocess_rust(text)
        text = strip_block_comments(text, suffix)
        text = strip_inline_line_comments(text, suffix)

        if _is_legacy_dashboard_bespoke_surface(rel):
            add_regex_findings(
                findings,
                rule_id=RULE_LEGACY_DASHBOARD_BESPOKE_SURFACE,
                rel_path=rel,
                text=text,
                regex=DASHBOARD_BESPOKE_LITERAL_RE,
                match_kind="literal_string",
            )

        if suffix == ".rs":
            regex_scan_text = strip_rust_string_literals(text)
            add_regex_findings(
                findings,
                rule_id=RULE_PLUGIN_CORE_IMPORT,
                rel_path=rel,
                text=regex_scan_text,
                regex=CORE_IMPORT_TOKEN_RE,
                match_kind="import_path",
            )
            add_regex_findings(
                findings,
                rule_id=RULE_CONCRETE_PLUGIN_IMPORT,
                rel_path=rel,
                text=regex_scan_text,
                regex=CONCRETE_IMPORT_TOKEN_RE,
                match_kind="crate_name",
                match_value_fn=_extract_import_crate_name,
                skip_match=lambda value: not _is_concrete_plugin_import(value),
            )
            add_plugin_ids_reference_findings(
                findings,
                rel_path=rel,
                text=regex_scan_text,
                canonical_constant_names=canonical.constant_names,
            )
            add_plugin_type_id_helper_definition_findings(
                findings,
                rel_path=rel,
                text=regex_scan_text,
            )
            add_plugin_type_id_helper_callsite_findings(
                findings,
                rel_path=rel,
                text=regex_scan_text,
            )
            add_regex_findings(
                findings,
                rule_id=RULE_FORBIDDEN_PLUGIN_HELPER,
                rel_path=rel,
                text=regex_scan_text,
                regex=IDENTITY_SPECIFIC_HELPER_RE,
                match_kind="api_name",
                match_value_fn=_extract_first_non_empty_group,
            )

        exempt_lines = canonical.exempt_lines_by_path.get(rel, frozenset())
        add_literal_findings(
            findings,
            rel_path=rel,
            text=text,
            suffix=suffix,
            canonical_plugin_ids=canonical.ids,
            exempt_lines=exempt_lines,
        )

    return sorted(findings)


def normalize_path(raw: str) -> str:
    normalized = raw.replace("\\", "/")
    if normalized.startswith("./"):
        normalized = normalized[2:]
    return normalized


def load_allowlist(path: Path) -> list[AllowlistEntry]:
    if tomllib is None:
        raise ConfigError("Python tomllib is unavailable")
    if not path.exists():
        raise ConfigError(f"allowlist file does not exist: {path}")

    data = tomllib.loads(path.read_text(encoding="utf-8"))
    version = data.get("version")
    if version != 1:
        raise ConfigError("allowlist version must be 1")

    entries_data = data.get("entries", [])
    if not isinstance(entries_data, list):
        raise ConfigError("allowlist entries must be a list")
    if not entries_data:
        raise ConfigError(
            "allowlist entries must not be empty; delete allowlist file for zero-tolerance mode"
        )

    entries: list[AllowlistEntry] = []
    for idx, raw in enumerate(entries_data):
        if not isinstance(raw, dict):
            raise ConfigError(f"allowlist entry {idx} must be a table")
        try:
            entry = AllowlistEntry(
                path=normalize_path(str(raw["path"])),
                rule_id=str(raw["rule_id"]),
                match_kind=str(raw["match_kind"]),
                match_value=str(raw["match_value"]),
                reason=str(raw["reason"]),
            )
        except KeyError as exc:
            raise ConfigError(f"allowlist entry {idx} missing key: {exc.args[0]}") from exc

        if BLOCK_GLOB_RE.search(entry.path):
            raise ConfigError(
                f"allowlist entry {idx} path glob patterns are not allowed: {entry.path}"
            )
        if entry.rule_id not in KNOWN_RULE_IDS:
            raise ConfigError(f"allowlist entry {idx} unknown rule_id: {entry.rule_id}")
        if entry.match_kind not in ALLOWED_MATCH_KINDS:
            raise ConfigError(f"allowlist entry {idx} invalid match_kind: {entry.match_kind}")
        if entry.match_kind not in RULE_MATCH_KINDS[entry.rule_id]:
            raise ConfigError(
                f"allowlist entry {idx} invalid match_kind '{entry.match_kind}' for rule_id '{entry.rule_id}'"
            )
        if not entry.reason.strip():
            raise ConfigError(f"allowlist entry {idx} reason must not be empty")
        if REGEX_LIKE_MATCH_VALUE_RE.search(entry.match_value):
            raise ConfigError(
                f"allowlist entry {idx} regex-like patterns are not allowed in match_value: {entry.match_value}"
            )
        entries.append(entry)
    return entries


def apply_allowlist(findings: list[Finding], allowlist_entries: list[AllowlistEntry]) -> list[Finding]:
    allow_keys = {
        (entry.rule_id, entry.path, entry.match_kind, entry.match_value)
        for entry in allowlist_entries
    }
    filtered = [
        finding
        for finding in findings
        if (finding.rule_id, finding.path, finding.match_kind, finding.match_value) not in allow_keys
    ]
    return sorted(filtered)


def render_text(findings: list[Finding]) -> str:
    if not findings:
        return "semantic-boundary clean"
    lines = [f"semantic-boundary violations: {len(findings)}"]
    for finding in findings:
        lines.append(
            "{rule} {path}:{line} match_kind={kind} match_value={value} excerpt={excerpt}".format(
                rule=finding.rule_id,
                path=finding.path,
                line=finding.line,
                kind=finding.match_kind,
                value=finding.match_value,
                excerpt=finding.excerpt,
            )
        )
    return "\n".join(lines)


def render_json(findings: list[Finding]) -> str:
    payload = {
        "status": "clean" if not findings else "violations",
        "findings": [
            {
                "rule_id": finding.rule_id,
                "path": finding.path,
                "line": finding.line,
                "match_kind": finding.match_kind,
                "match_value": finding.match_value,
                "excerpt": finding.excerpt,
            }
            for finding in findings
        ],
    }
    return json.dumps(payload, indent=2, sort_keys=True)


def render(findings: list[Finding], output_format: str) -> str:
    if output_format == "json":
        return render_json(findings)
    return render_text(findings)


def resolve_allowlist_path(root: Path, arg_value: str | None) -> Path | None:
    if arg_value:
        return Path(arg_value)

    default_path = root / "ci" / "plugin_semantic_boundary_allowlist.toml"
    if default_path.exists():
        return default_path
    return None


def main(argv: list[str]) -> int:
    try:
        args = parse_args(argv)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else EXIT_CONFIG_ERROR
        return code

    root = Path(args.root)
    if not root.exists() or not root.is_dir():
        print(f"semantic-boundary config error: invalid root path: {root}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    try:
        findings = collect_findings(root)
        allowlist_path = resolve_allowlist_path(root, args.allowlist)
        if allowlist_path is not None:
            allowlist_entries = load_allowlist(allowlist_path)
            findings = apply_allowlist(findings, allowlist_entries)
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError if tomllib else Exception, ConfigError) as exc:
        print(f"semantic-boundary config error: {exc}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    print(render(findings, args.format))
    if findings:
        return EXIT_VIOLATIONS
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
