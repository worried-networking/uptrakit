"""Unit tests for ci/verify_no_orphan_modules.py.

Fixtures live in ci/testdata/no_orphan_modules/ — `pass` and `fail/*` plus
per-scenario `pass_*` trees. Each test copies a
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

    def test_char_literal_brace_does_not_corrupt_depth_tracking(self):
        # Finding 1: an inline module containing a `'{'` (and `b'{'`) char
        # literal must not shift inline-module depth tracking. Before the
        # fix, sanitize() left the literal's brace in the "sanitized" text,
        # so the subsequent `mod a;` resolved one directory too deep and the
        # gate reported a bogus resolver gap instead of exiting clean.
        result = run_gate(FIXTURES / "pass_char_literal_in_inline_mod")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no-orphan-modules clean", result.stdout)

    def test_prefixed_raw_strings_do_not_corrupt_depth_tracking(self):
        # Finding 6: sanitize() recognised `r"…"` but rejected the prefixed
        # raw forms `br"…"` / `cr"…"` (the identifier-boundary guard read the
        # `b`/`c` as an identifier char). Those literals then took the
        # escape-aware ordinary-string path, which stops at the first bare
        # `"` inside `br#"a"b{"#` — leaking the `{` into brace-depth tracking
        # — and runs past a trailing `\` into the following code.
        result = run_gate(FIXTURES / "pass_raw_string_prefixes")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no-orphan-modules clean", result.stdout)

    def test_path_attr_single_line_and_interleaved_cfg_resolve(self):
        # Finding 2: PATH_ATTR_RE previously required the `#[path]` attribute
        # to be immediately followed by the `mod` line on the very next
        # line. Both a single-line `#[path = "..."] mod x;` and an
        # interleaved `#[cfg(...)]` between `#[path]` and `mod` are valid
        # Rust that rustc accepts; the gate must resolve both explicitly
        # instead of falling back to (and failing on) the default filename.
        result = run_gate(FIXTURES / "pass_path_attr_shapes")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no-orphan-modules clean", result.stdout)

    def test_commented_out_include_does_not_mask_a_real_orphan(self):
        # Finding 5: INCLUDE_RE/PATH_ATTR_RE previously ran against raw
        # source, so a commented-out `include!("dead.rs")` still counted as
        # a visit and hid a genuinely orphaned dead.rs behind a fake clean
        # result.
        result = run_gate(FIXTURES / "fail" / "commented_out_include")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("orphan module: app/src/dead.rs", result.stderr)

    def test_stale_allowlist_decl_file_half_is_config_error(self):
        # Finding 3: a `decl` entry whose file half matches no tracked file
        # is the same hazard as a stale `path` entry (it silently
        # pre-authorizes a future resolver gap at that path) and must be
        # rejected the same way.
        with tempfile.NamedTemporaryFile(
            "w", suffix=".toml", delete=False
        ) as allowlist:
            allowlist.write(
                '[[allow]]\ndecl = "app/src/never_existed.rs::ghost"\n'
                'reason = "stale entry for a file that is gone"\n'
            )
            allowlist_path = allowlist.name
        result = run_gate(FIXTURES / "pass", ["--allowlist", allowlist_path])
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("matches no tracked file", result.stderr)

    def test_allowlist_entry_with_both_path_and_decl_is_config_error(self):
        # Finding 4: `if "path" ... elif "decl"` silently dropped the decl
        # half of an entry carrying both keys. It must be a config error
        # telling the author to split it into two entries.
        with tempfile.NamedTemporaryFile(
            "w", suffix=".toml", delete=False
        ) as allowlist:
            allowlist.write(
                '[[allow]]\npath = "app/src/proxy/bookkeeping.rs"\n'
                'decl = "app/src/proxy/bookkeeping.rs::ghost"\n'
                'reason = "both keys set — should be rejected"\n'
            )
            allowlist_path = allowlist.name
        result = run_gate(
            FIXTURES / "fail" / "same_stem_orphan",
            ["--allowlist", allowlist_path],
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("both", result.stderr)
        self.assertIn("path", result.stderr)
        self.assertIn("decl", result.stderr)

    def test_non_string_allowlist_value_is_config_error(self):
        # A malformed (non-string) `decl` must be reported as a config error
        # (exit 2). An uncaught AttributeError would exit 1 instead — the
        # gate's "orphan violations" code — misreporting a config mistake as
        # a code violation.
        with tempfile.NamedTemporaryFile(
            "w", suffix=".toml", delete=False
        ) as allowlist:
            allowlist.write(
                '[[allow]]\ndecl = 42\nreason = "non-string decl"\n'
            )
            allowlist_path = allowlist.name
        result = run_gate(FIXTURES / "pass", ["--allowlist", allowlist_path])
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("non-string `decl`", result.stderr)


if __name__ == "__main__":
    unittest.main()
