import importlib.util
import pathlib
import tempfile
import unittest


SCRIPT_PATH = pathlib.Path(__file__).with_name("verify_db_access_policy.py")
SPEC = importlib.util.spec_from_file_location("verify_db_access_policy", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class VerifyDbAccessPolicyTests(unittest.TestCase):
    def _extract_from_text(self, source: str) -> tuple[dict[str, str], set[str]]:
        with tempfile.NamedTemporaryFile("w+", suffix=".rs", delete=False) as handle:
            path = pathlib.Path(handle.name)
            handle.write(source)
            handle.flush()

        try:
            return MODULE.extract_handlers(path)
        finally:
            path.unlink(missing_ok=True)

    def test_cfg_test_module_does_not_swallow_following_runtime_item(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
pub async fn runtime_before() {}

#[cfg(test)]
mod tests {
    async fn helper() {}
}

pub async fn runtime_after() {}
"""
        )

        self.assertEqual(set(handlers), {"runtime_before", "runtime_after"})
        self.assertEqual(source_names, {"runtime_before", "runtime_after"})

    def test_test_only_names_remain_visible_for_stale_checks(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(test)]
mod tests {
    async fn helper() {}
}
"""
        )

        self.assertEqual(handlers, {})
        self.assertEqual(source_names, set())

    def test_runtime_handler_wins_on_name_collision_with_tests(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
pub async fn collide(
    State(state): State<DbState>,
) {}

#[cfg(test)]
mod tests {
    async fn collide() {}
}
"""
        )

        self.assertIn("collide", handlers)
        self.assertIn("State<DbState>", handlers["collide"])
        self.assertEqual(source_names, {"collide"})

    def test_cfg_that_can_exist_without_test_is_not_excluded(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(any(test, feature = "interactive"))]
pub async fn runtime_or_test() {}

#[cfg(all(test, feature = "db-sqlite"))]
pub async fn test_only() {}
"""
        )

        self.assertIn("runtime_or_test", handlers)
        self.assertIn("runtime_or_test", source_names)
        self.assertNotIn("test_only", handlers)
        self.assertIn("test_only", source_names)

    def test_braces_in_test_only_strings_and_comments_do_not_merge_items(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
pub async fn runtime_before() {}

#[cfg(test)]
mod tests {
    const JSON: &str = "{ not a real brace }";
    /* comment with } and { should not affect top-level splitting */
    async fn helper() {}
}

pub async fn runtime_after() {}
"""
        )

        self.assertEqual(set(handlers), {"runtime_before", "runtime_after"})
        self.assertEqual(source_names, {"runtime_before", "runtime_after"})

    def test_stacked_cfg_attrs_are_treated_conjunctively(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(any(test, feature = "interactive"))]
#[cfg(not(feature = "interactive"))]
pub async fn combined_test_only() {}
"""
        )

        self.assertNotIn("combined_test_only", handlers)
        self.assertIn("combined_test_only", source_names)

    def test_cfg_attr_is_not_treated_as_cfg(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg_attr(test, allow(dead_code))]
pub async fn runtime_handler() {}
"""
        )

        self.assertIn("runtime_handler", handlers)
        self.assertIn("runtime_handler", source_names)

    def test_comment_text_does_not_spoof_or_hide_cfg_detection(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
// #[cfg(test)]
pub async fn runtime_from_line_comment() {}

/*
 * #[cfg(any(test, feature = "fake"))]
 */
#[cfg(test)]
pub async fn actually_test_only() {}

pub async fn runtime_after_block_comment() {}
"""
        )

        self.assertIn("runtime_from_line_comment", handlers)
        self.assertIn("runtime_after_block_comment", handlers)
        self.assertNotIn("actually_test_only", handlers)
        self.assertEqual(
            source_names,
            {
                "runtime_from_line_comment",
                "actually_test_only",
                "runtime_after_block_comment",
            },
        )

    def test_source_names_track_async_items_not_sync_helpers(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
fn sync_helper() {}

#[cfg(test)]
mod tests {
    fn sync_test_helper() {}
    async fn async_test_helper() {}
}

pub async fn runtime_handler() {}
"""
        )

        self.assertEqual(set(handlers), {"runtime_handler"})
        self.assertEqual(source_names, {"runtime_handler"})

    def test_stale_detection_uses_runtime_handlers_for_non_ignore_entries(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(test)]
pub async fn runtime_handler() {}

#[cfg(test)]
mod tests {
    async fn helper() {}
}
"""
        )

        self.assertFalse(
            MODULE._policy_entry_exists(
                "runtime_handler",
                "tenant-agnostic",
                handlers,
                source_names,
            )
        )

    def test_ignore_entries_do_not_match_top_level_test_only_async_items(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(test)]
pub async fn ignored_helper() {}

#[cfg(test)]
mod tests {
    async fn nested_helper() {}
}
"""
        )

        self.assertFalse(
            MODULE._policy_entry_exists(
                "ignored_helper",
                "ignore",
                handlers,
                source_names,
            )
        )
        self.assertFalse(
            MODULE._policy_entry_exists(
                "nested_helper",
                "ignore",
                handlers,
                source_names,
            )
        )

    def test_ignore_stale_collision_does_not_match_nested_test_helper(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(test)]
mod tests {
    async fn collide() {}
}
"""
        )

        self.assertFalse(
            MODULE._policy_entry_exists(
                "collide",
                "ignore",
                handlers,
                source_names,
            )
        )

    def test_cfg_any_accepts_trailing_comma(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(any(test, feature = "interactive",))]
pub async fn runtime_or_test() {}
"""
        )

        self.assertIn("runtime_or_test", handlers)
        self.assertIn("runtime_or_test", source_names)

    def test_cfg_not_handles_composite_expressions(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(not(any(test, feature = "interactive")))]
pub async fn runtime_when_feature_disabled() {}
"""
        )

        self.assertIn("runtime_when_feature_disabled", handlers)
        self.assertIn("runtime_when_feature_disabled", source_names)

    def test_multiline_cfg_attribute_is_parsed(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(
    any(test, feature = "interactive")
)]
pub async fn runtime_or_test_multiline() {}
"""
        )

        self.assertIn("runtime_or_test_multiline", handlers)
        self.assertIn("runtime_or_test_multiline", source_names)

    def test_cfg_not_accepts_trailing_comma(self) -> None:
        handlers, source_names = self._extract_from_text(
            """
#[cfg(not(test,))]
pub async fn runtime_when_not_test() {}
"""
        )

        self.assertIn("runtime_when_not_test", handlers)
        self.assertIn("runtime_when_not_test", source_names)

    def test_cfg_string_literal_may_contain_closing_bracket(self) -> None:
        handlers, source_names = self._extract_from_text(
            r'''
#[cfg(feature = "literal]value")]
pub async fn runtime_with_bracket_literal() {}
'''
        )

        self.assertIn("runtime_with_bracket_literal", handlers)
        self.assertIn("runtime_with_bracket_literal", source_names)


if __name__ == "__main__":
    unittest.main()
