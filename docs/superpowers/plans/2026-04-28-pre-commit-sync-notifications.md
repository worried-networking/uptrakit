# Pre-commit Sync Notifications + Pre-push Consistency Guard

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Notify developers when `sync-sdk` / `sync-openapi-client` update generated files
during pre-commit, and block pushes when source changes are committed without their
corresponding generated file updates.

**Architecture:** Two targeted bash changes — a post-sync diff check in `.husky/pre-commit`
(warning only, never blocks commit), and a commit-range consistency guard in `.husky/pre-push`
(blocks push when non-merge commits in the push range changed source files but not generated).
Uses `git log --no-merges` to avoid false positives from merge commits pulling in upstream
source changes. No new xtask subcommands.

**Tech Stack:** bash, git

**Known limitation:** `cargo xtask sync-sdk --commit` (suggested in warning/error messages)
calls `git commit` internally without `--no-verify`, which re-triggers pre-commit on the
generated files commit. This is harmless but adds latency. Fix is a separate xtask change
outside this plan's scope.

---

## Files

- Modify: `.husky/pre-commit` — add post-sync diff check after each sync command
- Modify: `.husky/pre-push` — add commit-range source→generated consistency guard

---

### Task 1: Pre-commit post-sync notification

After each `cargo xtask sync-*` call, detect whether the sync changed any generated files
(modified tracked files via `git diff HEAD`, plus new untracked files via `git ls-files
--others`). If any changed, print a warning block listing them and telling the developer
to commit them before pushing.

**Files:**

- Modify: `.husky/pre-commit`

- [ ] **Step 1: Replace the two sync lines at the bottom of `.husky/pre-commit`**

Current (lines 114–118):

```bash
echo '[pre-commit] Regenerating service-sdk generated types...'
cargo xtask sync-sdk

echo '[pre-commit] Regenerating openapi-client generated types...'
cargo xtask sync-openapi-client
```

Replace with:

```bash
echo '[pre-commit] Regenerating service-sdk generated types...'
cargo xtask sync-sdk
_sdk_modified=()
_sdk_new=()
mapfile -t _sdk_modified < <(git diff HEAD --name-only -- crates/shared/service-sdk/src/generated/)
mapfile -t _sdk_new < <(git ls-files --others --exclude-standard -- crates/shared/service-sdk/src/generated/)
SDK_UPDATED=()
if (( ${#_sdk_modified[@]} > 0 )); then SDK_UPDATED+=("${_sdk_modified[@]}"); fi
if (( ${#_sdk_new[@]} > 0 )); then SDK_UPDATED+=("${_sdk_new[@]}"); fi
if (( ${#SDK_UPDATED[@]} > 0 )); then
  echo '[pre-commit] WARNING: sync-sdk updated generated files — commit them in a separate commit before pushing:'
  printf '  %s\n' "${SDK_UPDATED[@]}"
  echo '[pre-commit] Run: cargo xtask sync-sdk --commit'
fi

echo '[pre-commit] Regenerating openapi-client generated types...'
cargo xtask sync-openapi-client
_openapi_modified=()
_openapi_new=()
mapfile -t _openapi_modified < <(git diff HEAD --name-only -- crates/shared/openapi-client/src/generated/)
mapfile -t _openapi_new < <(git ls-files --others --exclude-standard -- crates/shared/openapi-client/src/generated/)
OPENAPI_UPDATED=()
if (( ${#_openapi_modified[@]} > 0 )); then OPENAPI_UPDATED+=("${_openapi_modified[@]}"); fi
if (( ${#_openapi_new[@]} > 0 )); then OPENAPI_UPDATED+=("${_openapi_new[@]}"); fi
if (( ${#OPENAPI_UPDATED[@]} > 0 )); then
  echo '[pre-commit] WARNING: sync-openapi-client updated generated files — commit them in a separate commit before pushing:'
  printf '  %s\n' "${OPENAPI_UPDATED[@]}"
  echo '[pre-commit] Run: cargo xtask sync-openapi-client --commit'
fi
```

> `git diff HEAD --name-only` catches modified tracked files (staged or unstaged).
> `git ls-files --others --exclude-standard` catches new untracked files
> (e.g. when a new source module is added). Together they cover all cases.

- [ ] **Step 2: Verify notification fires when generated files are stale**

```bash
# Simulate stale generated files by deleting one and re-running sync
rm crates/shared/service-sdk/src/generated/mod.rs
cargo xtask sync-sdk
# mod.rs is now regenerated on disk but differs from HEAD

# Check: should list mod.rs
git diff HEAD --name-only -- crates/shared/service-sdk/src/generated/
# Expected: crates/shared/service-sdk/src/generated/mod.rs

# Restore to clean state
git checkout HEAD -- crates/shared/service-sdk/src/generated/
```

- [ ] **Step 3: Verify no warning fires when generated files are already up-to-date**

```bash
# Clean repo: run sync with nothing stale
cargo xtask sync-sdk
git diff HEAD --name-only -- crates/shared/service-sdk/src/generated/
# Expected: empty output
git ls-files --others --exclude-standard -- crates/shared/service-sdk/src/generated/
# Expected: empty output
```

- [ ] **Step 4: Commit**

```bash
git add .husky/pre-commit
git commit -m "feat(hooks): notify developer when pre-commit sync updates generated files"
```

---

### Task 2: Pre-push commit-range consistency guard

Read the refs being pushed from stdin, determine the base SHA for each ref (merge-base
with `<remote>/main` or `<remote>/master` for new branches, otherwise the remote SHA),
collect all files touched in non-merge commits in the pushed range, then block the push
if any source file changed without a corresponding generated file change.

Using `git log --no-merges` (not `git diff`) avoids false positives when a developer
merges upstream into their branch: files changed in the upstream merge commit are excluded
from the file list, so the guard doesn't fire for source changes the developer didn't author.

Source path arrays mirror `xtask/src/sync_sdk.rs` and `xtask/src/sync_openapi_client.rs`
— if a new source directory is added to either xtask command, update these arrays to match.

**Files:**

- Modify: `.husky/pre-push`

- [ ] **Step 1: Add the consistency guard block to `.husky/pre-push`**

Insert immediately after the `set -euo pipefail` line (line 4), before line 6 (`echo "[pre-push] Running cargo fmt check..."`):

```bash
# --- Sync consistency guard ---
# Blocks push when non-merge commits in the range changed source files
# but not their generated counterparts.
# Source path arrays mirror xtask/src/sync_sdk.rs + sync_openapi_client.rs — keep in sync.
_ZERO_SHA="0000000000000000000000000000000000000000"
_remote_name="${1:-origin}"
_ALL_PUSH_FILES=()

while IFS=' ' read -r _local_ref _local_sha _remote_ref _remote_sha; do
  [[ "$_local_sha" == "$_ZERO_SHA" ]] && continue  # deleted ref, skip
  if [[ "$_remote_sha" == "$_ZERO_SHA" ]]; then
    _base=$(git merge-base "$_local_sha" "${_remote_name}/main" 2>/dev/null \
         || git merge-base "$_local_sha" "${_remote_name}/master" 2>/dev/null \
         || echo "$_local_sha")
  else
    _base="$_remote_sha"
  fi
  _loop_files=()
  mapfile -t _loop_files < <(
    git log --no-merges --name-only --format="" "${_base}..${_local_sha}" 2>/dev/null \
    | awk 'NF'
  )
  if (( ${#_loop_files[@]} > 0 )); then
    _ALL_PUSH_FILES+=("${_loop_files[@]}")
  fi
done

_SDK_SOURCES=(
  "crates/shared/surfaces/src/"
  "crates/shared/wire/src/"
  "crates/shared/types/src/"
)
_SDK_GENERATED="crates/shared/service-sdk/src/generated/"

_OPENAPI_SOURCES=(
  "crates/shared/surfaces/src/"
  "crates/shared/wire/src/"
  "crates/shared/types/src/"
  "crates/shared/web-api-types/src/"
)
_OPENAPI_GENERATED="crates/shared/openapi-client/src/generated/"

_sync_guard() {
  local label="$1" generated="$2" fix_cmd="$3"
  shift 3
  local sources=("$@")
  local source_changed=false generated_changed=false f src
  if (( ${#_ALL_PUSH_FILES[@]} > 0 )); then
    for f in "${_ALL_PUSH_FILES[@]}"; do
      for src in "${sources[@]}"; do
        [[ "$f" == "$src"* ]] && source_changed=true
      done
      [[ "$f" == "$generated"* ]] && generated_changed=true
    done
  fi
  if [[ "$source_changed" == true ]] && [[ "$generated_changed" == false ]]; then
    echo "[pre-push] ERROR: $label sources changed but generated files not committed." >&2
    echo "[pre-push] Run: $fix_cmd" >&2
    return 1
  fi
  return 0
}

_guard_failed=false
_sync_guard "sync-sdk"            "$_SDK_GENERATED"    "cargo xtask sync-sdk --commit"            "${_SDK_SOURCES[@]}"    || _guard_failed=true
_sync_guard "sync-openapi-client" "$_OPENAPI_GENERATED" "cargo xtask sync-openapi-client --commit" "${_OPENAPI_SOURCES[@]}" || _guard_failed=true
if [[ "$_guard_failed" == true ]]; then exit 1; fi
# --- End sync consistency guard ---
```

> `stdin` is consumed by the `while read` loop. The pre-push hook does not currently
> read from stdin elsewhere. If stdin reads are added later, they must appear after this block.

- [ ] **Step 2: Verify guard blocks when source changed but generated not committed**

Run on a throwaway branch to avoid polluting history:

```bash
git checkout -b test/sync-guard

# 1. Make a source change, regenerate to disk, but commit ONLY the source
echo '// test' >> crates/shared/wire/src/lib.rs
cargo xtask sync-sdk           # writes to disk, does NOT commit
cargo xtask sync-openapi-client
git add crates/shared/wire/src/lib.rs
git commit --no-verify -m "test: wire source change without generated"

# 2. Set up a local bare remote to push to
git init --bare /tmp/uptrakit-test-remote.git
git remote add test-remote /tmp/uptrakit-test-remote.git

# 3. Push — expected to be blocked
git push test-remote test/sync-guard
# Expected output:
# [pre-push] ERROR: sync-sdk sources changed but generated files not committed.
# [pre-push] Run: cargo xtask sync-sdk --commit
# [pre-push] ERROR: sync-openapi-client sources changed but generated files not committed.
# [pre-push] Run: cargo xtask sync-openapi-client --commit

# 4. Clean up — restore dirty files BEFORE switching branches (git checkout
#    will refuse to switch if the modified files conflict with main's state)
git remote remove test-remote
rm -rf /tmp/uptrakit-test-remote.git
git checkout HEAD -- crates/shared/service-sdk/src/generated/ crates/shared/openapi-client/src/generated/
git checkout HEAD -- crates/shared/wire/src/lib.rs
git checkout main
git branch -D test/sync-guard
```

- [ ] **Step 3: Verify guard passes when both source and generated are committed**

```bash
git checkout -b test/sync-guard-pass

# 1. Make a source change, commit source + generated together
echo '// test' >> crates/shared/wire/src/lib.rs
cargo xtask sync-sdk --commit          # commits generated automatically
cargo xtask sync-openapi-client --commit
git add crates/shared/wire/src/lib.rs
git commit --no-verify -m "test: wire change with generated committed"

# 2. Set up local bare remote
git init --bare /tmp/uptrakit-test-remote.git
git remote add test-remote /tmp/uptrakit-test-remote.git

# 3. Push — guard should pass (no ERROR lines from sync guard)
git push test-remote test/sync-guard-pass

# 4. Clean up (working tree is clean — all changes were committed to the test branch)
git remote remove test-remote
rm -rf /tmp/uptrakit-test-remote.git
git checkout main
git branch -D test/sync-guard-pass
# wire/src/lib.rs reverts automatically when switching back to main
```

- [ ] **Step 4: Verify guard passes when no source files changed**

```bash
git checkout -b test/sync-guard-unrelated

echo 'test' >> README.md
git add README.md
git commit --no-verify -m "test: unrelated change"

git init --bare /tmp/uptrakit-test-remote.git
git remote add test-remote /tmp/uptrakit-test-remote.git
git push test-remote test/sync-guard-unrelated
# Expected: no [pre-push] ERROR lines from sync guard

git remote remove test-remote
rm -rf /tmp/uptrakit-test-remote.git
git checkout main
git branch -D test/sync-guard-unrelated
git checkout HEAD -- README.md
```

- [ ] **Step 5: Commit**

```bash
git add .husky/pre-push
git commit -m "feat(hooks): block push when sync sources changed but generated files not committed"
```
