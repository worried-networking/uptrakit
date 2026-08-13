#!/usr/bin/env bash
# Rewrite each newly-created GitHub release's body from the raw
# git-cliff commit dump release-plz produces into an LLM-polished
# executive summary + themed highlights, linking back to the package's
# CHANGELOG.md for the full commit-level history.
#
# Inputs (env):
#   RELEASES                 — JSON array of {package_name,tag,version}
#                               (required; from release-plz's `releases`
#                               job output — includes git_only packages
#                               that got a tag but no GitHub release page)
#   GH_TOKEN                 — token for `gh release view`/`gh release
#                               edit` (required; never passed to the
#                               opencode agent — see step 4)
#   GEMINI_API_KEY            — model API key (required in CI; consumed by
#                               opencode via the GOOGLE_GENERATIVE_AI_API_KEY
#                               export below, not read directly here. A
#                               local run may omit it and rely on
#                               `opencode auth login` instead)
#   MODEL                     — opencode model id, e.g.
#                               google/gemini-3.6-flash (required)
#   POLISH_NOTES_SKIP_PUBLISH — if "1", skip `gh release edit` (dry run);
#                               everything else (generation, validation)
#                               runs identically (optional)
#   RUNNER_TEMP               — scratch dir for raw/notes files; falls
#                               back to `mktemp -d` for local runs
#
# Exit codes:
#   0 — every release processed cleanly (including "nothing to do")
#   1 — at least one release failed filtering/generation/validation/
#       publication; each failure is reported as a distinct ::error::
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
readonly SCRIPT_DIR
readonly PROMPT_FILE="$SCRIPT_DIR/polish-notes-prompt.md"

# `:?` not `?`: the plain form only rejects an *unset* variable, so an
# empty secret or a workflow typo would sail past and fail per-release
# much later.
: "${RELEASES:?RELEASES must be set}"
: "${GH_TOKEN:?GH_TOKEN must be set}"
: "${MODEL:?MODEL must be set}"
: "${POLISH_NOTES_SKIP_PUBLISH:=}"
: "${RUNNER_TEMP:=$(mktemp -d)}"

# Credential. CI must pass it in the environment; a local run may instead
# rely on `opencode auth login`, which wins over both env names (opencode
# loads auth.json after env and passes options.apiKey explicitly).
: "${GEMINI_API_KEY:=}"
if [ -n "${GITHUB_ACTIONS:-}" ] && [ -z "$GEMINI_API_KEY" ]; then
  echo "::error::polish-notes: GEMINI_API_KEY must be set in CI" >&2
  exit 1
fi
# opencode only injects an apiKey for providers declaring exactly one env
# name (`key: provider.env.length === 1 ? apiKey : undefined`). models.dev
# lists three for google, so opencode enables the provider but leaves
# authentication to @ai-sdk/google, which reads GOOGLE_GENERATIVE_AI_API_KEY
# and nothing else — GEMINI_API_KEY alone therefore authenticates nothing.
# Bridge it. An already-set value wins, so swapping POLISH_NOTES_MODEL to
# another provider can supply its own credential.
if [ -n "$GEMINI_API_KEY" ]; then
  export GOOGLE_GENERATIVE_AI_API_KEY="${GOOGLE_GENERATIVE_AI_API_KEY:-$GEMINI_API_KEY}"
fi

if [ ! -f "$PROMPT_FILE" ]; then
  echo "::error::polish-notes: missing prompt file $PROMPT_FILE" >&2
  exit 1
fi

# Every fallible per-release call is guarded so one bad release records
# and continues instead of `set -e` aborting the whole loop.
FAILURES=()
record_failure() {
  FAILURES+=("$1")
  echo "::error::$1" >&2
}

success_count=0

# --- Step 1 FILTER data + Step 3 SCOPE data, in one pass -----------------
# Reads release-plz.toml (git_release_enable / changelog_include) and
# joins it with `cargo metadata` for the authoritative crate-name→dir
# map. Computed once, up front, for every [[package]] declared in
# release-plz.toml (independent of RELEASES) — that's every package that
# could possibly have git_release_enable=true, since anything not
# declared inherits the workspace default. Emits
# {package: {git_release_enable, dirs: [...], changelog: path|null}}.
SCOPE_JSON=$(python3 - <<'PY'
import json
import subprocess
import sys
import tomllib
from pathlib import Path


def fail(msg: str) -> None:
    print(f"::error::polish-notes: scope resolution: {msg}", file=sys.stderr)
    sys.exit(1)


try:
    metadata_raw = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
except (OSError, subprocess.CalledProcessError) as exc:
    fail(f"cargo metadata failed: {exc}")

metadata = json.loads(metadata_raw)
workspace_root = Path(metadata["workspace_root"])

name_to_dir: dict[str, str] = {}
for pkg in metadata["packages"]:
    manifest_dir = Path(pkg["manifest_path"]).parent
    try:
        rel = manifest_dir.relative_to(workspace_root)
    except ValueError:
        # Not part of this workspace tree; cannot happen for --no-deps
        # output but skip defensively rather than crash.
        continue
    name_to_dir[pkg["name"]] = rel.as_posix() if str(rel) != "." else "."

try:
    with open("release-plz.toml", "rb") as fh:
        toml_data = tomllib.load(fh)
except OSError as exc:
    fail(f"cannot read release-plz.toml: {exc}")

workspace_cfg = toml_data.get("workspace", {})
ws_git_release_enable = bool(workspace_cfg.get("git_release_enable", False))

result: dict[str, dict] = {}
for pkg_cfg in toml_data.get("package", []):
    name = pkg_cfg.get("name")
    if not name:
        continue

    git_release_enable = bool(pkg_cfg.get("git_release_enable", ws_git_release_enable))

    own_dir = name_to_dir.get(name)
    dirs: set[str] = set()
    if own_dir is not None:
        dirs.add(own_dir)
    for included in pkg_cfg.get("changelog_include", []):
        included_dir = name_to_dir.get(included)
        if included_dir is not None:
            dirs.add(included_dir)

    changelog = f"{own_dir}/CHANGELOG.md" if own_dir else None

    result[name] = {
        "git_release_enable": git_release_enable,
        "dirs": sorted(dirs),
        "changelog": changelog,
    }

json.dump(result, sys.stdout)
PY
) || { echo "::error::polish-notes: scope-resolution python heredoc failed" >&2; exit 1; }

if ! printf '%s' "$SCOPE_JSON" | jq -e . >/dev/null 2>&1; then
  echo "::error::polish-notes: scope-resolution produced invalid JSON" >&2
  exit 1
fi

# --- Per-release loop: steps 1 (filter) / 2 / 3 / 4 / 5 / 6 --------------
release_count=$(printf '%s' "$RELEASES" | jq -e 'length') || {
  echo "::error::polish-notes: RELEASES is not valid JSON" >&2
  exit 1
}

for idx in $(seq 0 $((release_count - 1))); do
  release=$(printf '%s' "$RELEASES" | jq -c ".[$idx]")
  package=$(printf '%s' "$release" | jq -r '.package_name')
  tag=$(printf '%s' "$release" | jq -r '.tag')

  # Step 1: FILTER — packages absent from the scope map, or present but
  # git_release_enable=false, are git_only crates. They have a tag but
  # deliberately no release page: silently drop, never a failure.
  pkg_entry=$(printf '%s' "$SCOPE_JSON" | jq -c --arg pkg "$package" '.[$pkg] // empty')
  if [ -z "$pkg_entry" ]; then
    continue
  fi
  git_release_enable=$(printf '%s' "$pkg_entry" | jq -r '.git_release_enable')
  if [ "$git_release_enable" != "true" ]; then
    continue
  fi

  # Step 2: SKIP — a filtered-in package whose release page is missing
  # is a release-plz anomaly: recorded as a failure, never created.
  if ! release_json=$(gh release view "$tag" --json body 2>&1); then
    record_failure "polish-notes: $tag: gh release view failed for a git_release_enable=true package: $release_json"
    continue
  fi
  body=$(printf '%s' "$release_json" | jq -r '.body')
  if [[ "$body" == "## Summary"* ]]; then
    echo "::notice::polish-notes: $tag: body already starts with '## Summary', skipping (idempotent)"
    success_count=$((success_count + 1))
    continue
  fi

  # Step 3: SCOPE — dirs/changelog resolved above; empty scope is a
  # hard per-release failure (release-plz.toml / cargo metadata drift).
  scope_paths=$(printf '%s' "$pkg_entry" | jq -r '.dirs | join(" ")')
  changelog_path=$(printf '%s' "$pkg_entry" | jq -r '.changelog // empty')
  if [ -z "$scope_paths" ] || [ -z "$changelog_path" ]; then
    record_failure "polish-notes: $tag: empty scope (no changelog_include dirs resolved for $package)"
    continue
  fi

  # Prev tag: derive the glob from the tag template (not the package
  # name), locate $TAG in the sorted tag list, take the next line.
  # Robust against stale higher tags left by aborted runs.
  tag_glob="${tag%-v*}-v*"
  mapfile -t tag_list < <(git tag --list "$tag_glob" --sort=-v:refname)
  prev_tag=""
  for i in "${!tag_list[@]}"; do
    if [ "${tag_list[$i]}" = "$tag" ]; then
      next_idx=$((i + 1))
      if [ "$next_idx" -lt "${#tag_list[@]}" ]; then
        prev_tag="${tag_list[$next_idx]}"
      fi
      break
    fi
  done

  # Step 4: GENERATE — read-only agent, no GH_TOKEN, stdout captured.
  raw_file="$RUNNER_TEMP/raw-${tag}.txt"
  err_file="$RUNNER_TEMP/err-${tag}.txt"
  if PACKAGE="$package" TAG="$tag" PREV_TAG="$prev_tag" SCOPE_PATHS="$scope_paths" CHANGELOG_PATH="$changelog_path" \
    env -u GH_TOKEN timeout 300 opencode run --agent plan -m "$MODEL" "$(cat "$PROMPT_FILE")" \
    >"$raw_file" 2>"$err_file"; then
    :
  else
    status=$?
    if [ "$status" -eq 124 ]; then
      record_failure "polish-notes: $tag: opencode run timed out after 300s"
    else
      record_failure "polish-notes: $tag: opencode run failed (exit $status): $(tail -n 20 "$err_file" 2>/dev/null | tr '\n' ' ')"
    fi
    continue
  fi

  # Extract the document between the sentinel lines. Resetting the
  # buffer on every BEGIN line means a repeated sentinel (agent logs
  # echoing the prompt) naturally keeps only the last occurrence.
  notes_file="$RUNNER_TEMP/notes-${tag}.md"
  awk '
    $0 == "=====BEGIN BODY=====" { in_body = 1; buf = ""; next }
    $0 == "=====END BODY=====" && in_body { in_body = 0; body = buf; next }
    in_body { buf = buf $0 ORS }
    END { printf "%s", body }
  ' "$raw_file" >"$notes_file" || true

  if [ ! -s "$notes_file" ]; then
    record_failure "polish-notes: $tag: no document found between =====BEGIN BODY=====/=====END BODY===== sentinels"
    continue
  fi

  # Step 5: VALIDATE — each failure is a distinct ::error::.
  if ! head -n1 "$notes_file" | grep -qx '## Summary'; then
    record_failure "polish-notes: $tag: notes do not start with '## Summary'"
    continue
  fi
  if ! grep -qx '## Highlights' "$notes_file"; then
    record_failure "polish-notes: $tag: notes are missing a '## Highlights' section"
    continue
  fi
  if ! awk -v tag="$tag" '/CHANGELOG\.md/ && index($0, tag) > 0 { found = 1 } END { exit !found }' "$notes_file"; then
    record_failure "polish-notes: $tag: notes are missing a CHANGELOG.md link containing tag $tag"
    continue
  fi
  notes_len=$(wc -c <"$notes_file" | tr -d ' ')
  if [ "$notes_len" -lt 400 ] || [ "$notes_len" -gt 20000 ]; then
    record_failure "polish-notes: $tag: notes length ${notes_len} chars outside the [400,20000] bound"
    continue
  fi
  fence_count=$(grep -c '^```' "$notes_file" || true)
  if [ $((fence_count % 2)) -ne 0 ]; then
    record_failure "polish-notes: $tag: unbalanced code fences (${fence_count} fence lines)"
    continue
  fi

  # Step 6: PUBLISH.
  if [ "$POLISH_NOTES_SKIP_PUBLISH" = "1" ]; then
    echo "::notice::polish-notes: $tag: validated notes ready (POLISH_NOTES_SKIP_PUBLISH=1, not published): $notes_file"
    success_count=$((success_count + 1))
    continue
  fi
  if ! gh release edit "$tag" --notes-file "$notes_file"; then
    record_failure "polish-notes: $tag: gh release edit failed"
    continue
  fi
  echo "::notice::polish-notes: $tag: published polished release notes"
  success_count=$((success_count + 1))
done

# --- Step 7: REPORT -------------------------------------------------------
if [ "${#FAILURES[@]}" -gt 0 ]; then
  echo "::error::polish-notes: ${#FAILURES[@]} release(s) failed" >&2
  exit 1
fi

if [ "$success_count" -eq 0 ]; then
  echo "::notice::polish-notes: nothing to do (no git_release_enable=true releases in this cycle)"
fi

exit 0
