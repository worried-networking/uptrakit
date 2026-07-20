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
fail=0

# Slurp-mode (perl -0777, precedent: ci/verify_handler_state_contract.sh)
# handles attributes rustfmt wraps across lines. The ^\s* anchor under /m is
# load-bearing: without it the pattern false-positives on prose mentions of
# the attribute inside // and /// comments.
# Validate every allowlist row up front (mirror verify_no_security_audit.sh's
# strictness: a malformed or unknown-rule row is an error, never silently skipped).
while IFS='|' read -r a_rule a_path a_regex; do
  if [ -z "${a_rule:-}" ] || [ -z "${a_path:-}" ] || [ -z "${a_regex:-}" ]; then
    echo "ERROR: malformed allowlist row (need rule|path|text-regex): $a_rule|$a_path|$a_regex" >&2
    exit 1
  fi
  if [ "$a_rule" != "$RULE" ]; then
    echo "ERROR: unknown rule '$a_rule' in $ALLOWLIST_FILE" >&2
    exit 1
  fi
  case "$a_path" in crates/*) ;; *)
    echo "ERROR: allowlist path must start with crates/: $a_path" >&2
    exit 1
  ;; esac
  # ERE validity: grep exits 1 on valid-but-unmatched, 2 on invalid regex.
  rc=0
  printf '' | grep -Eq "$a_regex" 2>/dev/null || rc=$?
  if [ "$rc" -eq 2 ]; then
    echo "ERROR: invalid ERE in allowlist row for $a_path: $a_regex" >&2
    exit 1
  fi
done < <(grep -v '^#' "$ALLOWLIST_FILE")

while IFS=: read -r file line text; do
  [ -z "${file:-}" ] && continue
  allowed=0
  while IFS='|' read -r a_rule a_path a_regex; do
    [ "$a_path" = "$file" ] || continue
    if printf '%s' "$text" | grep -Eq "$a_regex"; then
      allowed=1
      break
    fi
  done < <(grep -v '^#' "$ALLOWLIST_FILE")
  if [ "$allowed" -eq 0 ]; then
    echo "ERROR: negated-feature cfg attribute not in allowlist: $file:$line: $text" >&2
    echo "  Additive-only feature flags are required; if this site is genuinely" >&2
    echo "  necessary, add a justified allowlist entry (maintainer sign-off)." >&2
    fail=1
  fi
done < <(git ls-files 'crates/*' | grep '\.rs$' | while IFS= read -r f; do
  perl -0777 -ne '
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
    }' "$f"
done)

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "verify_no_new_cfg_not_feature: OK"
