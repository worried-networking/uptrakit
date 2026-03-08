#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
cwd=$(jq -r .cwd <<< "$input")
wt_path=$(jq -r .worktree_path <<< "$input")

git -C "$cwd" worktree remove "$wt_path" >&2 \
  || echo "warning: could not remove worktree at $wt_path (uncommitted changes?). Remove it manually." >&2
