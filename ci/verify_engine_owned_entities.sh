#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_engine_owned_entities: required tool 'rg' was not found"
  exit 1
fi

is_comment_only_line() {
  local text="$1" trimmed
  trimmed="${text#"${text%%[![:space:]]*}"}"
  case "$trimmed" in
    '//'* | '/*'* | '*'*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

# The access_grants entity is engine-owned (06-grant-model §Storage schema):
# only the query module and the migration dir may name it. pub(crate)
# visibility already stops other crates; this gate stops future in-crate
# siblings.
PATTERN='entity::access_grant\b|access_grant::(Entity|Model|ActiveModel|Column|Relation|GrantSubjectType)'

# Non-vacuity canary: the sanctioned consumer must still match the pattern —
# if it stops matching, the gate has gone stale (symbol rename), not clean.
if ! rg -q "$PATTERN" crates/shared/db/src/access_grants.rs; then
  echo "verify_engine_owned_entities: pattern no longer matches the sanctioned consumer (crates/shared/db/src/access_grants.rs) — gate is stale"
  exit 1
fi

violations=0

# rg exits 0 on matches, 1 on no matches (legal here), >=2 on error. Capture the
# status via a temp file: a `done < <(rg ...)` process substitution hides it, so
# a broken pattern or unreadable path would report an empty, green run.
RG_TMP="$(mktemp)"
RG_RC=0
rg -n --no-heading "$PATTERN" crates \
  --glob '**/*.rs' \
  --glob '!crates/shared/db/src/access_grants.rs' \
  --glob '!crates/shared/db/src/migration/**' >"$RG_TMP" || RG_RC=$?
if (( RG_RC > 1 )); then
  echo "verify_engine_owned_entities: rg failed (rc=${RG_RC})" >&2
  rm -f "$RG_TMP"
  exit 1
fi

while IFS= read -r line; do
  path="${line%%:*}"
  rest="${line#*:}"
  line_no="${rest%%:*}"
  text="${rest#*:}"
  if is_comment_only_line "$text"; then
    continue
  fi
  if (( violations == 0 )); then
    echo "verify_engine_owned_entities: access_grant entity access outside the engine-owned module:"
  fi
  echo "${path}:${line_no}:${text}"
  violations=$((violations + 1))
done <"$RG_TMP"
rm -f "$RG_TMP"

if (( violations > 0 )); then
  exit 1
fi

echo "verify_engine_owned_entities: OK"
