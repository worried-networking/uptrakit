"""Unit tests for ci/verify_no_orphan_modules.py.

Fixtures live in ci/testdata/no_orphan_modules/{pass,fail}. Each test copies a
fixture tree into a throwaway git repo (the gate enumerates candidates via
`git ls-files`, so fixture files must be tracked somewhere) and runs the gate
as a subprocess, asserting on exit code and message prefixes.
"""

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "verify_no_orphan_modules.py"
FIXTURES = ROOT / "testdata" / "no_orphan_modules"


def run_gate(fixture: Path, extra_args: list[str] | None = None):
    """Copy fixture into a fresh git repo, commit it, run the gate there."""
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp) / "repo"
        shutil.copytree(fixture, repo)
        subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
        subprocess.run(["git", "add", "-A"], cwd=repo, check=True)
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(repo), *(extra_args or [])],
            capture_output=True,
            text=True,
        )


class NoOrphanModulesGateTests(unittest.TestCase):
    def test_pass_tree_is_clean(self):
        # Covers: crate-root resolution (lib.rs + tests/*.rs targets),
        # foo.rs -> foo/ resolution, inline `mod b { mod c; }` -> b/c.rs,
        # tests/helpers/mod.rs reachable from an integration-test root.
        result = run_gate(FIXTURES / "pass")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no-orphan-modules clean", result.stdout)

    def test_same_stem_orphan_detected(self):
        # The historic incident shape: src/proxy/tests/bookkeeping.rs is live
        # (declared by src/proxy/tests.rs) while src/proxy/bookkeeping.rs is
        # an orphan. Stem matching would conflate them; directory-aware
        # resolution must flag exactly the orphan.
        result = run_gate(FIXTURES / "fail" / "same_stem_orphan")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("orphan module: app/src/proxy/bookkeeping.rs", result.stderr)
        self.assertNotIn("proxy/tests/bookkeeping.rs", result.stderr)

    def test_resolver_gap_is_loud_config_error(self):
        # A `mod ghost;` with no file must fail as Class B (exit 2), never
        # silently pass or masquerade as an orphan report.
        result = run_gate(FIXTURES / "fail" / "resolver_gap")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("resolver gap: `mod ghost;`", result.stderr)

    def test_allowlist_entry_without_reason_is_config_error(self):
        with tempfile.NamedTemporaryFile(
            "w", suffix=".toml", delete=False
        ) as allowlist:
            allowlist.write('[[allow]]\npath = "app/src/proxy/bookkeeping.rs"\n')
            allowlist_path = allowlist.name
        result = run_gate(
            FIXTURES / "fail" / "same_stem_orphan",
            ["--allowlist", allowlist_path],
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("missing a non-empty `reason`", result.stderr)

    def test_stale_allowlist_path_is_config_error(self):
        # A dead allowlist entry silently pre-authorizes a future orphan at
        # that path — the gate must reject it loudly instead.
        with tempfile.NamedTemporaryFile(
            "w", suffix=".toml", delete=False
        ) as allowlist:
            allowlist.write(
                '[[allow]]\npath = "app/src/never_existed.rs"\n'
                'reason = "stale entry for a file that is gone"\n'
            )
            allowlist_path = allowlist.name
        result = run_gate(FIXTURES / "pass", ["--allowlist", allowlist_path])
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("matches no tracked file", result.stderr)

    def test_allowlisted_orphan_passes(self):
        with tempfile.NamedTemporaryFile(
            "w", suffix=".toml", delete=False
        ) as allowlist:
            allowlist.write(
                '[[allow]]\npath = "app/src/proxy/bookkeeping.rs"\n'
                'reason = "fixture: intentionally orphaned for this test"\n'
            )
            allowlist_path = allowlist.name
        result = run_gate(
            FIXTURES / "fail" / "same_stem_orphan",
            ["--allowlist", allowlist_path],
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
