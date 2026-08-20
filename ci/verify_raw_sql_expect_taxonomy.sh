#!/usr/bin/env bash
# verify_raw_sql_expect_taxonomy.sh — shape gate for the raw-SQL clippy
# `disallowed-methods`/`disallowed-macros` `#[expect]` escape hatch.
#
# Every surviving raw-SQL site opts out with
# `#[expect(clippy::disallowed_methods, reason = "<category>: <concrete
# limitation>")]` (or `disallowed_macros` for `raw_sql!`), where `<category>`
# is exactly one of the four taxonomy categories documented in
# docs/development/coding-standards.md (Raw-SQL ban). Without this gate
# nothing mechanically enforces that taxonomy: a future
# `reason = "needed for this query"` or a top-of-file `#![expect(...)]` would
# pass every other check.
#
# Checks:
#   (a) every disallowed_methods/disallowed_macros `#[expect]` reason string
#       starts with an allowed prefix (path-scoped: the four taxonomy
#       categories everywhere, plus the pre-existing db-tx-specific reasons
#       only inside crates/shared/db-tx/src/lib.rs).
#   (b) no file-level `#![expect(clippy::disallowed_methods|disallowed_macros
#       …)]` anywhere in crates/ or xtask/.
#   (c) every "frozen merged migration" reason lives in a migration path (a
#       path with a migration/migrations directory component, or a file
#       named migration.rs / *_migration.rs).
#   (d) the total "frozen merged migration" occurrence count matches the
#       pinned literal below — the ratchet. Category 4 is the only category
#       that grants permission regardless of expressibility, and "merged to
#       main" is unknowable from source text alone; without this pin the
#       category is a self-certifying copy-paste precedent. Decrementing
#       (deleting/rewriting a frozen migration site) is always fine and needs
#       no action here; incrementing means a new raw-SQL migration landed and
#       requires explicit owner sign-off (update the pin in the same change).
#
# See docs/development/coding-standards.md § Raw-SQL ban for the full
# taxonomy and rationale.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v python3 >/dev/null 2>&1; then
  echo "verify_raw_sql_expect_taxonomy: python3 is required" >&2
  exit 1
fi

python3 - <<'PYEOF'
import os
import re
import sys

# The four taxonomy categories from docs/development/coding-standards.md
# § Raw-SQL ban. Reason strings take the shape "<category>: <concrete
# limitation>" — allowed everywhere in crates/ and xtask/.
TAXONOMY_PREFIXES = (
    "builder limitation:",
    "connectivity probe:",
    "test-only schema sabotage:",
    "frozen merged migration:",
)

# Pre-existing db-tx reasons, allowed ONLY in crates/shared/db-tx/src/lib.rs —
# otherwise e.g. "canary: proves the" becomes a taxonomy bypass anywhere in
# the tree. One lint name (clippy::disallowed_methods) covers both the
# raw-SQL ban and the begin*/transaction* ban family that db-tx's own canary
# module exercises, hence these reasons living under the same lint.
DB_TX_PATH = "crates/shared/db-tx/src/lib.rs"
DB_TX_PREFIXES = (
    "canary: proves the",
    "negative control:",
    "the workspace's sole sanctioned begin_with_options call site",
)

# The ratchet: current count of "frozen merged migration" reasons. Verified
# by hand against `rg -o 'frozen merged migration' crates/ xtask/ | wc -l`
# before pinning. Decrementing this number (a frozen migration is
# deleted/rewritten) is always fine — lower the pin to match. Incrementing it
# means a NEW raw-SQL migration landed under category 4 and needs explicit
# owner sign-off before the pin is raised.
PINNED_FROZEN_MERGED_MIGRATION_COUNT = 87

SCAN_DIRS = ("crates", "xtask")
DOC_POINTER = "docs/development/coding-standards.md § Raw-SQL ban"


def find_rs_files(dirs):
    for d in dirs:
        if not os.path.isdir(d):
            continue
        for root, _dirnames, filenames in os.walk(d):
            for name in filenames:
                if name.endswith(".rs"):
                    yield os.path.join(root, name)


def is_migration_path(path):
    parts = path.split(os.sep)
    if "migration" in parts or "migrations" in parts:
        return True
    base = os.path.basename(path)
    return base == "migration.rs" or base.endswith("_migration.rs")


def scan_expect_attrs(text):
    """Yield dicts describing every #[expect(...)] / #![expect(...)] block
    whose body mentions clippy::disallowed_methods or clippy::disallowed_macros.

    Rustfmt splits `#[expect(`, the lint path, and `reason = "…"` across
    lines, so this walks characters (tracking paren depth and string-literal
    state) rather than matching line-by-line or with a single-line regex.
    """
    results = []
    for m in re.finditer(r"#(!?)\[expect\(", text):
        inner = m.group(1) == "!"
        i = m.end()
        depth = 1
        in_string = False
        escape = False
        n = len(text)
        while i < n and depth > 0:
            c = text[i]
            if in_string:
                if escape:
                    escape = False
                elif c == "\\":
                    escape = True
                elif c == '"':
                    in_string = False
            else:
                if c == '"':
                    in_string = True
                elif c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
            i += 1
        body_start = m.end()
        body_end = i - 1  # exclude the closing ')'
        body = text[body_start:body_end]

        lints = set()
        if "clippy::disallowed_methods" in body:
            lints.add("clippy::disallowed_methods")
        if "clippy::disallowed_macros" in body:
            lints.add("clippy::disallowed_macros")
        if not lints:
            continue

        reason = None
        reason_line = None
        rm = re.search(r'reason\s*=\s*"((?:\\.|[^"\\])*)"', body)
        if rm:
            reason = rm.group(1)
            reason_abs = body_start + rm.start(1)
            reason_line = text[:reason_abs].count("\n") + 1

        attr_line = text[: m.start()].count("\n") + 1
        results.append(
            {
                "inner": inner,
                "attr_line": attr_line,
                "reason": reason,
                "reason_line": reason_line,
            }
        )
    return results


def main():
    violations = []
    frozen_count = 0

    for path in sorted(find_rs_files(SCAN_DIRS)):
        with open(path, encoding="utf-8") as fh:
            text = fh.read()

        for attr in scan_expect_attrs(text):
            if attr["inner"]:
                violations.append(
                    f"{path}:{attr['attr_line']}: file-level "
                    f"#![expect(clippy::disallowed…)] is banned — use a "
                    f"statement-level #[expect] instead ({DOC_POINTER})"
                )
                continue

            reason = attr["reason"]
            line = attr["reason_line"] or attr["attr_line"]

            if reason is None:
                violations.append(
                    f"{path}:{attr['attr_line']}: "
                    f"#[expect(clippy::disallowed…)] has no reason string "
                    f"({DOC_POINTER})"
                )
                continue

            allowed_prefixes = TAXONOMY_PREFIXES
            if path == DB_TX_PATH:
                allowed_prefixes = TAXONOMY_PREFIXES + DB_TX_PREFIXES

            if not any(reason.startswith(p) for p in allowed_prefixes):
                violations.append(
                    f'{path}:{line}: reason "{reason}" does not start with '
                    f"an allowed prefix ({DOC_POINTER})"
                )

            if reason.startswith("frozen merged migration"):
                frozen_count += 1
                if not is_migration_path(path):
                    violations.append(
                        f'{path}:{line}: "frozen merged migration" reason '
                        f"used outside a migration path ({DOC_POINTER})"
                    )

    if frozen_count != PINNED_FROZEN_MERGED_MIGRATION_COUNT:
        violations.append(
            f"frozen merged migration count is {frozen_count}, pinned at "
            f"{PINNED_FROZEN_MERGED_MIGRATION_COUNT} — decrementing is "
            f"always fine (lower the pin), incrementing means a new "
            f"raw-SQL migration landed and needs explicit owner sign-off "
            f"({DOC_POINTER})"
        )

    if violations:
        for v in violations:
            print(v, file=sys.stderr)
        sys.exit(1)

    print("verify_raw_sql_expect_taxonomy: OK")


main()
PYEOF
