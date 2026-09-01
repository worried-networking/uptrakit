#!/usr/bin/env bash
# bare_crate_sweep.sh: run `cargo clippy --all-targets -p <crate> -- -D warnings` per plugin crate
# in isolation.
#
# A workspace-wide `cargo clippy --all-features` unifies feature flags across every crate, which
# can mask a plugin crate that would fail to compile on its own (e.g. it imports a feature-gated
# item from uptrakit-plugin-infrastructure-core without enabling that feature itself — visible
# only in an isolated `cargo clippy -p <crate>` build; see the proxmox E0308 incident referenced
# by .github/workflows/ci.yml's "Bare-crate clippy sweep (plugin crates)" step). This script is
# the shared implementation behind that CI step (--full) and the pre-push hook's narrower
# diff-scoped check (--scoped), which delegates crate selection to ci/bare_crate_select.py.
#
# Usage:
#   ci/bare_crate_sweep.sh --full
#   ci/bare_crate_sweep.sh --scoped <base> <head>
#
# Exit codes: 0 = swept crates all clean (or nothing selected in --scoped mode), non-zero =
# clippy failure on at least one crate, or a usage/tooling error.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  echo "usage: $(basename "${BASH_SOURCE[0]}") --full" >&2
  echo "       $(basename "${BASH_SOURCE[0]}") --scoped <base> <head>" >&2
}

# Clippy one crate, matching the CI loop's group/echo + invocation verbatim.
sweep_crate() {
  local crate="$1"
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
    echo "::group::${crate}"
  else
    echo "==> ${crate}"
  fi
  cargo clippy --all-targets -p "$crate" -- -D warnings
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
    echo "::endgroup::"
  fi
}

if [[ "${1:-}" == "--full" ]]; then
  if [[ "$#" -ne 1 ]]; then
    usage
    exit 2
  fi
  for manifest in crates/plugins/*/*/Cargo.toml; do
    crate=$(grep -m1 '^name' "$manifest" | sed 's/.*"\(.*\)"/\1/')
    sweep_crate "$crate"
  done
elif [[ "${1:-}" == "--scoped" ]]; then
  if [[ "$#" -ne 3 ]]; then
    usage
    exit 2
  fi
  base="$2"
  head="$3"
  selected="$(python3 ci/bare_crate_select.py "$base" "$head")"
  if [[ -z "$selected" ]]; then
    echo "bare_crate_sweep: no plugin crates affected by ${base}..${head}; skipping sweep"
    exit 0
  fi
  while IFS= read -r crate; do
    [[ -n "$crate" ]] || continue
    sweep_crate "$crate"
  done <<<"$selected"
else
  usage
  exit 2
fi
