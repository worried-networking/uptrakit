"""Unit tests for ci/bare_crate_select.py's pure selection functions.

Drives `select_affected_plugin_crates` directly against a fabricated, minimal
`cargo metadata --format-version 1`-shaped fixture — no subprocess, no real git/cargo calls
(mirrors ci/test_verify_action_security_declarations.py: fast, isolated, direct-import pattern).

Fixture workspace shape:
  crates/shared/types                       (uptrakit-shared-types)      — non-plugin
  crates/shared/test-support                (uptrakit-test-support)      — non-plugin, dev-dep only
  crates/core/agent                         (uptrakit-agent)             — non-plugin
  crates/plugins/infrastructure/core        (uptrakit-plugin-infra-core) — plugin, deps on shared-types
  crates/plugins/infrastructure/proxmox     (uptrakit-plugin-proxmox)    — plugin, deps on infra-core
  crates/plugins/generic/shell              (uptrakit-plugin-shell)     — plugin, dev-deps on test-support
"""

from __future__ import annotations

import copy
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MODULE_PATH = ROOT / "bare_crate_select.py"

_spec = importlib.util.spec_from_file_location("bare_crate_select", MODULE_PATH)
assert _spec is not None and _spec.loader is not None
bcs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(bcs)


WORKSPACE_ROOT = "/repo"

SHARED_TYPES = "shared-types-id"
TEST_SUPPORT = "test-support-id"
AGENT = "agent-id"
INFRA_CORE = "infra-core-id"
PROXMOX = "proxmox-id"
SHELL = "shell-plugin-id"
EXTERNAL_SERDE = "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0"


def _package(pkg_id: str, name: str, rel_manifest: str) -> dict:
    return {
        "id": pkg_id,
        "name": name,
        "manifest_path": f"{WORKSPACE_ROOT}/{rel_manifest}",
    }


def _node(pkg_id: str, deps: list[tuple[str, str | None]]) -> dict:
    return {
        "id": pkg_id,
        "deps": [
            {"pkg": dep_id, "dep_kinds": [{"kind": kind, "target": None}]}
            for dep_id, kind in deps
        ],
    }


_BASE_METADATA = {
    "workspace_root": WORKSPACE_ROOT,
    "workspace_members": [
        SHARED_TYPES,
        TEST_SUPPORT,
        AGENT,
        INFRA_CORE,
        PROXMOX,
        SHELL,
    ],
    "packages": [
        _package(SHARED_TYPES, "uptrakit-shared-types", "crates/shared/types/Cargo.toml"),
        _package(TEST_SUPPORT, "uptrakit-test-support", "crates/shared/test-support/Cargo.toml"),
        _package(AGENT, "uptrakit-agent", "crates/core/agent/Cargo.toml"),
        _package(
            INFRA_CORE,
            "uptrakit-plugin-infra-core",
            "crates/plugins/infrastructure/core/Cargo.toml",
        ),
        _package(
            PROXMOX, "uptrakit-plugin-proxmox", "crates/plugins/infrastructure/proxmox/Cargo.toml"
        ),
        _package(SHELL, "uptrakit-plugin-shell", "crates/plugins/generic/shell/Cargo.toml"),
        # A non-workspace (external) package present in `packages` but absent from
        # `workspace_members` — must never leak into selection or the dependency closure.
        {
            "id": EXTERNAL_SERDE,
            "name": "serde",
            "manifest_path": "/registry/serde-1.0.0/Cargo.toml",
        },
    ],
    "resolve": {
        "nodes": [
            _node(SHARED_TYPES, []),
            _node(TEST_SUPPORT, []),
            _node(AGENT, [(SHARED_TYPES, None)]),
            _node(INFRA_CORE, [(SHARED_TYPES, None), (EXTERNAL_SERDE, None)]),
            _node(PROXMOX, [(INFRA_CORE, None)]),
            _node(SHELL, [(TEST_SUPPORT, "dev")]),
            _node(EXTERNAL_SERDE, []),
        ]
    },
}


def metadata_fixture() -> dict:
    """Fresh deep copy of the fabricated fixture, so tests can't leak mutations."""
    return copy.deepcopy(_BASE_METADATA)


PLUGIN_NAMES = {"uptrakit-plugin-infra-core", "uptrakit-plugin-proxmox", "uptrakit-plugin-shell"}


class SelectAffectedPluginCratesTests(unittest.TestCase):
    def test_direct_plugin_src_change_selects_only_that_plugin(self):
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/plugins/generic/shell/src/lib.rs"]
        )
        self.assertEqual(selected, {"uptrakit-plugin-shell"})

    def test_infra_core_src_change_selects_infra_core_and_dependent_proxmox(self):
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/plugins/infrastructure/core/src/lib.rs"]
        )
        self.assertEqual(selected, {"uptrakit-plugin-infra-core", "uptrakit-plugin-proxmox"})
        self.assertNotIn("uptrakit-plugin-shell", selected)

    def test_shared_types_change_selects_all_transitive_dependents(self):
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/shared/types/src/lib.rs"]
        )
        self.assertEqual(selected, {"uptrakit-plugin-infra-core", "uptrakit-plugin-proxmox"})
        self.assertNotIn("uptrakit-plugin-shell", selected)

    def test_cargo_lock_change_selects_all_plugin_crates(self):
        selected = bcs.select_affected_plugin_crates(metadata_fixture(), ["Cargo.lock"])
        self.assertEqual(selected, PLUGIN_NAMES)

    def test_root_cargo_toml_change_selects_all_plugin_crates(self):
        selected = bcs.select_affected_plugin_crates(metadata_fixture(), ["Cargo.toml"])
        self.assertEqual(selected, PLUGIN_NAMES)

    def test_docs_only_change_selects_nothing(self):
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["docs/development/quality-gates.md", "README.md"]
        )
        self.assertEqual(selected, set())

    def test_plugin_manifest_change_selects_that_plugin(self):
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/plugins/generic/shell/Cargo.toml"]
        )
        self.assertEqual(selected, {"uptrakit-plugin-shell"})

    def test_deleted_file_path_in_crate_dir_still_counts_as_a_change(self):
        # The core function only ever sees a path string — it never stats the filesystem — so a
        # deleted file's path must be treated exactly like any other changed path.
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/plugins/generic/shell/src/removed_module.rs"]
        )
        self.assertEqual(selected, {"uptrakit-plugin-shell"})

    def test_dev_dependency_edge_selects_dependent_plugin(self):
        # test-support is only ever a dev-dependency (of the shell plugin); changing it must still
        # select the plugin, since the closure spans normal + dev + build dependency kinds.
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/shared/test-support/src/lib.rs"]
        )
        self.assertEqual(selected, {"uptrakit-plugin-shell"})

    def test_unrelated_non_plugin_member_change_selects_nothing(self):
        selected = bcs.select_affected_plugin_crates(
            metadata_fixture(), ["crates/core/agent/src/main.rs"]
        )
        self.assertEqual(selected, set())


if __name__ == "__main__":
    unittest.main()
