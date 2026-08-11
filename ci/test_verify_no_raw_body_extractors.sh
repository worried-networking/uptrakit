#!/usr/bin/env bash
# test_verify_no_raw_body_extractors.sh — checked-in probe matrix for the
# baseline-subset history check in ci/verify_no_raw_body_extractors.sh (spec
# 2026-08-06 item 6). This is the most intricate bash in ci/ (bijective
# renames, two baseline modes, four skip paths) and must not be verified only
# by one-shot manual probes.
#
# Every case below builds a throwaway git repo under a private mktemp -d root
# from purely synthetic fixtures — never the real tree's rows or files — then
# copies the gate script under test into it and runs it against that sandbox.
# Assertions target MARKER STRINGS in captured stderr/stdout, not bare exit
# codes, because STALE/RESIDUAL/raw-scan findings can legitimately co-fire
# with the history check under test.
set -euo pipefail

cd "$(dirname "$0")/.."
REPO_ROOT="$(pwd)"
GATE_SCRIPT="$REPO_ROOT/ci/verify_no_raw_body_extractors.sh"

if [ ! -f "$GATE_SCRIPT" ]; then
  echo "gate script not found at $GATE_SCRIPT" >&2
  exit 1
fi

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

# Full isolation from the invoking user's git identity, signing config, and
# hook templates — these are throwaway fixture repos, not the project repo,
# so hermetic git behavior here is not a "skip hooks on the real repo" act.
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_SYSTEM=/dev/null
export HOME="$TMP_ROOT/home"
mkdir -p "$HOME"

fail=0
pass_count=0
total_count=0

assert_contains() {
  local desc="$1" haystack="$2" needle="$3"
  total_count=$((total_count + 1))
  if printf '%s' "$haystack" | grep -qF -- "$needle"; then
    pass_count=$((pass_count + 1))
    echo "PASS: $desc"
  else
    fail=1
    echo "FAIL: $desc — expected to find: $needle" >&2
    echo "--- captured output ---" >&2
    printf '%s\n' "$haystack" >&2
    echo "--- end output ---" >&2
  fi
}

assert_not_contains() {
  local desc="$1" haystack="$2" needle="$3"
  total_count=$((total_count + 1))
  if printf '%s' "$haystack" | grep -qF -- "$needle"; then
    fail=1
    echo "FAIL: $desc — unexpected: $needle" >&2
    echo "--- captured output ---" >&2
    printf '%s\n' "$haystack" >&2
    echo "--- end output ---" >&2
  else
    pass_count=$((pass_count + 1))
    echo "PASS: $desc"
  fi
}

assert_exit_code() {
  local desc="$1" actual="$2" expected="$3"
  total_count=$((total_count + 1))
  if [ "$actual" -eq "$expected" ]; then
    pass_count=$((pass_count + 1))
    echo "PASS: $desc"
  else
    fail=1
    echo "FAIL: $desc — expected exit $expected, got $actual" >&2
  fi
}

# new_sandbox <name> — creates $TMP_ROOT/<name> with the routes/allowlist
# layout the gate script expects, a copy of the gate script under test, and
# an initialized git repo with a local (non-global) identity.
new_sandbox() {
  local name="$1"
  local dir="$TMP_ROOT/$name"
  mkdir -p "$dir/ci" "$dir/crates/ui/web-api/src/routes"
  cp "$GATE_SCRIPT" "$dir/ci/verify_no_raw_body_extractors.sh"
  chmod +x "$dir/ci/verify_no_raw_body_extractors.sh"
  (
    cd "$dir"
    git init -q
    git config user.email "sandbox@example.invalid"
    git config user.name "Sandbox Test"
  )
  printf '%s' "$dir"
}

# write_routes_file <path> <name:kind...> — kind is "bytes" (flagged raw_body
# signature) or "plain" (unflagged, ordinary handler). Overwrites <path>.
write_routes_file() {
  local path="$1"
  shift
  : >"$path"
  local spec name kind
  for spec in "$@"; do
    name="${spec%%:*}"
    kind="${spec#*:}"
    case "$kind" in
      bytes) printf 'pub async fn %s(body: Bytes) {\n}\n\n' "$name" >>"$path" ;;
      plain) printf 'pub async fn %s(id: u32) {\n}\n\n' "$name" >>"$path" ;;
      *)
        echo "write_routes_file: unknown kind '$kind' for '$name'" >&2
        exit 1
        ;;
    esac
  done
}

# write_allowlist <path> <row...> — overwrites <path> with a header plus zero
# or more raw_body|path|regex rows (zero rows is a legal state: file exists,
# no raw_* rows — see case 11).
write_allowlist() {
  local path="$1"
  shift
  {
    echo "# synthetic sandbox allowlist"
    echo "# Row format: class|path|fn-regex"
    local row
    for row in "$@"; do
      printf '%s\n' "$row"
    done
  } >"$path"
}

append_allowlist_row() {
  local path="$1" row="$2"
  printf '%s\n' "$row" >>"$path"
}

commit_all() {
  local dir="$1" msg="$2"
  (cd "$dir" && git add -A && git commit -q -m "$msg")
}

# run_gate <dir> [args...] — sets GATE_OUT (combined stdout+stderr) and
# GATE_RC (exit code) as script globals.
run_gate() {
  local dir="$1"
  shift
  set +e
  GATE_OUT="$(cd "$dir" && bash ci/verify_no_raw_body_extractors.sh "$@" 2>&1)"
  GATE_RC=$?
  set -e
}

ALLOWLIST_REL="ci/verify_no_raw_body_extractors_allowlist.txt"
ROUTES_REL="crates/ui/web-api/src/routes"

# --- Case 1: append a row that never existed at the baseline -> VIOLATION ---
case1_dir="$(new_sandbox case1)"
write_routes_file "$case1_dir/$ROUTES_REL/probe.rs" "keepme:bytes"
write_allowlist "$case1_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn keepme\b"
commit_all "$case1_dir" "baseline"
append_allowlist_row "$case1_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_new.rs|fn probe_new\b"
run_gate "$case1_dir" HEAD
assert_contains "case1: appended row absent from baseline is flagged" "$GATE_OUT" "not in baseline"
assert_exit_code "case1: exit code is 1" "$GATE_RC" 1

# --- Case 2: swap — delete a converted row, add an unrelated new one -------
case2_dir="$(new_sandbox case2)"
write_routes_file "$case2_dir/$ROUTES_REL/probe.rs" "func_del:bytes" "func_clean:plain"
write_allowlist "$case2_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn func_del\b"
commit_all "$case2_dir" "baseline"
# "Convert" func_del away (drop its fn and its row) while regressing
# func_clean to a raw read and allowlisting it — the net-zero-count swap the
# old row-count ratchet could not see.
write_routes_file "$case2_dir/$ROUTES_REL/probe.rs" "func_clean:bytes"
write_allowlist "$case2_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn func_clean\b"
run_gate "$case2_dir" HEAD
assert_contains "case2: swap (delete+unrelated add) is caught by the history check" "$GATE_OUT" "not in baseline"
assert_exit_code "case2: exit code is 1" "$GATE_RC" 1

# --- Case 3: delete one row (clean), then delete all rows (must complete) --
case3_dir="$(new_sandbox case3)"
write_routes_file "$case3_dir/$ROUTES_REL/probe.rs" "rowx:bytes" "rowy:bytes"
write_allowlist "$case3_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn rowx\b" \
  "raw_body|$ROUTES_REL/probe.rs|fn rowy\b"
commit_all "$case3_dir" "baseline"

# Leg 1: delete rowX's allowlist row only (its fn stays raw — an unrelated
# "not allowlisted" finding is expected and fine; the history check itself
# must stay quiet for a plain deletion).
write_allowlist "$case3_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn rowy\b"
run_gate "$case3_dir" HEAD
assert_not_contains "case3 leg1: deleting one row alone is not a baseline violation" "$GATE_OUT" "not in baseline"

# Leg 2: delete ALL rows AND scrub every flagged signature from the tree —
# otherwise the raw scan sets violations=1 for an unrelated reason and the
# script exits 1 without printing OK, which would fail this assertion for the
# wrong reason. This pins the zero-row `grep ... || true` pipefail guard.
write_allowlist "$case3_dir/$ALLOWLIST_REL"
write_routes_file "$case3_dir/$ROUTES_REL/probe.rs" "harmless:plain"
run_gate "$case3_dir" HEAD
assert_contains "case3 leg2: zero-row allowlist against a non-empty baseline still completes" "$GATE_OUT" "verify_no_raw_body_extractors: OK"
assert_exit_code "case3 leg2: exit code is 0" "$GATE_RC" 0

# --- Case 4: baseline-landed deletion, then re-add in the working tree -----
case4_dir="$(new_sandbox case4)"
write_routes_file "$case4_dir/$ROUTES_REL/probe.rs" "rowx:bytes"
write_allowlist "$case4_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn rowx\b"
commit_all "$case4_dir" "commit1: row present"
write_allowlist "$case4_dir/$ALLOWLIST_REL"
commit_all "$case4_dir" "commit2 (HEAD): row deleted"
# Working tree re-adds the row the baseline (HEAD) already deleted — the
# delete-then-re-add hole this check exists to close.
write_allowlist "$case4_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn rowx\b"
run_gate "$case4_dir" HEAD
assert_contains "case4: re-adding a row the baseline already deleted is a violation" "$GATE_OUT" "not in baseline"
assert_exit_code "case4: exit code is 1" "$GATE_RC" 1

# --- Case 5: rename (old row removed) -> history check passes --------------
case5_dir="$(new_sandbox case5)"
write_routes_file "$case5_dir/$ROUTES_REL/probe_old.rs" "func_r:bytes"
write_allowlist "$case5_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_old.rs|fn func_r\b"
commit_all "$case5_dir" "baseline"
# Only the ALLOWLIST row's path moves; the .rs file itself is untouched. This
# isolates the history check's bijective-rename rule from the STALE check,
# which correctly still fires below since the file didn't actually move.
write_allowlist "$case5_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_moved.rs|fn func_r\b"
run_gate "$case5_dir" HEAD
assert_not_contains "case5: bijective rename is not flagged not-in-baseline" "$GATE_OUT" "not in baseline"
assert_not_contains "case5: bijective rename does not trip the double-claim check" "$GATE_OUT" "same renamed base row"

# --- Case 6: same as 5 but the old-path row is kept -> addition ------------
case6_dir="$(new_sandbox case6)"
write_routes_file "$case6_dir/$ROUTES_REL/probe_old.rs" "func_r:bytes"
write_allowlist "$case6_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_old.rs|fn func_r\b"
commit_all "$case6_dir" "baseline"
write_allowlist "$case6_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_old.rs|fn func_r\b" \
  "raw_body|$ROUTES_REL/probe_moved.rs|fn func_r\b"
run_gate "$case6_dir" HEAD
assert_contains "case6: keeping the old-path row makes the new one an addition, not a rename" "$GATE_OUT" "not in baseline"
assert_exit_code "case6: exit code is 1" "$GATE_RC" 1

# --- Case 7: split one base row into two current rows -> double-claim ------
case7_dir="$(new_sandbox case7)"
write_routes_file "$case7_dir/$ROUTES_REL/probe_old.rs" "func_s:bytes"
write_allowlist "$case7_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_old.rs|fn func_s\b"
commit_all "$case7_dir" "baseline"
write_allowlist "$case7_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_b.rs|fn func_s\b" \
  "raw_body|$ROUTES_REL/probe_c.rs|fn func_s\b"
run_gate "$case7_dir" HEAD
assert_contains "case7: a split claiming the same base row twice is rejected" "$GATE_OUT" "two added rows claim the same renamed base row"
assert_exit_code "case7: exit code is 1" "$GATE_RC" 1

# --- Case 11: zero-row BASELINE (file exists, no raw_* rows) ---------------
case11_dir="$(new_sandbox case11)"
write_routes_file "$case11_dir/$ROUTES_REL/probe.rs" "funcz:bytes"
write_allowlist "$case11_dir/$ALLOWLIST_REL"
commit_all "$case11_dir" "baseline: allowlist exists, zero raw rows"
write_allowlist "$case11_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn funcz\b"
run_gate "$case11_dir" HEAD
assert_contains "case11: adding a row against a zero-row baseline is a violation" "$GATE_OUT" "not in baseline"
assert_not_contains "case11: a zero-row baseline is not treated as an absent-file warn-skip" "$GATE_OUT" "allowlist absent"
assert_exit_code "case11: exit code is 1" "$GATE_RC" 1

# --- Case 12: no-arg mode resolves baseline via merge-base with origin/main ---
case12_dir="$(new_sandbox case12)"
write_routes_file "$case12_dir/$ROUTES_REL/probe.rs" "rowm:bytes"
write_allowlist "$case12_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe.rs|fn rowm\b"
commit_all "$case12_dir" "baseline"
# No real remote exists in these throwaway repos; a remote-tracking ref is
# faked directly so the no-arg fallback (origin/main) resolves, exercising
# the merge-base baseline mode rather than the explicit-BASE_REF mode every
# other case above pins via the `HEAD` argument.
(cd "$case12_dir" && git update-ref refs/remotes/origin/main "$(git rev-parse HEAD)")

# Leg 1: clean tree — the no-arg invocation must resolve
# merge-base(HEAD, origin/main) and complete normally.
run_gate "$case12_dir"
assert_contains "case12 leg1: no-arg merge-base mode completes on a clean tree" "$GATE_OUT" "verify_no_raw_body_extractors: OK"
assert_exit_code "case12 leg1: exit code is 0" "$GATE_RC" 0

# Leg 2: append a row absent from the origin/main-derived baseline — still caught.
append_allowlist_row "$case12_dir/$ALLOWLIST_REL" \
  "raw_body|$ROUTES_REL/probe_new.rs|fn probe_new\b"
run_gate "$case12_dir"
assert_contains "case12 leg2: no-arg merge-base mode still catches an unbaselined addition" "$GATE_OUT" "not in baseline"
assert_exit_code "case12 leg2: exit code is 1" "$GATE_RC" 1

# --- Case 13: explicit non-hex BASE_REF that doesn't resolve -> hard FATAL ---
case13_dir="$(new_sandbox case13)"
write_routes_file "$case13_dir/$ROUTES_REL/probe.rs" "harmless:plain"
write_allowlist "$case13_dir/$ALLOWLIST_REL"
commit_all "$case13_dir" "baseline"
run_gate "$case13_dir" "not-a-real-ref-zzz"
assert_contains "case13: non-hex unresolvable BASE_REF is a hard FATAL, not a skip" "$GATE_OUT" "FATAL: explicit BASE_REF"
assert_not_contains "case13: FATAL path does not also print the WARNING skip message" "$GATE_OUT" "WARNING: allowlist history sub-check SKIPPED"
assert_exit_code "case13: exit code is 1" "$GATE_RC" 1

# --- Case 14: bare-SHA unresolvable BASE_REF -> WARNING skip, other checks run
case14_dir="$(new_sandbox case14)"
write_routes_file "$case14_dir/$ROUTES_REL/probe.rs" "harmless:plain"
write_allowlist "$case14_dir/$ALLOWLIST_REL"
commit_all "$case14_dir" "baseline"
run_gate "$case14_dir" "0000000000000000000000000000000000000000"
assert_contains "case14: unresolvable bare-SHA BASE_REF warns instead of failing" "$GATE_OUT" "WARNING: allowlist history sub-check SKIPPED"
assert_contains "case14: unresolvable bare-SHA BASE_REF still completes the other checks" "$GATE_OUT" "verify_no_raw_body_extractors: OK"
assert_exit_code "case14: exit code is 0" "$GATE_RC" 0

echo
echo "verify_no_raw_body_extractors sandbox probe matrix: $pass_count/$total_count passed"
if [ "$fail" -ne 0 ]; then
  echo "test_verify_no_raw_body_extractors: FAIL" >&2
  exit 1
fi
echo "test_verify_no_raw_body_extractors: OK"
