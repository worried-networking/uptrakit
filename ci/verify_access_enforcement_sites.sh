#!/usr/bin/env bash
# Access-enforcement site inventory gate (M2.1 spec §3).
#
# Every PRODUCTION `.authorize(` call site (the coarse, targetless gate)
# must appear in ci/verify_access_enforcement_sites_inventory.txt with a
# per-file count and a classification. Series invariant: while any site is
# still `needs-fine-check`, the SelectorPhaseGate write gate must still
# exist in access_grants.rs — M2.3 may only lift the gate in the same
# change that routes every needs-fine-check site through authorize_target
# (its row then flips to `fine-checked`, positively verified below).
# The three M2.3 target files are additionally PINNED: while such a file
# exists it must carry a needs-fine-check/fine-checked row, so a row
# cannot be silently deleted to satisfy the absence-shaped checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_access_enforcement_sites: rg not found" >&2
  exit 1
fi

# `|| true`: rg exits 1 on zero matches; under pipefail that would fail the
# script, but zero production sites is a state the checker must judge.
# rg output goes through a temp file, NOT a pipe: `python3 -` reads its
# PROGRAM from stdin (the heredoc), so piped data on fd 0 would be lost.
hits="$(mktemp)"
trap 'rm -f "$hits"' EXIT
{ rg -n --type rust '\.authorize\(' "$ROOT/crates" || true; } > "$hits"
python3 - "$ROOT" "$hits" <<'PYEOF'
import re
import sys
from collections import Counter
from pathlib import Path

root = Path(sys.argv[1])
hits_path = Path(sys.argv[2])
inventory_path = root / "ci" / "verify_access_enforcement_sites_inventory.txt"

# First `#[cfg(...test...)]` line per file = production/test boundary.
# Token-boundary "test" detection per spec §3: matches `#[cfg(test)]` and
# nested forms (`#[cfg(all(test, feature = "..."))]`, `#[cfg(any(..., test))]`)
# where `test` is a bare token delimited by `(`, `,`, `)` or whitespace —
# quoted feature names containing "test" (e.g. "test-utils") never match.
TEST_BOUNDARY = re.compile(r"#\[cfg\((?:.*[(,\s])?test[\s,)]")
boundaries = {}


def boundary(path):
    if path not in boundaries:
        line_no = None
        with open(path, encoding="utf-8") as fh:
            for i, line in enumerate(fh, 1):
                if TEST_BOUNDARY.search(line):
                    line_no = i
                    break
        boundaries[path] = line_no
    return boundaries[path]


counts = Counter()
all_hit_files = set()
for raw in hits_path.read_text(encoding="utf-8").splitlines():
    if not raw:
        continue
    path, line_no, rest = raw.split(":", 2)
    # same comment skip as the fine-checked scan below: a rustdoc line
    # mentioning `.authorize(` must not inflate a production count
    if rest.lstrip().startswith("//"):
        continue
    rel = str(Path(path).relative_to(root))
    all_hit_files.add(rel)
    b = boundary(path)
    if b is not None and int(line_no) >= b:
        continue  # test code
    counts[rel] += 1

rows = {}
failures = []
for raw in inventory_path.read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [p.strip() for p in line.split("|")]
    if len(parts) != 3 or parts[2] not in {
        "coarse-only",
        "needs-fine-check",
        "fine-checked",
        "test-only",
    }:
        failures.append(f"malformed inventory row: {raw}")
        continue
    rows[parts[0]] = (int(parts[1]), parts[2])

for path, count in sorted(counts.items()):
    if path not in rows:
        failures.append(f"unlisted production authorize() site file: {path} ({count} sites)")
    elif rows[path][0] != count:
        failures.append(f"count drift for {path}: inventory says {rows[path][0]}, found {count}")
for path in sorted(all_hit_files):
    if counts.get(path, 0) == 0 and path not in rows:
        b = boundary(root / path)
        failures.append(
            f"{path}: no production .authorize( sites found (all hits are "
            f"after the first #[cfg(...test...)] at line {b}) — verify that "
            "boundary is correct MANUALLY before adding a |0|test-only row; "
            "an early #[cfg(test)] item can hide real production sites"
        )
for path, (count, cls) in sorted(rows.items()):
    if cls == "test-only":
        # test-only rows document files whose .authorize( hits are all in
        # test code — required so an early `#[cfg(test)]` item can never
        # silently zero a file's production sites (the row forces a look).
        if counts.get(path, 0) != 0 or count != 0:
            failures.append(f"test-only row but production sites found: {path}")
        elif path not in all_hit_files:
            failures.append(f"stale test-only row (no authorize hits at all): {path}")
        continue
    # fine-checked requires ZERO remaining coarse calls: partial routing
    # (route 1 of 3, flip the row) must not satisfy the series invariant.
    # A file that must deliberately keep a coarse site after fine-checking
    # cannot be expressed here — edit this gate in a reviewed change.
    if cls == "fine-checked" and count != 0:
        failures.append(f"fine-checked row must have count 0 (found {count}): {path}")
    if path not in counts and not (cls == "fine-checked" and count == 0):
        failures.append(f"stale inventory row (no production sites found): {path}")

# fine-checked is a POSITIVE claim: the file must contain at least one
# production `authorize_target(` call. Absence of coarse calls alone
# proves nothing (a deleted or renamed call site would satisfy it).
for path, (_count, cls) in sorted(rows.items()):
    if cls != "fine-checked":
        continue
    file_path = root / path
    if not file_path.is_file():
        failures.append(f"fine-checked row for missing file: {path}")
        continue
    b = boundary(file_path)
    has_fine = False
    with open(file_path, encoding="utf-8") as fh:
        for i, line in enumerate(fh, 1):
            if b is not None and i >= b:
                break
            # skip comment lines: a rustdoc mention of authorize_target
            # must not satisfy the positive routing claim
            if line.lstrip().startswith("//"):
                continue
            if ".authorize_target(" in line:
                has_fine = True
                break
    if not has_fine:
        failures.append(
            f"fine-checked row without a production authorize_target( call: {path}"
        )

# M2.3 target files (spec §3/§5) are PINNED: while the file exists it must
# carry a needs-fine-check or fine-checked row — blocks silent row deletion.
PINNED = [
    "crates/ui/mcp/src/oauth/tool_auth.rs",
    "crates/ui/web-api/src/middleware/action.rs",
    "crates/ui/web-api/src/routes/surfaces.rs",
]
for pinned in PINNED:
    if not (root / pinned).is_file():
        continue
    if rows.get(pinned, (0, ""))[1] not in {"needs-fine-check", "fine-checked"}:
        failures.append(
            f"pinned M2.3 target file lacks a needs-fine-check/fine-checked row: {pinned}"
        )

needs_fine = sorted(p for p, (_c, cls) in rows.items() if cls == "needs-fine-check")
if needs_fine:
    gate_src = (root / "crates/shared/db/src/access_grants.rs").read_text(encoding="utf-8")
    # Gate presence = the actual bail site AND its pinning test, not a bare
    # substring (a leftover comment or error-variant name would false-satisfy it).
    gate_present = re.search(
        r"bail!\(\s*AccessGrantError::SelectorPhaseGate", gate_src
    ) and ("b9_valid_non_all_selectors_hit_phase_gate_last" in gate_src)
    if not gate_present:
        failures.append(
            "series invariant violated: needs-fine-check sites remain "
            f"({', '.join(needs_fine)}) but the SelectorPhaseGate write gate "
            "(bail! site + b9_valid_non_all_selectors_hit_phase_gate_last test) "
            "is gone — M2.3 may only lift the gate once every such site is fine-checked"
        )

if failures:
    for failure in failures:
        print(f"verify_access_enforcement_sites: {failure}", file=sys.stderr)
    sys.exit(1)

print("verify_access_enforcement_sites: OK")
PYEOF
