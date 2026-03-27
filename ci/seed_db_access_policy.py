#!/usr/bin/env python3
"""Generate initial db_access_policy.toml by scanning routes/ and auto-classifying.

Classification logic (applied in order):
  1. Not `pub` or `pub(super)` visibility → 'ignore' (private helper)
  2. Has State<Arc<AppState>> in signature → 'full-state'
  3. Has TenantDb in signature → 'tenant-scoped'
  4. Has State<DbState> in signature → 'tenant-agnostic'
  5. Otherwise → 'no-db'

Run once from the repository root:
    python3 ci/seed_db_access_policy.py

Review the output and manually correct any misclassified entries.
"""
import re
import pathlib

ROUTES_DIR = pathlib.Path("crates/ui/web-api/src/routes")
OUTPUT = pathlib.Path("crates/ui/web-api/db_access_policy.toml")

HEADER = """# Database access policy for route handlers in crates/ui/web-api/src/routes/.
#
# Classifications:
#   tenant-agnostic  — handler uses State<DbState>, never TenantDb or State<Arc<AppState>>
#   tenant-scoped    — handler uses TenantDb, never State<DbState> or State<Arc<AppState>>
#   no-db            — handler accesses no database at all
#   full-state       — handler uses State<Arc<AppState>> (uncovered fields; migration pending)
#   ignore           — non-handler helper function; skipped by verify_db_access_policy.py
#
# Validated by ci/verify_db_access_policy.py on every CI run.
# Updated atomically with handler signature changes during migration.
"""


def classify_fn(name: str, full_text_before_brace: str) -> str:
    """Classify a single async fn by its declaration and signature."""
    # Extract the visibility prefix (everything before 'async fn name')
    vis_match = re.match(
        r"^\s*(pub(?:\s*\([^)]*\))?\s+)?async\s+fn\s+\w+",
        full_text_before_brace,
    )
    if vis_match:
        vis = (vis_match.group(1) or "").strip()
    else:
        vis = ""

    # Private (no pub) or pub(crate) with no Axum-style params → ignore
    # pub(crate) helpers that take non-Axum params (no State<>, TenantDb, etc.)
    is_public = vis == "pub" or vis.startswith("pub(super)")
    is_pub_crate = vis == "pub(crate)"

    sig = " ".join(full_text_before_brace.split())

    has_full_state = "State<Arc<AppState>>" in sig
    has_tenant_db = "TenantDb" in sig
    has_db_state = "State<DbState>" in sig
    has_axum_params = any(
        p in sig
        for p in [
            "State<",
            "TenantDb",
            "Query<",
            "Json<",
            "Path<",
            "Extension<",
            "CanView",
            "CanManage",
            "CanCreate",
            "CanDelete",
            "CanUpdate",
            "Authenticated",
            "AuthenticatedUser",
            "Request,",
            "Next,",
            "Request<",
        ]
    )

    if not is_public and not is_pub_crate:
        return "ignore"

    if is_pub_crate and not has_axum_params:
        return "ignore"

    if has_full_state:
        return "full-state"
    if has_tenant_db:
        return "tenant-scoped"
    if has_db_state:
        return "tenant-agnostic"
    return "no-db"


def extract_fns(rs_file: pathlib.Path) -> list[tuple[str, str]]:
    """Return [(fn_name, classification)] for each async fn."""
    text = rs_file.read_text()
    results = []
    for m in re.finditer(r"(?:pub(?:\s*\([^)]*\))?\s+)?async\s+fn\s+(\w+)", text):
        name = m.group(1)
        start = m.start()
        brace = text.find("{", start)
        declaration = text[start:brace] if brace != -1 else text[start : start + 500]
        classification = classify_fn(name, declaration)
        results.append((name, classification))
    return results


sections = []
total_handlers = 0
for rs_file in sorted(ROUTES_DIR.rglob("*.rs")):
    rel = rs_file.relative_to(ROUTES_DIR)
    fns = extract_fns(rs_file)
    if not fns:
        continue
    lines = [f'[routes."{rel}"]']
    for fn_name, classification in fns:
        lines.append(f'{fn_name} = "{classification}"')
        if classification != "ignore":
            total_handlers += 1
    sections.append("\n".join(lines))

OUTPUT.write_text(HEADER + "\n" + "\n\n".join(sections) + "\n")
total_fns = sum(len(extract_fns(pathlib.Path("crates/ui/web-api/src/routes").joinpath(
    str(pathlib.Path(s.split("\n")[0].split('"')[1]))
))) for s in sections)
print(
    f"Wrote {OUTPUT} with ~{total_handlers} non-ignored handlers "
    f"across {len(sections)} route files."
)
print("Review the output and manually correct any misclassified entries.")
