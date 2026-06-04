#!/usr/bin/env bash
# Unit tests for parse-backfill-tags.sh. Pure-shell, no network: the
# `gh release view` invocation is redirected through GH_RELEASE_VIEW_CMD
# to a mock that checks against a fixture list of "existing" tags.
#
# Usage:
#   ./ci/release-plz/test_parse-backfill-tags.sh
#
# Exit codes:
#   0 — all cases pass
#   1 — at least one case failed (output explains which)
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
readonly SCRIPT_DIR
readonly SCRIPT="$SCRIPT_DIR/parse-backfill-tags.sh"

if [ ! -x "$SCRIPT" ]; then
  echo "missing or non-executable: $SCRIPT" >&2
  exit 1
fi

# Shared mock for `gh release view`. The mock reads a newline-delimited
# list of "existing" tags from MOCK_EXISTING_TAGS_FILE; success means the
# tag is in the list. The script under test redirects stderr/stdout to
# /dev/null, so a bare exit code is all that matters.
TMP=$(mktemp -d)
readonly TMP
trap 'rm -rf "$TMP"' EXIT
readonly MOCK="$TMP/mock-gh-release-view"
cat >"$MOCK" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
needle="$1"
while IFS= read -r tag; do
  if [ "$tag" = "$needle" ]; then exit 0; fi
done < "${MOCK_EXISTING_TAGS_FILE:?MOCK_EXISTING_TAGS_FILE must be set}"
exit 1
EOF
chmod +x "$MOCK"

fails=0
passes=0

# run_case <name> <expected_exit> <backfill_tags> <existing_tags_csv>
#         <expected_plan> <expected_tags_csv> <expected_needs_frontend>
#
# Empty <existing_tags_csv> means "no releases exist at all". The last
# three fields are unused when expected_exit != 0.
run_case() {
  local name="$1" exp_exit="$2" backfill="$3" existing_csv="$4"
  local exp_plan="${5:-}" exp_tags_csv="${6:-}" exp_needs_fe="${7:-}"

  local out_file existing_file
  out_file=$(mktemp -p "$TMP" "out.XXXXXX")
  existing_file=$(mktemp -p "$TMP" "existing.XXXXXX")
  : >"$out_file"
  if [ -n "$existing_csv" ]; then
    tr ',' '\n' <<< "$existing_csv" > "$existing_file"
  fi

  local actual_exit=0
  BACKFILL_TAGS="$backfill" \
  GITHUB_OUTPUT="$out_file" \
  GH_RELEASE_VIEW_CMD="$MOCK" \
  MOCK_EXISTING_TAGS_FILE="$existing_file" \
    "$SCRIPT" >/dev/null 2>"$TMP/last_stderr" || actual_exit=$?

  if [ "$actual_exit" -ne "$exp_exit" ]; then
    echo "FAIL [$name]: expected exit $exp_exit, got $actual_exit"
    echo "  stderr: $(cat "$TMP/last_stderr")"
    fails=$((fails + 1))
    return
  fi

  if [ "$exp_exit" -ne 0 ]; then
    passes=$((passes + 1))
    return
  fi

  local actual_plan actual_tags_csv actual_needs_fe
  actual_plan=$(grep '^plan=' "$out_file" | head -1 | cut -d= -f2-)
  actual_tags_csv=$(grep '^tags_csv=' "$out_file" | head -1 | cut -d= -f2-)
  actual_needs_fe=$(grep '^needs_frontend=' "$out_file" | head -1 | cut -d= -f2-)

  if [ "$actual_plan" != "$exp_plan" ]; then
    echo "FAIL [$name]: plan mismatch"
    echo "  expected: $exp_plan"
    echo "  actual:   $actual_plan"
    fails=$((fails + 1))
    return
  fi
  if [ "$actual_tags_csv" != "$exp_tags_csv" ]; then
    echo "FAIL [$name]: tags_csv mismatch"
    echo "  expected: $exp_tags_csv"
    echo "  actual:   $actual_tags_csv"
    fails=$((fails + 1))
    return
  fi
  if [ "$actual_needs_fe" != "$exp_needs_fe" ]; then
    echo "FAIL [$name]: needs_frontend mismatch (expected $exp_needs_fe, got $actual_needs_fe)"
    fails=$((fails + 1))
    return
  fi

  passes=$((passes + 1))
}

# ---------------------------------------------------------------------------
# Case 1: single valid non-frontend tag
run_case "single cli tag" 0 \
  "uptrakit-cli-v0.0.3" \
  "uptrakit-cli-v0.0.3" \
  '[{"package_name":"uptrakit-cli","tag":"uptrakit-cli-v0.0.3","version":"0.0.3"}]' \
  "uptrakit-cli-v0.0.3" \
  "false"

# Case 2: single controller tag → needs_frontend=true
run_case "single controller tag" 0 \
  "uptrakit-controller-v0.0.3" \
  "uptrakit-controller-v0.0.3" \
  '[{"package_name":"uptrakit-controller","tag":"uptrakit-controller-v0.0.3","version":"0.0.3"}]' \
  "uptrakit-controller-v0.0.3" \
  "true"

# Case 3: single controller-standalone tag → needs_frontend=true (verifies
# the longer alternation branch wins over `controller`)
run_case "single controller-standalone tag" 0 \
  "uptrakit-controller-standalone-v0.0.3" \
  "uptrakit-controller-standalone-v0.0.3" \
  '[{"package_name":"uptrakit-controller-standalone","tag":"uptrakit-controller-standalone-v0.0.3","version":"0.0.3"}]' \
  "uptrakit-controller-standalone-v0.0.3" \
  "true"

# Case 4: two valid tags, one frontend one not
run_case "two tags, controller + cli" 0 \
  "uptrakit-controller-v0.0.3,uptrakit-cli-v0.0.3" \
  "uptrakit-controller-v0.0.3,uptrakit-cli-v0.0.3" \
  '[{"package_name":"uptrakit-controller","tag":"uptrakit-controller-v0.0.3","version":"0.0.3"},{"package_name":"uptrakit-cli","tag":"uptrakit-cli-v0.0.3","version":"0.0.3"}]' \
  "uptrakit-controller-v0.0.3,uptrakit-cli-v0.0.3" \
  "true"

# Case 5: pre-release tag
run_case "pre-release tag (rc.1)" 0 \
  "uptrakit-controller-v0.1.0-rc.1" \
  "uptrakit-controller-v0.1.0-rc.1" \
  '[{"package_name":"uptrakit-controller","tag":"uptrakit-controller-v0.1.0-rc.1","version":"0.1.0-rc.1"}]' \
  "uptrakit-controller-v0.1.0-rc.1" \
  "true"

# Case 6: tag whose release does not exist
run_case "non-existent release" 1 \
  "uptrakit-cli-v9.9.9" \
  ""

# Case 7a-c: invalid tag formats — each fails
run_case "invalid: no uptrakit prefix" 1 \
  "controller-v0.0.3" \
  "uptrakit-cli-v0.0.3"
run_case "invalid: unknown package" 1 \
  "uptrakit-frontend-v0.0.3" \
  "uptrakit-cli-v0.0.3"
run_case "invalid: non-semver version" 1 \
  "uptrakit-cli-v0.0" \
  "uptrakit-cli-v0.0.3"

# Case 8: empty input → empty plan, exit 0, needs_frontend=false
run_case "empty input" 0 \
  "" \
  "" \
  "[]" \
  "" \
  "false"

# Case 9: tolerated whitespace + trailing comma — canonical form emerges
run_case "whitespace and trailing comma tolerated" 0 \
  "  uptrakit-cli-v0.0.3 ,, uptrakit-mqtt-v0.0.3 ," \
  "uptrakit-cli-v0.0.3,uptrakit-mqtt-v0.0.3" \
  '[{"package_name":"uptrakit-cli","tag":"uptrakit-cli-v0.0.3","version":"0.0.3"},{"package_name":"uptrakit-mqtt","tag":"uptrakit-mqtt-v0.0.3","version":"0.0.3"}]' \
  "uptrakit-cli-v0.0.3,uptrakit-mqtt-v0.0.3" \
  "false"

# ---------------------------------------------------------------------------
echo
echo "passed: $passes"
echo "failed: $fails"
exit $((fails > 0 ? 1 : 0))
