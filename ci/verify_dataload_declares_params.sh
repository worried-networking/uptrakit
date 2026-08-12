#!/usr/bin/env bash
# verify_dataload_declares_params.sh
#
# Ratchet guard: every in-repo `InteractionKind::DataLoad` registration site
# must either declare the params it accepts via `.with_params(...)` on its
# `InteractionDescriptor`, or be explicitly allowlisted in
# ci/dataload_params_allowlist.txt (one relative path per line, with a
# comment explaining why).
#
# KNOWN LIMITATION (intentional, documented here on purpose): detection is
# FILE-granular, not per-interaction. A file containing ANY `.with_params(`
# call is trusted for EVERY `InteractionKind::DataLoad` site it contains
# (e.g. plugins/infrastructure/proxmox/src/plugin.rs registers close to a
# dozen DataLoad interactions from one file — declaring params on a single
# one would blind this check to the rest). Per-interaction enforcement is a
# deferred hard-block follow-up; this gate only ratchets at file scope.
#
# Every DataLoad interaction registered in the repo today is genuinely
# param-less — reserved-key coercion at the surface-proxy boundary keeps
# them working without declared params (see docs/development/surfaces.md).
# That is why every file currently matched below is seeded onto the
# allowlist instead of gaining a `.with_params(` call: the hard-block policy
# (require every new DataLoad interaction to declare params) is an
# explicitly deferred follow-up, not enforced by this seed commit.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOWLIST_FILE="ci/dataload_params_allowlist.txt"
DATALOAD_PATTERN='InteractionKind::DataLoad'
WITH_PARAMS_PATTERN='\.with_params\('

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_dataload_declares_params: required tool 'rg' was not found"
  exit 1
fi

if [[ ! -f "$ALLOWLIST_FILE" ]]; then
  echo "verify_dataload_declares_params: allowlist file is missing: $ALLOWLIST_FILE"
  exit 1
fi

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

is_well_formed_path() {
  local path="$1"
  case "$path" in
    crates/*.rs)
      [[ "$path" != *[[:space:]]* && "$path" != *'|'* && "$path" != *'*'* ]]
      ;;
    *)
      return 1
      ;;
  esac
}

declare -a ALLOW_PATHS=()

load_allowlist() {
  local line line_no=0 raw path
  while IFS= read -r line || [[ -n "$line" ]]; do
    line_no=$((line_no + 1))
    raw="${line%$'\r'}"

    if [[ -z "$(trim "$raw")" ]]; then
      continue
    fi
    if [[ "$(trim "$raw")" == \#* ]]; then
      continue
    fi

    path="$(trim "$raw")"

    if ! is_well_formed_path "$path"; then
      echo "verify_dataload_declares_params: invalid allowlist row ${line_no}: expected a bare 'crates/...rs' path, got: ${raw}"
      exit 1
    fi

    if [[ ! -f "$path" ]]; then
      echo "verify_dataload_declares_params: allowlisted path does not exist at row ${line_no}: ${path}"
      exit 1
    fi

    if ! rg -q "$DATALOAD_PATTERN" "$path"; then
      echo "verify_dataload_declares_params: stale allowlist row ${line_no} (no longer matches the DataLoad pattern): ${path}"
      exit 1
    fi

    ALLOW_PATHS+=("$path")
  done <"$ALLOWLIST_FILE"
}

is_allowlisted() {
  local path="$1"
  local candidate
  for candidate in "${ALLOW_PATHS[@]}"; do
    [[ "$candidate" == "$path" ]] && return 0
  done
  return 1
}

load_allowlist

# `rg -l` emits one bare file path per match, newline-terminated, with no
# embedded field separators to split on — unlike a `path:line:text` producer,
# a single `while IFS= read` consumer here cannot mis-split a record.
declare -a FINDINGS=()

# rg exits 0 on matches, 1 on no matches (legal here), >=2 on error. Capture the
# status via a temp file: both a `done < <(rg ...)` process substitution and a
# pipe into `sort` hide it, so a broken pattern or unreadable path would report
# an empty, green run.
RG_TMP="$(mktemp)"
RG_RC=0
rg -l "$DATALOAD_PATTERN" crates \
  --glob '**/*.rs' \
  --glob '!crates/shared/surfaces/**' \
  --glob '!crates/ui/surface-proxy/**' \
  --glob '!**/tests/**' \
  --glob '!**/tests.rs' \
  --glob '!**/integration_tests/**' >"$RG_TMP" || RG_RC=$?
if (( RG_RC > 1 )); then
  echo "verify_dataload_declares_params: rg failed (rc=${RG_RC})" >&2
  rm -f "$RG_TMP"
  exit 1
fi

while IFS= read -r line; do
  [[ -n "$line" ]] || continue
  FINDINGS+=("$line")
done < <(sort "$RG_TMP")
rm -f "$RG_TMP"

declare -a VIOLATIONS=()
declare path

for path in "${FINDINGS[@]}"; do
  if rg -q "$WITH_PARAMS_PATTERN" "$path"; then
    continue
  fi
  if is_allowlisted "$path"; then
    continue
  fi
  VIOLATIONS+=("$path")
done

if (( ${#VIOLATIONS[@]} > 0 )); then
  echo "verify_dataload_declares_params: DataLoad interactions without declared params (.with_params(...)) or an allowlist entry:"
  for path in "${VIOLATIONS[@]}"; do
    echo "  ${path}"
  done
  exit 1
fi

echo "verify_dataload_declares_params: OK"
