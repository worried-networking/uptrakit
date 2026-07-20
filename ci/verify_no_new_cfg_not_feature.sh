#!/usr/bin/env bash
# Gate: no NEW negated-feature cfg attributes. Enforces the additive-only
# feature-flag invariant (docs/development/coding-standards.md#feature-flags).
# Catches all in-tree spellings: #[cfg(not(feature = ...))],
# #[cfg(not(any(feature = ...)))], #[cfg(all(..., not(feature = ...)))], and
# the inner-attribute form #![cfg(...)]. The allowlist is a shrink-only
# ratchet grandfathering pre-existing sites; adding an entry requires
# maintainer sign-off (see docs/superpowers/specs/
# 2026-07-20-proxmox-bare-crate-gates-design.md, Gate B).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOWLIST_FILE="ci/verify_no_new_cfg_not_feature_allowlist.txt"
RULE="negated_feature_cfg"

if [[ ! -f "$ALLOWLIST_FILE" ]]; then
  echo "verify_no_new_cfg_not_feature: allowlist file is missing: $ALLOWLIST_FILE"
  exit 1
fi

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

# ERE validity: grep exits 1 on valid-but-unmatched, 2 on invalid regex.
is_valid_ere() {
  local pattern="$1"
  printf '' | grep -E "$pattern" >/dev/null 2>&1 || [[ $? -eq 1 ]]
}

declare -a ALLOW_PATHS=()
declare -a ALLOW_TEXT_PATTERNS=()

# Validate + load every allowlist row up front (mirror verify_no_security_audit.sh:
# blank and comment lines are skipped for editability; a malformed, unknown-rule,
# bad-path, or invalid-regex row is a fatal error reported with its line number,
# never silently skipped).
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
      echo "verify_no_new_cfg_not_feature: invalid allowlist row ${line_no}: expected 'rule|path|text-regex'"
      exit 1
    fi
    rule="${raw%%|*}"
    rest="${raw#*|}"

    if [[ "$rest" != *"|"* ]]; then
      echo "verify_no_new_cfg_not_feature: invalid allowlist row ${line_no}: expected 'rule|path|text-regex'"
      exit 1
    fi
    path="${rest%%|*}"
    text_pattern="${rest#*|}"

    rule="$(trim "$rule")"
    path="$(trim "$path")"
    text_pattern="$(trim "$text_pattern")"

    if [[ -z "$rule" || -z "$path" || -z "$text_pattern" ]]; then
      echo "verify_no_new_cfg_not_feature: invalid allowlist row ${line_no}: expected 'rule|path|text-regex'"
      exit 1
    fi

    if [[ "$rule" != "$RULE" ]]; then
      echo "verify_no_new_cfg_not_feature: invalid allowlist rule '${rule}' at row ${line_no}"
      exit 1
    fi

    if [[ "$path" != crates/* ]]; then
      echo "verify_no_new_cfg_not_feature: allowlist path must start with 'crates/' at row ${line_no}"
      exit 1
    fi

    if ! is_valid_ere "$text_pattern"; then
      echo "verify_no_new_cfg_not_feature: invalid regex in allowlist row ${line_no}: ${text_pattern}"
      exit 1
    fi

    ALLOW_PATHS+=("$path")
    ALLOW_TEXT_PATTERNS+=("$text_pattern")
  done <"$ALLOWLIST_FILE"
}

# A finding is allowlisted when a row's path matches exactly AND its text-regex
# pins the specific feature name in the flagged attribute.
is_allowlisted() {
  local path="$1"
  local text="$2"
  local idx pattern

  for idx in "${!ALLOW_PATHS[@]}"; do
    [[ "${ALLOW_PATHS[$idx]}" == "$path" ]] || continue
    pattern="${ALLOW_TEXT_PATTERNS[$idx]}"
    if printf '%s\n' "$text" | grep -Eq "$pattern"; then
      return 0
    fi
  done
  return 1
}

load_allowlist

# Collect the tracked .rs file list once, then hand it to a SINGLE perl process
# (precedent: ci/verify_handler_state_contract.sh batches files into one
# `perl -0777` invocation rather than forking per file).
rs_files=()
while IFS= read -r f; do
  rs_files+=("$f")
done < <(git ls-files 'crates/*' | grep '\.rs$')

# Slurp-mode (perl -0777) handles attributes rustfmt wraps across lines. The
# ^\s* anchor is load-bearing: without it the pattern false-positives on prose
# mentions of the attribute inside // and /// comments.
fail=0
if [ "${#rs_files[@]}" -gt 0 ]; then
  while IFS=: read -r file line text; do
    [ -z "${file:-}" ] && continue
    if is_allowlisted "$file" "$text"; then
      continue
    fi
    echo "ERROR: negated-feature cfg attribute not in allowlist: $file:$line: $text" >&2
    echo "  Additive-only feature flags are required; if this site is genuinely" >&2
    echo "  necessary, add a justified allowlist entry (maintainer sign-off)." >&2
    fail=1
  done < <(perl -0777 -ne '
      while (/^([^\S\n]*#!?\[cfg\([^\]]*not\s*\(\s*(?:any\s*\(\s*)?feature[^\n]*)/gsm) {
        my $off  = $-[1];
        my $line = 1 + substr($_, 0, $off) =~ tr/\n//;
        my $text = $1;
        $text =~ s/^\s+//;
        # Join wrapped attributes into ONE record: an embedded newline would
        # split the file:line:text stream at the IFS=: read consumer, making
        # wrapped sites unallowlistable (verified failure mode).
        $text =~ s/\s*\n\s*/ /g;
        print "$ARGV:$line:$text\n";
      }' "${rs_files[@]}")
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "verify_no_new_cfg_not_feature: OK"
