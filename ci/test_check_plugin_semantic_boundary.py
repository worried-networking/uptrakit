from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent
HELPER = ROOT / "check_plugin_semantic_boundary.py"
SHELL_HELPER = ROOT / "check_plugin_semantic_boundary.sh"
FIXTURES = ROOT / "testdata" / "plugin_semantic_boundary"


def run_checker(
    fixture_name: str | None = None,
    *,
    cwd: Path | None = None,
    root: Path | None = None,
    output_format: str = "text",
) -> subprocess.CompletedProcess[str]:
    if root is None and fixture_name is not None:
        root = FIXTURES / fixture_name
    command = [
        "python3",
        str(HELPER),
    ]
    if root is not None:
        command.extend(["--root", str(root)])
    command.extend(["--format", output_format])
    return subprocess.run(
        command,
        cwd=cwd,
        capture_output=True,
        text=True,
    )


def run_shell_checker(
    fixture_name: str,
    *,
    report_all: bool = False,
) -> subprocess.CompletedProcess[str]:
    fixture_root = FIXTURES / fixture_name
    with tempfile.TemporaryDirectory() as tmpdir:
        temp_root = Path(tmpdir) / "repo"
        shutil.copytree(fixture_root, temp_root)
        temp_ci_dir = temp_root / "ci"
        temp_ci_dir.mkdir(exist_ok=True)
        temp_shell_helper = temp_ci_dir / "check_plugin_semantic_boundary.sh"
        shutil.copy2(SHELL_HELPER, temp_shell_helper)
        temp_shell_helper.chmod(0o755)

        env = os.environ.copy()
        if report_all:
            env["UPTRAKIT_SEMANTIC_BOUNDARY_REPORT_ALL"] = "1"

        return subprocess.run(
            ["bash", str(temp_shell_helper)],
            cwd=temp_root,
            capture_output=True,
            text=True,
            env=env,
        )


def parse_shell_findings(output: str) -> list[tuple[str, str, int]]:
    findings: list[tuple[str, str, int]] = []
    current_label: str | None = None

    for raw_line in output.splitlines():
        line = raw_line.rstrip()
        if line.startswith("semantic-boundary violation: "):
            current_label = line.removeprefix("semantic-boundary violation: ")
            continue
        if not line:
            current_label = None
            continue
        if current_label is None or not line.startswith("./"):
            continue

        path, line_no, _excerpt = line[2:].split(":", 2)
        findings.append((current_label, f"crates/{path}", int(line_no)))

    return findings


def index_python_rule_ids_by_location(
    findings: list[dict[str, object]],
) -> dict[tuple[str, int], set[str]]:
    indexed: dict[tuple[str, int], set[str]] = {}
    for finding in findings:
        key = (str(finding["path"]), int(finding["line"]))
        indexed.setdefault(key, set()).add(str(finding["rule_id"]))
    return indexed


class PluginSemanticBoundaryTests(unittest.TestCase):
    def assertRuleIds(
        self,
        result: subprocess.CompletedProcess[str],
        expected: set[str],
        unexpected: set[str],
    ) -> None:
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        for rule_id in expected:
            self.assertIn(rule_id, output, msg=output)
        for rule_id in unexpected:
            self.assertNotIn(rule_id, output, msg=output)

    def assertMatchKinds(
        self,
        fixture_name: str,
        expected_match_kinds_by_rule: dict[str, str],
    ) -> None:
        result = run_checker(fixture_name, output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        actual = {finding["rule_id"]: finding["match_kind"] for finding in payload["findings"]}
        for rule_id, match_kind in expected_match_kinds_by_rule.items():
            self.assertIn(rule_id, actual, msg=output)
            self.assertEqual(actual[rule_id], match_kind, msg=output)

    def test_pass_fixture_succeeds(self) -> None:
        result = run_checker("pass")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
        output = result.stdout + result.stderr
        self.assertNotIn("legacy_migration_only", output, msg=output)

    def test_root_defaults_to_current_working_directory(self) -> None:
        fixture_root = FIXTURES / "pass"
        result = run_checker(cwd=fixture_root)
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_rule_families_produce_rule_ids(self) -> None:
        all_rule_ids = {
            "plugin-core-import",
            "concrete-plugin-import",
            "plugin-ids-reference",
            "forbidden-plugin-helper",
            "hardcoded-plugin-type-literal",
            "manifest-plugin-dependency",
        }
        cases = [
            ("fail/imports", {"plugin-core-import"}),
            ("fail/concrete_imports", {"plugin-core-import", "concrete-plugin-import"}),
            ("fail/plugin_ids_reference", {"plugin-ids-reference"}),
            ("fail/plugin_ids_wildcard_import", {"plugin-ids-reference"}),
            ("fail/helper_definition", {"forbidden-plugin-helper"}),
            ("fail/helper_callsite", {"forbidden-plugin-helper"}),
            ("fail/literals", {"hardcoded-plugin-type-literal"}),
            ("fail/manifests", {"manifest-plugin-dependency"}),
        ]
        for fixture_name, expected_rules in cases:
            with self.subTest(fixture=fixture_name):
                result = run_checker(fixture_name)
                self.assertRuleIds(result, expected_rules, all_rule_ids - expected_rules)

    def test_inline_fully_qualified_core_and_concrete_imports_are_rejected(self) -> None:
        all_rule_ids = {
            "plugin-core-import",
            "concrete-plugin-import",
            "plugin-ids-reference",
            "forbidden-plugin-helper",
            "hardcoded-plugin-type-literal",
            "manifest-plugin-dependency",
        }
        result = run_checker("fail/inline_import_references")
        output = result.stdout + result.stderr
        self.assertRuleIds(
            result,
            {"plugin-core-import", "concrete-plugin-import"},
            all_rule_ids - {"plugin-core-import", "concrete-plugin-import"},
        )
        self.assertIn("uptrakit_plugin_infrastructure_core::BatchFetchResult", output, msg=output)
        self.assertIn("uptrakit_plugin_package_manager_apt::AptPlugin", output, msg=output)

    def test_alias_imports_and_notification_imports_are_rejected(self) -> None:
        result = run_checker("fail/concrete_imports")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("plugin-core-import", output, msg=output)
        self.assertIn("concrete-plugin-import", output, msg=output)
        self.assertIn("uptrakit_plugin_infrastructure_core", output, msg=output)
        self.assertIn("uptrakit_plugin_package_manager_apt", output, msg=output)
        self.assertIn("uptrakit_notification_plugin_email", output, msg=output)

    def test_plugin_ids_import_alias_and_namespace_references_are_rejected(self) -> None:
        result = run_checker("fail/plugin_ids_reference", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        values = {finding["match_value"] for finding in findings}
        self.assertIn("plugin_ids::GENERIC_SHELL", values, msg=output)
        self.assertIn("plugin_ids :: GENERIC_SHELL", values, msg=output)
        self.assertIn("ids::GENERIC_SHELL", values, msg=output)
        self.assertIn("id_catalog::WEBHOOK", values, msg=output)
        self.assertIn("GENERIC_SHELL", values, msg=output)

    def test_plugin_ids_inline_qualified_path_is_rejected(self) -> None:
        result = run_checker("fail/plugin_ids_inline_qualified", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            f for f in payload["findings"]
            if f["rule_id"] == "plugin-ids-reference"
        ]
        self.assertTrue(len(findings) > 0, msg=f"Expected plugin-ids-reference findings\n{output}")
        values = {f["match_value"] for f in findings}
        self.assertTrue(
            any("plugin_ids::GENERIC_SHELL" in v for v in values),
            msg=f"Expected plugin_ids::GENERIC_SHELL in findings, got {values}\n{output}",
        )

    def test_plugin_ids_alias_chains_register_transitive_bindings(self) -> None:
        result = run_checker("fail/plugin_ids_reference", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        values = {finding["match_value"] for finding in findings}
        self.assertIn("ids3::WEBHOOK", values, msg=output)
        self.assertIn("HOOK", values, msg=output)

    def test_plugin_ids_wildcard_import_references_are_rejected(self) -> None:
        result = run_checker("fail/plugin_ids_wildcard_import", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        values = {finding["match_value"] for finding in findings}
        self.assertIn("GENERIC_SHELL", values, msg=output)
        self.assertIn("WEBHOOK", values, msg=output)
        self.assertIn("PACKAGE_MANAGER_APT", values, msg=output)

    def test_nested_plugin_ids_brace_imports_register_aliases_and_constants(self) -> None:
        result = run_checker("fail/plugin_ids_nested_brace_import", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        values = {finding["match_value"] for finding in findings}
        self.assertIn("ids::WEBHOOK", values, msg=output)
        self.assertIn("GS", values, msg=output)

    def test_registry_and_catalogue_surface_imports_are_allowed(self) -> None:
        result = run_checker("pass/registry_catalogue_import_surface")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_forbidden_helper_definition_in_plugin_type_id_is_rejected(self) -> None:
        result = run_checker("fail/helper_definition")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("forbidden-plugin-helper", output, msg=output)
        self.assertIn("is_package_manager", output, msg=output)
        self.assertIn("crates/shared/types/src/plugin_type_id.rs", output, msg=output)
        self.assertNotIn("crates/shared/types/src/lib.rs", output, msg=output)
        self.assertIn("fn display_name(", output, msg=output)

    def test_forbidden_plugin_type_id_helper_callsite_in_app_is_rejected(self) -> None:
        result = run_checker("fail/helper_callsite")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("forbidden-plugin-helper", output, msg=output)
        self.assertIn("crates/app/src/lib.rs", output, msg=output)
        self.assertIn(".is_package_manager(", output, msg=output)
        self.assertIn("PluginTypeId::display_name(", output, msg=output)
        self.assertIn("PluginTypeId :: display_name(", output, msg=output)
        self.assertIn("make_id(id).display_name(", output, msg=output)
        self.assertIn("{ make_id(id) }.display_name(", output, msg=output)
        self.assertIn("self.plugin_types[0].display_name(", output, msg=output)
        self.assertIn("let _ = alias_id.display_name();", output, msg=output)
        self.assertIn("make_plugin_type().display_name();", output, msg=output)
        self.assertIn(
            "make_plugin_type_with_generic::<u8>(source()).display_name();",
            output,
            msg=output,
        )
        self.assertIn(
            "ids.into_iter().collect::<Vec<_>>()[0].display_name(",
            output,
            msg=output,
        )
        self.assertIn("self.plugin_type.display_name();", output, msg=output)
        self.assertIn("self.plugin_type.is_package_manager();", output, msg=output)
        self.assertIn("let _ = Id::display_name(id);", output, msg=output)
        self.assertIn("let _ = Id::is_package_manager(id);", output, msg=output)

    def test_forbidden_helper_callsite_detects_newline_split_calls(self) -> None:
        result = run_checker("fail/helper_callsite", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "forbidden-plugin-helper"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        line_and_value = {(finding["line"], finding["match_value"]) for finding in findings}
        self.assertIn((55, "display_name"), line_and_value, msg=output)
        self.assertIn((60, "display_name"), line_and_value, msg=output)
        self.assertIn((65, "display_name"), line_and_value, msg=output)
        self.assertIn((70, "is_package_manager"), line_and_value, msg=output)
        self.assertIn((98, "display_name"), line_and_value, msg=output)
        self.assertIn((100, "is_package_manager"), line_and_value, msg=output)

    def test_forbidden_helper_callsite_detects_direct_plugin_type_fields(self) -> None:
        result = run_checker("fail/helper_callsite", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "forbidden-plugin-helper"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        line_and_value = {(finding["line"], finding["match_value"]) for finding in findings}
        self.assertIn((92, "display_name"), line_and_value, msg=output)
        self.assertIn((93, "is_package_manager"), line_and_value, msg=output)

    def test_forbidden_helper_callsite_in_plugin_type_id_is_rejected(self) -> None:
        result = run_checker("fail/helper_callsite")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("forbidden-plugin-helper", output, msg=output)
        self.assertIn("crates/shared/types/src/plugin_type_id.rs", output, msg=output)
        self.assertIn(".display_name(", output, msg=output)

    def test_forbidden_helper_callsite_detects_inferred_plugin_type_locals(self) -> None:
        result = run_checker("fail/helper_callsite_inferred_local")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("forbidden-plugin-helper", output, msg=output)
        self.assertIn("crates/app/src/lib.rs", output, msg=output)
        self.assertIn("let _ = inferred.display_name();", output, msg=output)
        self.assertIn("let _ = inferred_nested.is_package_manager();", output, msg=output)
        self.assertIn("let _ = inferred_multiline.display_name();", output, msg=output)
        self.assertIn(
            "let _ = inferred_multiline_nested.is_package_manager();",
            output,
            msg=output,
        )

    def test_forbidden_helper_function_items_are_rejected_once_per_binding(self) -> None:
        result = run_checker("fail/helper_function_item", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "forbidden-plugin-helper"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        line_and_value = {(finding["line"], finding["match_value"]) for finding in findings}
        self.assertEqual({5, 6}, {finding["line"] for finding in findings}, msg=output)
        self.assertEqual(
            {(5, "display_name"), (6, "is_package_manager")},
            line_and_value,
            msg=output,
        )
        self.assertIn(
            "let _display_name = PluginTypeId::display_name;",
            {finding["excerpt"] for finding in findings},
            msg=output,
        )
        self.assertIn(
            "let _is_package_manager = Id::is_package_manager;",
            {finding["excerpt"] for finding in findings},
            msg=output,
        )

    def test_unrelated_display_name_method_stays_clean(self) -> None:
        result = run_checker("pass/unrelated_display_name_method_clean")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_method_return_name_collision_does_not_infer_plugin_type_id(self) -> None:
        result = run_checker("pass/unrelated_plugin_type_returning_name_collision")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_plugin_type_id_binding_scope_does_not_leak_into_unrelated_blocks(self) -> None:
        result = run_checker("pass/helper_callsite_scope_isolated")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_scope_tracking_ignores_brace_char_literals(self) -> None:
        result = run_checker("pass/helper_callsite_scope_char_literal_braces")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_allowlist_suppresses_exactly_one_finding(self) -> None:
        result = run_checker("fail/allowlist")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("crates/app/src/lib.rs", output, msg=output)
        self.assertIn("generic_shell", output, msg=output)
        self.assertNotIn("frontend/src/lib.ts", output, msg=output)
        self.assertNotIn("releases_github", output, msg=output)

    def test_inline_test_module_exclusion_works(self) -> None:
        result = run_checker("pass/inline_test_module_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_same_line_cfg_test_items_are_stripped_without_shifting_line_numbers(self) -> None:
        result = run_checker(
            "fail/same_line_cfg_test_item_line_numbers_preserved",
            output_format="json",
        )
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        self.assertEqual(1, len(findings), msg=output)
        self.assertEqual(10, findings[0]["line"], msg=output)
        self.assertEqual("plugin_ids::HOOK_SYSTEMD", findings[0]["match_value"], msg=output)

    def test_cfg_all_inline_test_module_exclusion_works(self) -> None:
        result = run_checker("pass/inline_cfg_all_test_module_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_namespaced_test_attribute_exclusion_works(self) -> None:
        result = run_checker("pass/inline_test_module_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_trailing_inline_comments_on_test_attributes_are_ignored(self) -> None:
        result = run_checker("pass/inline_test_module_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_test_only_item_skipping_ignores_braces_in_literals_comments_and_char_literals(self) -> None:
        result = run_checker("fail/test_item_brace_depth_over_skip", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        line_and_value = {(finding["line"], finding["match_value"]) for finding in findings}
        self.assertIn((25, "plugin_ids::GENERIC_SHELL"), line_and_value, msg=output)
        self.assertIn((29, "plugin_ids::HOOK_SYSTEMD"), line_and_value, msg=output)

    def test_stripping_test_only_items_preserves_production_finding_line_numbers(self) -> None:
        result = run_checker("fail/test_item_line_numbers_preserved", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        self.assertEqual(1, len(findings), msg=output)
        self.assertEqual(21, findings[0]["line"], msg=output)
        self.assertEqual("plugin_ids::GENERIC_SHELL", findings[0]["match_value"], msg=output)

    def test_production_test_filename_is_still_scanned(self) -> None:
        result = run_checker("fail/test_filename", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = payload["findings"]
        paths = {str(finding["path"]) for finding in findings}
        self.assertEqual({"crates/app/src/config_test.rs"}, paths, msg=output)
        self.assertIn("plugin-ids-reference", output, msg=output)
        self.assertNotIn("frontend/src/components/widget.test.ts", output, msg=output)
        self.assertNotIn("frontend/src/components/widget.stories.ts", output, msg=output)
        self.assertNotIn("hardcoded-plugin-type-literal", output, msg=output)

    def test_shared_types_module_is_scanned_except_for_exact_constant_definitions(self) -> None:
        result = run_checker("fail/shared_types_scope")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("plugin-ids-reference", output, msg=output)
        self.assertIn("crates/shared/types/src/lib.rs", output, msg=output)

    def test_plugin_type_id_file_only_exempts_canonical_constant_lines(self) -> None:
        result = run_checker("fail/plugin_type_id_scope")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("hardcoded-plugin-type-literal", output, msg=output)
        self.assertIn("crates/shared/types/src/plugin_type_id.rs", output, msg=output)
        self.assertIn("runtime_plugin_type", output, msg=output)

    def test_raw_string_parsing_works_for_rust_and_frontend(self) -> None:
        result = run_checker("fail/literals")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("releases_github", output, msg=output)
        self.assertIn("infrastructure_proxmox", output, msg=output)
        self.assertIn("webhook", output, msg=output)
        self.assertIn("hardcoded-plugin-type-literal", output, msg=output)
        self.assertIn("frontend/src/channel.js", output, msg=output)

    def test_manifest_scanning_covers_plain_target_and_workspace_dependencies(self) -> None:
        result = run_checker("fail/manifests")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("manifest-plugin-dependency", output, msg=output)
        self.assertIn("crates/plain/Cargo.toml", output, msg=output)
        self.assertIn("crates/targeted/Cargo.toml", output, msg=output)
        self.assertIn("crates/workspace/Cargo.toml", output, msg=output)
        self.assertIn("uptrakit-notification-plugin-email", output, msg=output)

    def test_manifest_scanning_allows_registry_catalogue_dependency(self) -> None:
        result = run_checker("pass/manifest_registry_dependency")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_manifest_scanning_ignores_root_manifest_outside_crates_scope(self) -> None:
        result = run_checker("pass/root_manifest_out_of_scope")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_route_path_literal_context_is_detected(self) -> None:
        result = run_checker("fail/literal_route_context")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("hardcoded-plugin-type-literal", output, msg=output)
        self.assertIn("/api/plugin-types/releases_github/config", output, msg=output)

    def test_multiline_literal_context_is_detected(self) -> None:
        result = run_checker("fail/literal_multiline_context")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("hardcoded-plugin-type-literal", output, msg=output)
        self.assertIn("releases_github", output, msg=output)
        self.assertIn("releases_gitlab", output, msg=output)
        self.assertIn("generic_shell", output, msg=output)
        self.assertIn("webhook", output, msg=output)
        self.assertIn("telegram", output, msg=output)
        self.assertIn("crates/app/src/lib.rs", output, msg=output)
        self.assertIn("frontend/src/lib.ts", output, msg=output)

    def test_multiline_literals_preserve_comment_like_content_inside_literal_bodies(self) -> None:
        result = run_checker(
            "fail/literal_multiline_comment_like_content",
            output_format="json",
        )
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "hardcoded-plugin-type-literal"
        ]
        actual = {(finding["path"], finding["match_value"]) for finding in findings}
        self.assertIn(
            ("crates/app/src/lib.rs", "releases_github"),
            actual,
            msg=output,
        )
        self.assertIn(
            ("frontend/src/lib.ts", "webhook"),
            actual,
            msg=output,
        )

    def test_legacy_dashboard_bespoke_surface_uses_migration_only_python_rule(self) -> None:
        result = run_checker("fail/shell_legacy_parity", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        rule_ids_by_location = index_python_rule_ids_by_location(payload["findings"])

        for line in (4, 5):
            with self.subTest(line=line):
                location = ("crates/ui/web-api/src/routes/mod.rs", line)
                self.assertIn(location, rule_ids_by_location, msg=output)
                self.assertIn(
                    "legacy-dashboard-bespoke-surface",
                    rule_ids_by_location[location],
                    msg=output,
                )
                self.assertNotIn(
                    "hardcoded-plugin-type-literal",
                    rule_ids_by_location[location],
                    msg=output,
                )

    def test_shell_report_all_mode_collects_without_changing_default_fail_fast(self) -> None:
        default_result = run_shell_checker("fail/shell_legacy_parity")
        default_output = default_result.stdout + default_result.stderr
        self.assertNotEqual(default_result.returncode, 0, msg=default_output)
        self.assertIn("dashboard-icons bespoke surface", default_output, msg=default_output)
        self.assertNotIn(
            "PluginTypeId semantic helper definitions",
            default_output,
            msg=default_output,
        )

        report_all_result = run_shell_checker("fail/shell_legacy_parity", report_all=True)
        report_all_output = report_all_result.stdout + report_all_result.stderr
        self.assertNotEqual(report_all_result.returncode, 0, msg=report_all_output)
        self.assertIn("dashboard-icons bespoke surface", report_all_output, msg=report_all_output)
        self.assertIn(
            "PluginTypeId semantic helper callsites/uses",
            report_all_output,
            msg=report_all_output,
        )
        self.assertIn(
            "PluginTypeId semantic helper definitions",
            report_all_output,
            msg=report_all_output,
        )
        self.assertIn("identity-specific helpers", report_all_output, msg=report_all_output)
        self.assertIn(
            "plugin_ids token references in non-plugin production code",
            report_all_output,
            msg=report_all_output,
        )
        self.assertIn("let _instance_display = id.display_name();", report_all_output, msg=report_all_output)

    def test_python_checker_maps_every_shell_report_all_finding_to_expected_rule_id(self) -> None:
        shell_result = run_shell_checker("fail/shell_legacy_parity", report_all=True)
        shell_output = shell_result.stdout + shell_result.stderr
        self.assertNotEqual(shell_result.returncode, 0, msg=shell_output)
        shell_findings = parse_shell_findings(shell_output)
        self.assertGreaterEqual(len(shell_findings), 6, msg=shell_output)

        python_result = run_checker("fail/shell_legacy_parity", output_format="json")
        python_output = python_result.stdout + python_result.stderr
        self.assertNotEqual(python_result.returncode, 0, msg=python_output)
        payload = json.loads(python_result.stdout)
        python_rule_ids_by_location = index_python_rule_ids_by_location(payload["findings"])
        expected_rule_by_shell_label = {
            "dashboard-icons bespoke surface": "legacy-dashboard-bespoke-surface",
            "PluginTypeId semantic helper callsites/uses": "forbidden-plugin-helper",
            "PluginTypeId semantic helper definitions": "forbidden-plugin-helper",
            "identity-specific helpers": "forbidden-plugin-helper",
            "plugin_ids token references in non-plugin production code": "plugin-ids-reference",
        }

        for label, path, line in shell_findings:
            with self.subTest(label=label, path=path, line=line):
                location = (path, line)
                self.assertIn(location, python_rule_ids_by_location, msg=python_output)
                self.assertIn(label, expected_rule_by_shell_label, msg=shell_output)
                self.assertIn(
                    expected_rule_by_shell_label[label],
                    python_rule_ids_by_location[location],
                    msg=python_output,
                )

    def test_rust_raw_payload_literals_are_detected_without_external_context(self) -> None:
        result = run_checker("fail/rust_raw_payload_literals")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("hardcoded-plugin-type-literal", output, msg=output)
        self.assertIn("crates/app/src/lib.rs", output, msg=output)
        self.assertIn("crates/app/src/raw_multiline.rs", output, msg=output)
        self.assertIn("match_value=releases_github", output, msg=output)
        self.assertIn("match_value=generic_shell", output, msg=output)

    def test_inline_comments_and_prose_do_not_trigger_false_positives(self) -> None:
        result = run_checker("pass/inline_comments_and_prose")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_nested_block_comments_do_not_leak_tokens_into_scan(self) -> None:
        all_rule_ids = {
            "plugin-core-import",
            "concrete-plugin-import",
            "plugin-ids-reference",
            "forbidden-plugin-helper",
            "hardcoded-plugin-type-literal",
            "manifest-plugin-dependency",
        }
        result = run_checker("fail/nested_block_comments", output_format="json")
        output = result.stdout + result.stderr
        self.assertRuleIds(
            result,
            {"plugin-ids-reference"},
            all_rule_ids - {"plugin-ids-reference"},
        )
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "plugin-ids-reference"
            and finding["path"] == "crates/app/src/lib.rs"
        ]
        self.assertEqual(1, len(findings), msg=output)
        self.assertEqual("plugin_ids::GENERIC_SHELL", findings[0]["match_value"], msg=output)
        self.assertEqual(10, findings[0]["line"], msg=output)

    def test_prose_error_string_with_plugin_type_token_does_not_trigger_context(self) -> None:
        result = run_checker("pass/prose_error_string_context")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_same_line_non_identity_suffix_names_do_not_trigger_context(self) -> None:
        result = run_checker("pass/same_line_non_identity_suffix")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_non_canonical_dashboard_tokens_do_not_trigger_hardcoded_plugin_type_literal(self) -> None:
        result = run_checker("pass/non_canonical_dashboard_tokens")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_adjacent_prose_string_is_not_tainted_by_neighbor_identity_literal(self) -> None:
        result = run_checker("fail/literal_adjacent_prose_context", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            finding
            for finding in payload["findings"]
            if finding["rule_id"] == "hardcoded-plugin-type-literal"
            and finding["path"] == "crates/app/src/lib.rs"
            and finding["match_value"] == "releases_github"
        ]
        self.assertEqual(1, len(findings), msg=output)
        self.assertIn('"plugin_type":"releases_github"', findings[0]["excerpt"], msg=output)

    def test_rust_string_literals_do_not_trigger_regex_rules(self) -> None:
        result = run_checker("pass/rust_string_literal_regex_rules")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_interpolated_template_literals_are_excluded_from_literal_scanning(self) -> None:
        result = run_checker("pass/interpolated_template_literals_excluded")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_allowlist_rejects_missing_reason_invalid_rule_invalid_kind_and_glob_paths(self) -> None:
        cases = [
            ("fail/allowlist_missing_reason", "missing key: reason"),
            ("fail/allowlist_bad_rule", "unknown rule_id"),
            ("fail/allowlist_bad_match_kind", "invalid match_kind"),
            ("fail/allowlist_glob_path", "glob patterns are not allowed"),
            ("fail/allowlist_regex_match_value", "regex-like patterns are not allowed"),
        ]
        for fixture_name, expected_message in cases:
            with self.subTest(fixture=fixture_name):
                result = run_checker(fixture_name)
                output = result.stdout + result.stderr
                self.assertEqual(result.returncode, 2, msg=output)
                self.assertIn(expected_message, output, msg=output)

    def test_allowlist_rejects_empty_checked_in_file(self) -> None:
        result = run_checker("fail/allowlist_empty")
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 2, msg=output)
        self.assertIn("allowlist entries must not be empty", output, msg=output)

    def test_checker_emits_spec_canonical_match_kinds(self) -> None:
        self.assertMatchKinds(
            "fail/imports",
            {"plugin-core-import": "import_path"},
        )
        self.assertMatchKinds(
            "fail/concrete_imports",
            {"concrete-plugin-import": "crate_name"},
        )
        self.assertMatchKinds(
            "fail/plugin_ids_reference",
            {"plugin-ids-reference": "module_token"},
        )
        self.assertMatchKinds(
            "fail/helper_definition",
            {"forbidden-plugin-helper": "api_name"},
        )
        self.assertMatchKinds(
            "fail/literals",
            {"hardcoded-plugin-type-literal": "literal_string"},
        )
        self.assertMatchKinds(
            "fail/manifests",
            {"manifest-plugin-dependency": "manifest_dependency"},
        )

    def test_frontend_path_based_exclusions_keep_fixtures_and_tests_directories_ignored(self) -> None:
        result = run_checker("pass")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_frontend_singular_story_file_is_excluded(self) -> None:
        result = run_checker("pass/frontend_singular_story_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_frontend_story_directories_are_excluded_from_production_scan(self) -> None:
        result = run_checker("pass/frontend_story_directory_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_rust_integration_tests_path_is_excluded_from_production_scan(self) -> None:
        result = run_checker("pass/integration_tests_path_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_src_docs_paths_remain_in_scope_and_scanned(self) -> None:
        result = run_checker("fail/src_docs_paths_scanned")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("plugin-ids-reference", output, msg=output)
        self.assertIn("hardcoded-plugin-type-literal", output, msg=output)
        self.assertIn("crates/app/src/docs/lib.rs", output, msg=output)
        self.assertIn("frontend/src/docs/lib.ts", output, msg=output)

    def test_test_only_manifests_are_excluded(self) -> None:
        result = run_checker("pass_test_only_manifests_excluded")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_standalone_cfg_test_items_are_excluded_from_production_scan(self) -> None:
        result = run_checker("pass/standalone_cfg_test_item_exclusion")
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)

    def test_mixed_cfg_target_manifests_are_scanned_but_pure_test_targets_are_excluded(self) -> None:
        result = run_checker("fail/manifest_mixed_cfg_targets")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        self.assertIn("manifest-plugin-dependency", output, msg=output)
        self.assertIn("crates/app/Cargo.toml", output, msg=output)
        self.assertIn("uptrakit-plugin-releases-github", output, msg=output)
        self.assertNotIn("uptrakit-plugin-package-manager-apt", output, msg=output)

    def test_target_set_misconfiguration_fails_closed(self) -> None:
        result = run_checker("fail/target_misconfigured")
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 2, msg=output)
        self.assertIn("target-set misconfiguration", output, msg=output)

    def test_target_set_misconfiguration_fails_closed_per_expected_slice(self) -> None:
        cases = [
            ("fail/target_set_missing_frontend", "frontend production target set matched 0 files"),
            ("fail/target_set_missing_rust", "rust production target set matched 0 files"),
            ("fail/target_set_missing_manifests", "manifest target set matched 0 files"),
            (
                "fail/target_set_unresolved_plugin_ids_all",
                "plugin_ids::ALL references unknown canonical plugin id constants: LEGACY_MIGRATION_ONLY",
            ),
        ]
        for fixture_name, expected_message in cases:
            with self.subTest(fixture=fixture_name):
                result = run_checker(fixture_name)
                output = result.stdout + result.stderr
                self.assertEqual(result.returncode, 2, msg=output)
                self.assertIn("target-set misconfiguration", output, msg=output)
                self.assertIn(expected_message, output, msg=output)

    def test_malformed_plugin_ids_use_tree_reports_stable_config_error(self) -> None:
        result = run_checker("fail/plugin_ids_malformed_use_tree")
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 2, msg=output)
        self.assertIn("semantic-boundary config error:", output, msg=output)
        self.assertIn("malformed use tree", output, msg=output)
        self.assertIn("crates/app/src/lib.rs", output, msg=output)
        self.assertIn("unmatched brace", output, msg=output)
        self.assertNotIn("Traceback", output, msg=output)

    def test_notification_plugin_error_transport_variant_in_non_plugin_code_is_rejected(self) -> None:
        result = run_checker("fail/plugin_transport_escape", output_format="json")
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, msg=output)
        payload = json.loads(result.stdout)
        findings = [
            f for f in payload["findings"]
            if f["rule_id"] == "plugin-transport-escape"
        ]
        self.assertTrue(len(findings) > 0, msg=f"Expected plugin-transport-escape findings\n{output}")
        values = {f["match_value"] for f in findings}
        self.assertTrue(
            any("SmtpNotConfigured" in v for v in values),
            msg=f"Expected SmtpNotConfigured in findings, got {values}\n{output}",
        )

    def test_non_transport_notification_error_is_not_flagged(self) -> None:
        result = run_checker("pass/plugin_transport_escape_ok", output_format="json")
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, msg=output)


if __name__ == "__main__":
    unittest.main()
