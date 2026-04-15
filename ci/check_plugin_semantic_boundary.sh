#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
REPORT_ALL_MODE="${UPTRAKIT_SEMANTIC_BOUNDARY_REPORT_ALL:-0}"
REPORT_ALL_FINDINGS=()

TARGETS_DASHBOARD=(
  'ui/web-api/src/routes/**/*.rs'
  'ui/web-api/src/router.rs'
  'ui/web-api/src/routes/mod.rs'
  'ui/web-api/db_access_policy.toml'
  'ui/web-api-auth/src/**/*.rs'
  'shared/web-api-types/src/**/*.rs'
)

TARGETS_HELPERS=(
  'ui/web-api/src/**/*.rs'
  'ui/web-api-queries/src/queries/**/*.rs'
  'shared/types/src/plugin_type_id.rs'
  'plugins/infrastructure/registry/src/**/*.rs'
)

TARGETS_HELPER_DEFINITIONS=(
  'shared/types/src/plugin_type_id.rs'
)

TARGETS_PLUGIN_IDS=(
  'ui/web-api/src/**/*.rs'
  'ui/web-api-queries/src/queries/**/*.rs'
)

PERMANENT_EXCLUSIONS_UI_WEB_API_TEST_ONLY=(
  # Avoid obvious test-only files/directories under ui/web-api/src.
  'ui/web-api/src/**/tests/**'
  'ui/web-api/src/**/tests.rs'
  'ui/web-api/src/**/*_test.rs'
  'ui/web-api/src/**/*_tests.rs'
  'ui/web-api/src/**/test_*.rs'
)

PERMANENT_EXCLUSIONS_HELPERS=(
  "${PERMANENT_EXCLUSIONS_UI_WEB_API_TEST_ONLY[@]}"
)

PLUGIN_IDS_PREFIX_SCAN_FILES=(
  # Scan only production prefix (before first cfg(test)-style module gate) for these inline-test containers.
  'ui/web-api-queries/src/queries/autodiscovery/mod.rs'
  'ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs'
  'ui/web-api-queries/src/queries/discovery_allowlist.rs'
  'ui/web-api/src/routes/service_ws/handler/messages.rs'
)

PERMANENT_EXCLUSIONS_PLUGIN_IDS_MAIN_PASS=(
  "${PERMANENT_EXCLUSIONS_UI_WEB_API_TEST_ONLY[@]}"
  "${PLUGIN_IDS_PREFIX_SCAN_FILES[@]}"
)

ALLOWLIST_DASHBOARD=(
)

ALLOWLIST_HELPERS=(
)

ALLOWLIST_PLUGIN_IDS=(
)

EMPTY_EXCLUSIONS=()

record_violation() {
  local label="$1"
  local files="$2"
  local block="semantic-boundary violation: $label"$'\n'"$files"

  if [[ "$REPORT_ALL_MODE" == "1" ]]; then
    REPORT_ALL_FINDINGS+=("$block")
    return
  fi

  echo "$block"
  exit 1
}

flush_report_all_findings() {
  local block

  if [[ "$REPORT_ALL_MODE" != "1" ]]; then
    return
  fi
  if (( ${#REPORT_ALL_FINDINGS[@]} == 0 )); then
    return
  fi

  for block in "${REPORT_ALL_FINDINGS[@]}"; do
    echo "$block"
    echo
  done
  exit 1
}

contains_path() {
  local needle="$1"
  local arr_name="$2"
  local -n arr_ref="$arr_name"
  local entry
  for entry in "${arr_ref[@]}"; do
    if [[ "$entry" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

validate_target_globs() {
  local label="$1"
  local targets_name="$2"
  local -n targets_ref="$targets_name"
  local target
  local match
  for target in "${targets_ref[@]}"; do
    match="$(cd crates && rg --files -g "$target" . | head -n 1 || true)"
    if [[ -z "$match" ]]; then
      echo "semantic-boundary misconfiguration: target glob '$target' for '$label' matched 0 files under crates/"
      exit 1
    fi
  done
}

deny_in() {
  local label="$1"
  local pattern="$2"
  local targets_name="$3"
  local exclusions_name="$4"
  local allowlist_name="$5"
  local -n targets_ref="$targets_name"
  local -n exclusions_ref="$exclusions_name"
  local -n allowlist_ref="$allowlist_name"
  local files
  local rg_args=()
  local target
  for target in "${targets_ref[@]}"; do
    rg_args+=(-g "$target")
  done
  for target in "${exclusions_ref[@]}"; do
    rg_args+=(-g "!$target")
  done
  for target in "${allowlist_ref[@]}"; do
    rg_args+=(-g "!$target")
  done
  files="$(cd crates && rg -n "$pattern" "${rg_args[@]}" . || true)"
  if [[ -n "$files" ]]; then
    record_violation "$label" "$files"
  fi
}

scan_prefix_before_test_gate() {
  local rel_path="$1"
  local pattern="$2"
  local full_path="crates/$rel_path"
  local gate_line

  gate_line="$(
    awk '
      BEGIN {
        pending = 0
        cfg_line = 0
      }
      /^[[:space:]]*#\[cfg\([^]]*test[^]]*\)\]/ {
        pending = 1
        cfg_line = NR
        next
      }
      pending == 1 {
        if ($0 ~ /^[[:space:]]*$/) {
          next
        }
        if ($0 ~ /^[[:space:]]*#\[/) {
          next
        }
        if ($0 ~ /^[[:space:]]*(pub(\(crate\))?[[:space:]]+)?mod[[:space:]]+(tests|tests_common)[[:space:]]*\{/) {
          print cfg_line
          exit
        }
        pending = 0
      }
    ' "$full_path" || true
  )"

  if [[ -z "$gate_line" ]]; then
    rg -n "$pattern" "$full_path" | sed "s#^$full_path:#./$rel_path:#" || true
    return
  fi

  if (( gate_line <= 1 )); then
    return
  fi

  awk -v max="$((gate_line - 1))" -v pat="$pattern" -v file="./$rel_path" '
    NR <= max && $0 ~ pat { printf "%s:%d:%s\n", file, NR, $0 }
  ' "$full_path"
}

deny_plugin_ids_rule() {
  local label="$1"
  local pattern="$2"
  local files
  local prefix_hits=""
  local rel_path
  local hits

  deny_in "$label" "$pattern" TARGETS_PLUGIN_IDS PERMANENT_EXCLUSIONS_PLUGIN_IDS_MAIN_PASS ALLOWLIST_PLUGIN_IDS

  for rel_path in "${PLUGIN_IDS_PREFIX_SCAN_FILES[@]}"; do
    if contains_path "$rel_path" ALLOWLIST_PLUGIN_IDS; then
      continue
    fi
    hits="$(scan_prefix_before_test_gate "$rel_path" "$pattern" || true)"
    if [[ -n "$hits" ]]; then
      prefix_hits+="$hits"$'\n'
    fi
  done

  if [[ -n "$prefix_hits" ]]; then
    record_violation "$label" "$prefix_hits"
  fi
}

validate_target_globs "dashboard-icons bespoke surface" TARGETS_DASHBOARD
validate_target_globs "PluginTypeId semantic helpers" TARGETS_HELPERS
validate_target_globs "PluginTypeId semantic helper definitions" TARGETS_HELPER_DEFINITIONS
validate_target_globs "identity-specific helpers" TARGETS_HELPERS
validate_target_globs "concrete plugin-id imports in non-plugin production code" TARGETS_PLUGIN_IDS

deny_in "dashboard-icons bespoke surface" 'settings_dashboard_icons|dashboard_icons\.enabled' TARGETS_DASHBOARD EMPTY_EXCLUSIONS ALLOWLIST_DASHBOARD
deny_in "PluginTypeId semantic helper callsites/uses" 'PluginTypeId::is_package_manager|PluginTypeId::display_name|\.is_package_manager\(|\.display_name\(' TARGETS_HELPERS PERMANENT_EXCLUSIONS_HELPERS ALLOWLIST_HELPERS
deny_in "PluginTypeId semantic helper definitions" 'fn is_package_manager\(|fn display_name\(' TARGETS_HELPER_DEFINITIONS EMPTY_EXCLUSIONS ALLOWLIST_HELPERS
deny_in "identity-specific helpers" 'fn is_[a-z0-9_]*dashboard|fn has_[a-z0-9_]*dashboard|is_dashboard_icons|has_dashboard_icons' TARGETS_HELPERS PERMANENT_EXCLUSIONS_HELPERS ALLOWLIST_HELPERS
deny_plugin_ids_rule "plugin_ids token references in non-plugin production code" 'plugin_ids'
flush_report_all_findings
