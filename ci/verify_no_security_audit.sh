#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOWLIST_FILE="ci/verify_no_security_audit_allowlist.txt"
SEP=$'\x1f'

if ! command -v rg >/dev/null 2>&1; then
  echo "verify_no_security_audit: required tool 'rg' was not found"
  exit 1
fi

if [[ ! -f "$ALLOWLIST_FILE" ]]; then
  echo "verify_no_security_audit: allowlist file is missing: $ALLOWLIST_FILE"
  exit 1
fi

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

is_valid_ere() {
  local pattern="$1"
  printf '' | grep -E "$pattern" >/dev/null 2>&1 || [[ $? -eq 1 ]]
}

is_comment_only_line() {
  local text="$1"
  local trimmed
  trimmed="$(trim "$text")"
  case "$trimmed" in
    '//'* | '/*'* | '*'*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

declare -a ALLOW_RULES=()
declare -a ALLOW_PATHS=()
declare -a ALLOW_TEXT_PATTERNS=()

load_allowlist() {
  local line line_no=0 raw rule path text_pattern rest
  while IFS= read -r line || [[ -n "$line" ]]; do
    line_no=$((line_no + 1))
    raw="${line%$'\r'}"

    if [[ -z "$(trim "$raw")" ]]; then
      continue
    fi
    if [[ "$(trim "$raw")" == \#* ]]; then
      continue
    fi

    if [[ "$raw" != *"|"* ]]; then
      echo "verify_no_security_audit: invalid allowlist row ${line_no}: expected 'rule|path|text-regex'"
      exit 1
    fi
    rule="${raw%%|*}"
    rest="${raw#*|}"

    if [[ "$rest" != *"|"* ]]; then
      echo "verify_no_security_audit: invalid allowlist row ${line_no}: expected 'rule|path|text-regex'"
      exit 1
    fi
    path="${rest%%|*}"
    text_pattern="${rest#*|}"

    rule="$(trim "$rule")"
    path="$(trim "$path")"
    text_pattern="$(trim "$text_pattern")"

    if [[ -z "$rule" || -z "$path" || -z "$text_pattern" ]]; then
      echo "verify_no_security_audit: invalid allowlist row ${line_no}: expected 'rule|path|text-regex'"
      exit 1
    fi

    if [[ "$rule" != "security_audit" && "$rule" != "raw_action" ]]; then
      echo "verify_no_security_audit: invalid allowlist rule '${rule}' at row ${line_no}"
      exit 1
    fi

    if [[ "$path" != crates/* ]]; then
      echo "verify_no_security_audit: allowlist path must start with 'crates/' at row ${line_no}"
      exit 1
    fi

    if ! is_valid_ere "$text_pattern"; then
      echo "verify_no_security_audit: invalid regex in allowlist row ${line_no}: ${text_pattern}"
      exit 1
    fi

    ALLOW_RULES+=("$rule")
    ALLOW_PATHS+=("$path")
    ALLOW_TEXT_PATTERNS+=("$text_pattern")
  done <"$ALLOWLIST_FILE"
}

is_allowlisted() {
  local rule="$1"
  local path="$2"
  local text="$3"
  local idx pattern

  for idx in "${!ALLOW_RULES[@]}"; do
    [[ "${ALLOW_RULES[$idx]}" == "$rule" ]] || continue
    [[ "${ALLOW_PATHS[$idx]}" == "$path" ]] || continue

    pattern="${ALLOW_TEXT_PATTERNS[$idx]}"
    if printf '%s\n' "$text" | grep -Eq "$pattern"; then
      return 0
    fi
  done

  return 1
}

declare -a FINDINGS=()

collect_findings() {
  local rule="$1"
  local pattern="$2"
  shift 2
  local line path rest line_no text

  while IFS= read -r line; do
    path="${line%%:*}"
    rest="${line#*:}"
    line_no="${rest%%:*}"
    text="${rest#*:}"

    if is_comment_only_line "$text"; then
      continue
    fi

    FINDINGS+=("${rule}${SEP}${path}${SEP}${line_no}${SEP}${text}")
  done < <(rg -n --no-heading "$pattern" "$@" 2>/dev/null || true)
}

load_allowlist

collect_findings "security_audit" 'target:\s*"security_audit"' \
  crates \
  --glob '**/*.rs' \
  --glob '!**/migration/**'

collect_findings "raw_action" 'AuditActionType::(new|from_static)\("|AuditEntry::builder\("|action_type:\s*"' \
  crates \
  --glob '**/*.rs' \
  --glob '!**/migration/**' \
  --glob '!**/fixtures/**' \
  --glob '!**/docs/**'

declare -a VIOLATIONS=()
declare -A COUNTS=(
  ["security_audit"]=0
  ["raw_action"]=0
)
declare entry rule path line_no text

for entry in "${FINDINGS[@]}"; do
  IFS="$SEP" read -r rule path line_no text <<<"$entry"

  if is_allowlisted "$rule" "$path" "$text"; then
    continue
  fi

  COUNTS["$rule"]=$((COUNTS["$rule"] + 1))
  VIOLATIONS+=("$entry")
done

if (( ${#VIOLATIONS[@]} > 0 )); then
  if (( COUNTS["security_audit"] > 0 )); then
    echo "verify_no_security_audit: legacy security_audit callsites remain:"
    for entry in "${VIOLATIONS[@]}"; do
      IFS="$SEP" read -r rule path line_no text <<<"$entry"
      [[ "$rule" == "security_audit" ]] || continue
      echo "${path}:${line_no}:${text}"
    done
  fi

  if (( COUNTS["raw_action"] > 0 )); then
    if (( COUNTS["security_audit"] > 0 )); then
      echo
    fi
    echo "verify_no_security_audit: raw semantic action strings remain outside allowlist:"
    for entry in "${VIOLATIONS[@]}"; do
      IFS="$SEP" read -r rule path line_no text <<<"$entry"
      [[ "$rule" == "raw_action" ]] || continue
      echo "${path}:${line_no}:${text}"
    done
  fi

  exit 1
fi

echo "verify_no_security_audit: OK"
