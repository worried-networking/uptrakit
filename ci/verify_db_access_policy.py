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


def extract_handlers(rs_file: pathlib.Path) -> dict[str, str]:
    """Return {fn_name: full_signature_text} for each async fn in the file."""
    text = rs_file.read_text()
    handlers: dict[str, str] = {}
    for m in re.finditer(r"(?:pub\s+)?async\s+fn\s+(\w+)", text):
        name = m.group(1)
        start = m.start()
        brace = text.find("{", start)
        sig = text[start:brace] if brace != -1 else text[start : start + 500]
        sig = " ".join(sig.split())  # normalize whitespace
        handlers[name] = sig
    return handlers


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
        handlers = extract_handlers(rs_file)
        if not handlers:
            continue

        file_policy = routes_policy.get(rel, {})

        # Check for unlisted handlers (in code but not in policy).
        for fn_name in handlers:
            if fn_name not in file_policy:
                all_errors.append(
                    f"{rel}: handler '{fn_name}' is not listed in db_access_policy.toml"
                )
                exit_code = 1

        # Check for stale entries (in policy but not in code).
        for fn_name in file_policy:
            if fn_name not in handlers:
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
