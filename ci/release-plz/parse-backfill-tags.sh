#!/usr/bin/env bash
# Validate a comma-separated list of release tags supplied via the
# release-plz workflow's `workflow_dispatch.inputs.backfill_tags` input,
# confirm each one points at a real GitHub release, and emit a plan JSON
# the downstream backfill jobs consume.
#
# Inputs (env):
#   BACKFILL_TAGS       — comma-separated tag list (required, may be empty
#                         after trimming)
#   GITHUB_OUTPUT       — path the script writes plan/tags_csv/needs_frontend
#                         to (required; CI sets it; tests pass a temp file)
#   GH_RELEASE_VIEW_CMD — command used to confirm release existence;
#                         defaults to `gh release view`. Tests override this
#                         to a fixture function so the harness runs without
#                         network.
#
# Outputs (written to $GITHUB_OUTPUT):
#   plan           — JSON array of {package_name, tag, version} triples
#   tags_csv       — comma-joined tag list (canonical order)
#   needs_frontend — "true" if any package_name is uptrakit-controller
#                    or uptrakit-controller-standalone, else "false"
#
# Exit codes:
#   0 — plan emitted (may be empty when BACKFILL_TAGS trims to nothing)
#   1 — at least one tag failed validation, or its release does not exist
set -euo pipefail

# Use `${VAR?msg}` (no colon) so the var must be *declared* but is allowed
# to be empty. An empty BACKFILL_TAGS is a valid no-op: it emits an empty
# plan and exits 0.
: "${BACKFILL_TAGS?BACKFILL_TAGS must be set}"
: "${GITHUB_OUTPUT?GITHUB_OUTPUT must be set}"
: "${GH_RELEASE_VIEW_CMD:=gh release view}"

# Alternation order matters: longer prefixes (`controller-standalone`,
# `agent-ssh`) must come first so a tag like
# `uptrakit-controller-standalone-v0.1.0` does not greedily match
# `controller` and leave `-standalone-v0.1.0` to fail the suffix anchor.
# Trailing `(-[0-9A-Za-z.-]+)?` allows SemVer pre-release tags such as
# `v0.1.0-rc.1`.
readonly TAG_REGEX='^uptrakit-(controller-standalone|controller|agent-ssh|agent|mqtt|scheduler|cli)-v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'

# Split BACKFILL_TAGS on ','. Whitespace around each item is trimmed and
# empty items are skipped, so "a, b, ,c" parses as the canonical "a,b,c".
trim() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

# Default-IFS read with `-a` preserves empty trailing fields; we filter
# them in the loop body.
IFS=',' read -ra raw_tags <<< "$BACKFILL_TAGS"

plan_entries=()
tag_list=()
saw_frontend_pkg=false

for raw in "${raw_tags[@]}"; do
  tag=$(trim "$raw")
  [ -z "$tag" ] && continue

  if ! [[ "$tag" =~ $TAG_REGEX ]]; then
    echo "::error::backfill_tags: invalid tag '$tag' (must match $TAG_REGEX)" >&2
    exit 1
  fi

  # Strip the `-vX.Y.Z[-suffix]` tail off the matched tag to recover the
  # package name and version. Doing this with parameter expansion avoids
  # a second regex roundtrip.
  pkg="${tag%-v*}"
  version="${tag##"$pkg"-v}"

  if ! $GH_RELEASE_VIEW_CMD "$tag" >/dev/null 2>&1; then
    echo "::error::backfill_tags: release '$tag' does not exist on GitHub (we never synthesise releases via backfill)" >&2
    exit 1
  fi

  plan_entries+=("{\"package_name\":\"$pkg\",\"tag\":\"$tag\",\"version\":\"$version\"}")
  tag_list+=("$tag")
  if [ "$pkg" = "uptrakit-controller" ] || [ "$pkg" = "uptrakit-controller-standalone" ]; then
    saw_frontend_pkg=true
  fi
done

if [ "${#plan_entries[@]}" -eq 0 ]; then
  plan='[]'
  tags_csv=''
else
  # Comma-join the JSON entries by hand so we do not need jq for the
  # happy path. Validate the result with jq at the end (cheap correctness
  # check that also rejects accidental quoting bugs).
  joined=$(IFS=,; printf '%s' "${plan_entries[*]}")
  plan="[$joined]"
  tags_csv=$(IFS=,; printf '%s' "${tag_list[*]}")
fi

# Trip-wire: re-parse with jq to confirm the assembled JSON is valid.
if ! printf '%s' "$plan" | jq -e . >/dev/null; then
  echo "::error::internal error: produced invalid plan JSON: $plan" >&2
  exit 1
fi

needs_frontend=$([ "$saw_frontend_pkg" = true ] && echo true || echo false)

{
  printf 'plan=%s\n' "$plan"
  printf 'tags_csv=%s\n' "$tags_csv"
  printf 'needs_frontend=%s\n' "$needs_frontend"
} >> "$GITHUB_OUTPUT"
