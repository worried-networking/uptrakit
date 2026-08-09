"""Unit tests for ci/verify_action_security_declarations.py's pure functions.

Drives the parser/checker functions directly on inline fixture strings — no
subprocess, no real repo files (mirrors the intent of
ci/test_check_plugin_semantic_boundary.py: fast, isolated, one assertion
group per rule).
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MODULE_PATH = ROOT / "verify_action_security_declarations.py"

_spec = importlib.util.spec_from_file_location(
    "verify_action_security_declarations", MODULE_PATH
)
assert _spec is not None and _spec.loader is not None
vasd = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vasd)


# A minimal but structurally faithful action_extractor! invocation.
ACTION_RS_FIXTURE = """
macro_rules! action_extractor {
    ($($name:ident => $action:expr),* $(,)?) => {};
}

action_extractor! {
    /// `hosts:read` — list/get hosts.
    CanReadHosts => actions::HOSTS_READ,
    /// `hosts:update` — edit host properties.
    CanUpdateHosts => actions::HOSTS_UPDATE,
}
"""

# A minimal but structurally faithful access_catalog! invocation.
CATALOG_RS_FIXTURE = """
access_catalog! {
    Hosts, "hosts", {
        Read => ("read", Host, HOSTS_READ, HOSTS_READ_STR, "View hosts"),
        Update => ("update", Host, HOSTS_UPDATE, HOSTS_UPDATE_STR, "Update host properties"),
    };
}
"""

# Same catalog, but the Read verb tuple is wrapped across three lines — the
# parser trap the whitespace-tolerant regex exists for.
CATALOG_RS_MULTILINE_FIXTURE = """
access_catalog! {
    Hosts, "hosts", {
        Read => (
            "read",
            Host,
            HOSTS_READ, HOSTS_READ_STR, "View hosts"
        ),
    };
}
"""

# Extractor-name -> "resource:verb" action-string map, as `main()` would
# derive it by joining _parse_extractor_map with _parse_catalog_map.
EXTRACTOR_ACTIONS = {
    "CanReadHosts": "hosts:read",
    "CanUpdateHosts": "hosts:update",
}

# Catalog action-string set, as `main()` would derive it from
# _parse_catalog_map(...).values().
CATALOG_ACTIONS = {"hosts:read", "hosts:update"}


class ParseExtractorMapTests(unittest.TestCase):
    def test_parses_name_to_const_pairs(self) -> None:
        result = vasd._parse_extractor_map(ACTION_RS_FIXTURE)
        self.assertEqual(
            result,
            {"CanReadHosts": "HOSTS_READ", "CanUpdateHosts": "HOSTS_UPDATE"},
        )

    def test_missing_macro_invocation_yields_empty_map(self) -> None:
        result = vasd._parse_extractor_map("// no action_extractor! here\n")
        self.assertEqual(result, {})


class ParseCatalogMapTests(unittest.TestCase):
    def test_parses_const_to_resource_verb_pairs(self) -> None:
        result = vasd._parse_catalog_map(CATALOG_RS_FIXTURE)
        self.assertEqual(
            result,
            {"HOSTS_READ": "hosts:read", "HOSTS_UPDATE": "hosts:update"},
        )

    def test_verb_tuple_wrapped_across_three_lines_still_parses(self) -> None:
        # Parser trap: a plain line-based regex would silently drop this
        # tuple entirely once rustfmt wraps it across lines.
        result = vasd._parse_catalog_map(CATALOG_RS_MULTILINE_FIXTURE)
        self.assertEqual(result, {"HOSTS_READ": "hosts:read"})

    def test_missing_macro_invocation_yields_empty_map(self) -> None:
        result = vasd._parse_catalog_map("// no access_catalog! here\n")
        self.assertEqual(result, {})


class CheckFilePassTests(unittest.TestCase):
    def test_consistent_operation_is_clean(self) -> None:
        source = """
use crate::middleware::action::CanReadHosts;

#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    tag = "Hosts",
    security(("oauth2" = ["hosts:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(CanReadHosts(_user): CanReadHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/hosts.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(violations, [])
        self.assertEqual(converted, 1)


class CheckFileR1Tests(unittest.TestCase):
    def test_r1_scopes_do_not_match_used_extractor(self) -> None:
        # Direction 1: oauth2 scopes declared, but the handler's extractor
        # set does not equal exactly those scopes.
        source = """
use crate::middleware::action::CanUpdateHosts;

#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    tag = "Hosts",
    security(("oauth2" = ["hosts:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(CanUpdateHosts(_user): CanUpdateHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/hosts.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(len(violations), 1)
        self.assertIn("R1", violations[0])
        self.assertIn("['hosts:read']", violations[0])
        self.assertIn("['hosts:update']", violations[0])
        self.assertEqual(converted, 1)

    def test_r1_extractor_used_without_oauth2_declaration(self) -> None:
        # Direction 2: the handler uses an action extractor, but the attr
        # declares no oauth2 requirement at all.
        source = """
use crate::middleware::action::CanReadHosts;

#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    tag = "Hosts"
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(CanReadHosts(_user): CanReadHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/hosts.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(len(violations), 1)
        self.assertIn("R1", violations[0])
        self.assertIn("declares no oauth2 requirement", violations[0])
        self.assertEqual(converted, 0)


class CheckFileR2Tests(unittest.TestCase):
    def test_empty_oauth2_scope_with_extractor_still_used(self) -> None:
        source = """
use crate::middleware::action::CanReadHosts;

#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    tag = "Hosts",
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(CanReadHosts(_user): CanReadHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/hosts.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(len(violations), 1)
        self.assertIn("R2", violations[0])
        self.assertIn("must not use action extractors", violations[0])
        self.assertEqual(converted, 1)


class CheckFileR3Tests(unittest.TestCase):
    def test_mixed_x_required_permission_and_oauth2_in_converted_file(self) -> None:
        source = """
use crate::middleware::action::CanReadHosts;

#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    tag = "Hosts",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("oauth2" = ["hosts:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(CanReadHosts(_user): CanReadHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/hosts.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        r3 = [v for v in violations if "R3" in v]
        self.assertEqual(len(r3), 1)
        self.assertIn("retired x-required-permission extension", r3[0])
        self.assertEqual(converted, 0)

    def test_mixed_x_required_permission_and_oauth2_in_legacy_file(self) -> None:
        # R3 is the one rule enforced even when the file has NOT imported
        # middleware::action — mixed-world declarations are always wrong.
        source = """
use crate::middleware::permission::CanViewHosts;

#[utoipa::path(
    get,
    path = "/api/v1/host-tags",
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("oauth2" = ["hosts:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_host_tags(CanViewHosts(_user): CanViewHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/host_tags.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(len(violations), 1)
        self.assertIn("R3", violations[0])
        self.assertEqual(converted, 0)


class CheckFileR4Tests(unittest.TestCase):
    def test_oauth2_requirement_without_developer_token_pairing(self) -> None:
        source = """
use crate::middleware::action::CanReadHosts;

#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    tag = "Hosts",
    security(("oauth2" = ["hosts:read"]))
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(CanReadHosts(_user): CanReadHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/hosts.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(len(violations), 1)
        self.assertIn("R4", violations[0])
        self.assertIn("without developer_token pairing", violations[0])
        self.assertEqual(converted, 1)


class CheckFileNonVacuityTests(unittest.TestCase):
    def test_empty_extractor_map_from_missing_macro(self) -> None:
        result = vasd._parse_extractor_map("// nothing here\n")
        self.assertEqual(result, {})

    def test_empty_catalog_map_from_missing_macro(self) -> None:
        result = vasd._parse_catalog_map("// nothing here\n")
        self.assertEqual(result, {})

    def test_zero_converted_operations_in_legacy_only_file(self) -> None:
        # A file that never imports middleware::action still fails on the
        # retired extension (M1.8 ban) and contributes zero converted
        # operations.
        source = """
use crate::middleware::permission::CanViewHosts;

#[utoipa::path(
    get,
    path = "/api/v1/host-tags",
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_host_tags(CanViewHosts(_user): CanViewHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/host_tags.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        self.assertEqual(len(violations), 1)
        self.assertIn("retired x-required-permission extension", violations[0])
        self.assertEqual(converted, 0)


class ImportGatingTests(unittest.TestCase):
    def test_legacy_file_using_can_update_hosts_without_action_import_fails_only_r3(self) -> None:
        # Parser trap: CanUpdateHosts is defined in BOTH middleware::action
        # and middleware::permission. Without a middleware::action import,
        # this must produce zero R1 violations — the import line is what
        # disambiguates, not the bare identifier.
        source = """
use crate::middleware::permission::CanUpdateHosts;

#[utoipa::path(
    put,
    path = "/api/v1/host-tags/{id}",
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_host_tag(CanUpdateHosts(_user): CanUpdateHosts) -> Response {
    todo!()
}
"""
        violations, converted = vasd._check_file("routes/host_tags.rs", source, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)
        r1 = [v for v in violations if "R1" in v]
        self.assertEqual(r1, [])
        self.assertEqual(len(violations), 1)
        self.assertIn("retired x-required-permission extension", violations[0])
        self.assertEqual(converted, 0)


class OAuth2GroupsTests(unittest.TestCase):
    def test_none_when_no_oauth2_key(self) -> None:
        self.assertIsNone(vasd._oauth2_groups('security(("bearer_token" = []))'))

    def test_single_empty_group(self) -> None:
        self.assertEqual(vasd._oauth2_groups('security(("oauth2" = []))'), [[]])

    def test_scopes_extracted_in_order(self) -> None:
        self.assertEqual(
            vasd._oauth2_groups('security(("oauth2" = ["hosts:read", "hosts:update"]))'),
            [["hosts:read", "hosts:update"]],
        )

    def test_multiple_groups_in_declaration_order(self) -> None:
        self.assertEqual(
            vasd._oauth2_groups(
                'security(("oauth2" = ["hosts:read"]), ("oauth2" = ["hosts:update"]), ("developer_token" = []))'
            ),
            [["hosts:read"], ["hosts:update"]],
        )


class CheckFileR5Tests(unittest.TestCase):
    OR_OP_TEMPLATE = """
use crate::middleware::action::{{AccessAuthority, authorize_any}};

#[utoipa::path(
    post,
    path = "/api/v1/hosts/batch",
    tag = "Hosts",
    security({security})
)]
#[tracing::instrument(skip_all)]
pub async fn batch_hosts(
    State(state): State<Arc<AppState>>,
    Extension(authority): Extension<AccessAuthority>,
    Json(body): Json<BatchActionRequest>,
) -> Response {{
    authorize_any(&state.access_engine, &ctx, required_actions);
    todo!()
}}
"""

    def _check(self, security: str, source: str | None = None):
        src = source if source is not None else self.OR_OP_TEMPLATE.format(security=security)
        return vasd._check_file("routes/hosts.rs", src, EXTRACTOR_ACTIONS, CATALOG_ACTIONS)

    def test_clean_or_operation(self) -> None:
        violations, converted = self._check(
            '("oauth2" = ["hosts:read"]), ("oauth2" = ["hosts:update"]), ("developer_token" = [])'
        )
        self.assertEqual(violations, [])
        self.assertEqual(converted, 1)

    def test_or_operation_with_extractor_is_flagged(self) -> None:
        source = """
use crate::middleware::action::{AccessAuthority, CanReadHosts, authorize_any};

#[utoipa::path(
    post,
    path = "/api/v1/hosts/batch",
    tag = "Hosts",
    security(("oauth2" = ["hosts:read"]), ("oauth2" = ["hosts:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_hosts(
    CanReadHosts(_user): CanReadHosts,
    Extension(authority): Extension<AccessAuthority>,
) -> Response {
    authorize_any(&engine, &ctx, actions);
    todo!()
}
"""
        violations, _ = self._check("", source=source)
        self.assertTrue(any("must not use action extractors" in v for v in violations))

    def test_multi_scope_alternative_is_flagged(self) -> None:
        violations, _ = self._check(
            '("oauth2" = ["hosts:read", "hosts:update"]), ("oauth2" = ["hosts:update"]), ("developer_token" = [])'
        )
        self.assertTrue(any("exactly one scope" in v for v in violations))

    def test_off_catalog_scope_is_flagged(self) -> None:
        violations, _ = self._check(
            '("oauth2" = ["hosts:read"]), ("oauth2" = ["hosts:reed"]), ("developer_token" = [])'
        )
        self.assertTrue(any("not in the action catalog" in v for v in violations))

    def test_duplicate_alternatives_are_flagged(self) -> None:
        violations, _ = self._check(
            '("oauth2" = ["hosts:read"]), ("oauth2" = ["hosts:read"]), ("developer_token" = [])'
        )
        self.assertTrue(any("duplicate OR alternatives" in v for v in violations))

    def test_or_without_access_authority_is_flagged(self) -> None:
        source = """
use crate::middleware::action::authorize_any;

#[utoipa::path(
    post,
    path = "/api/v1/hosts/batch",
    tag = "Hosts",
    security(("oauth2" = ["hosts:read"]), ("oauth2" = ["hosts:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_hosts(State(state): State<Arc<AppState>>) -> Response {
    authorize_any(&engine, &ctx, actions);
    todo!()
}
"""
        violations, _ = self._check("", source=source)
        self.assertTrue(any("Extension<AccessAuthority>" in v for v in violations))

    def test_dynamic_operation_with_scoped_group_is_flagged(self) -> None:
        source = """
use crate::middleware::action::AccessAuthority;

#[utoipa::path(
    get,
    path = "/api/v1/surfaces/{surface_id}",
    tag = "Surfaces",
    extensions(("x-action-dynamic" = json!(true))),
    security(("oauth2" = ["hosts:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_surface_read(Extension(authority): Extension<AccessAuthority>) -> Response {
    todo!()
}
"""
        violations, _ = self._check("", source=source)
        self.assertTrue(any("x-action-dynamic" in v for v in violations))


if __name__ == "__main__":
    unittest.main()
