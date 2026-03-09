#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
cwd=$(jq -r .cwd <<< "$input")
name=$(jq -r .name <<< "$input")

wt_path="$cwd/.claude/worktrees/$name"

# WorktreeCreate replaces Claude's default git behavior — we own the worktree add.
git -C "$cwd" worktree add "$wt_path" -b "worktree-$name" >&2

# --- Rust --------------------------------------------------------------------

if [[ -f "$wt_path/Cargo.toml" ]]; then
  # Seed build cache (CoW clone via APFS clonefile; silently skipped on other FSes).
  if [[ -d "$cwd/target" ]]; then
    echo ":: Seeding target/ (CoW)..." >&2
    cp -cR "$cwd/target" "$wt_path/" 2>/dev/null \
      || echo ":: warning: CoW clone of target/ failed, skipping" >&2
  fi
fi

# --- Frontend ----------------------------------------------------------------

if [[ -f "$wt_path/frontend/package.json" ]]; then
  # Seed node_modules (CoW clone).
  if [[ -d "$cwd/frontend/node_modules" ]]; then
    echo ":: Seeding frontend/node_modules (CoW)..." >&2
    cp -cR "$cwd/frontend/node_modules" "$wt_path/frontend/" 2>/dev/null \
      || echo ":: warning: CoW clone of frontend/node_modules failed, skipping" >&2
  fi

  # Seed SvelteKit build cache (CoW clone).
  if [[ -d "$cwd/frontend/.svelte-kit" ]]; then
    echo ":: Seeding frontend/.svelte-kit (CoW)..." >&2
    cp -cR "$cwd/frontend/.svelte-kit" "$wt_path/frontend/" 2>/dev/null \
      || echo ":: warning: CoW clone of frontend/.svelte-kit failed, skipping" >&2
  fi

  echo ":: Running npm install..." >&2
  bash -lc "cd '$wt_path/frontend' && npm install" >&2

  echo ":: Building frontend..." >&2
  bash -lc "cd '$wt_path/frontend' && npm run build" >&2
fi

# Required: print the worktree path on stdout so Claude Code knows where it is.
echo "$wt_path"
