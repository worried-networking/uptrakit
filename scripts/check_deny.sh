#!/usr/bin/env bash
# check_deny.sh [BASE_REF]
#
# Without BASE_REF: full cargo deny check (warnings treated as errors).
# With BASE_REF:    diff-aware check — fails only on error/warning violations
#                   not present on BASE_REF. Requires jq.
set -euo pipefail

BASE_REF="${1:-}"

if [ -z "$BASE_REF" ]; then
    echo "[deny] Running full cargo deny check..."
    if deny_output=$(cargo deny --color=never check 2>&1); then
        printf '%s\n' "$deny_output"
        if printf '%s\n' "$deny_output" | grep -qE '^warning\['; then
            echo "[deny] Warnings treated as errors. Fix or document in deny.toml."
            exit 1
        fi
    else
        printf '%s\n' "$deny_output"
        exit 1
    fi
    echo "[deny] OK"
    exit 0
fi

# Diff-aware mode
if ! command -v jq > /dev/null 2>&1; then
    echo "[deny] jq required for diff-aware check. Install jq." >&2
    exit 1
fi

echo "[deny] Running diff-aware check (base: $BASE_REF)..."

DENY_HEAD_TMP=$(mktemp)
DENY_BASE_TMP=$(mktemp)
WORKTREE=$(mktemp -d)
trap 'rm -f "$DENY_HEAD_TMP" "$DENY_BASE_TMP"; git worktree remove --force "$WORKTREE" 2>/dev/null; rm -rf "$WORKTREE"' EXIT

fingerprints() {
    jq -r 'select(.type == "diagnostic"
                and (.fields.severity == "error" or .fields.severity == "warning"))
        | [.fields.severity,
           (.fields.advisory.id // .fields.code // "unknown"),
           (.fields.graphs[0].Krate.name // "unknown"),
           (.fields.graphs[0].Krate.version // "unknown")]
        | join("\t")' "$1" | LC_ALL=C sort -u
}

sanity_check() {
    if ! grep -q '"type":"summary"' "$1"; then
        echo "[deny] cargo deny produced no output for $2 — network error or mis-install?" >&2
        exit 1
    fi
}

# HEAD
# Unset git hook env vars so git-fetch-with-cli child process doesn't inherit GIT_DIR
# and accidentally fetch into the project repo instead of ~/.cargo/advisory-dbs.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE 2>/dev/null || true
cargo deny -f json check > /dev/null 2> "$DENY_HEAD_TMP" || true
sanity_check "$DENY_HEAD_TMP" "HEAD"

# Base
git worktree add "$WORKTREE" "$BASE_REF" --detach -q
cp deny.toml "$WORKTREE/deny.toml"
# --disable-fetch: advisory DB was just fetched by the HEAD check above.
(cd "$WORKTREE" && cargo deny -f json check --disable-fetch > /dev/null 2> "$DENY_BASE_TMP") || true
sanity_check "$DENY_BASE_TMP" "$BASE_REF"

NEW=$(comm -23 <(fingerprints "$DENY_HEAD_TMP") <(fingerprints "$DENY_BASE_TMP"))

if [ -n "$NEW" ]; then
    echo "[deny] New violations introduced relative to $BASE_REF:"
    echo ""
    while IFS=$'\t' read -r severity code kname kver; do
        msg=$(jq -r --arg s "$severity" --arg c "$code" --arg n "$kname" --arg v "$kver" \
            'select(.type == "diagnostic"
                and .fields.severity == $s
                and ((.fields.advisory.id // .fields.code // "unknown") == $c)
                and ((.fields.graphs[0].Krate.name // "unknown") == $n)
                and ((.fields.graphs[0].Krate.version // "unknown") == $v))
            | .fields.message' "$DENY_HEAD_TMP" | head -1)
        echo "  [$severity] $code — $kname $kver"
        [ -n "$msg" ] && echo "    $msg"
    done <<< "$NEW"
    echo ""
    echo "Run 'cargo deny check' for full details."
    exit 1
fi

echo "[deny] OK (no new violations relative to $BASE_REF)"
