# Shared Provider Credentials Implementation Plan

> Superseded by
> `2026-04-17-global-github-provider-for-global-plugins.md`.
> This plan is retained as historical context for the earlier
> cross-plugin fallback design and no longer matches the implementation
> carried in this branch.
>
> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans` to implement this task-by-task.

**Goal:** historical V1 plan for global GitHub provider defaults threaded
through regular GitHub config materialization paths.

**Architecture:** historical fallback-based architecture, superseded by the
later global-plugin-only provider design.

## File Map

### Shared provider storage and validation

- Create: [crates/shared/db/src/provider_settings.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/db/src/provider_settings.rs)
- Create: [crates/shared/types/src/provider_validation.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/provider_validation.rs)
- Modify: [crates/shared/types/Cargo.toml](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/Cargo.toml)
- Modify: [crates/shared/types/src/lib.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/lib.rs)
- Modify: [crates/shared/db/src/lib.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/db/src/lib.rs)
- Modify: [crates/core/controller/src/reencrypt.rs](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/reencrypt.rs)

### API / client / CLI

- Create: [crates/shared/web-api-types/src/settings_provider_github.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/settings_provider_github.rs)
- Modify: [crates/shared/web-api-types/src/lib.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/lib.rs)
- Create: [crates/shared/openapi-client/src/settings_provider_github.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/openapi-client/src/settings_provider_github.rs)
- Modify: [crates/shared/openapi-client/src/lib.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/openapi-client/src/lib.rs)
- Modify: [crates/shared/openapi-client/src/paths.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/openapi-client/src/paths.rs)
- Create: [crates/ui/cli/src/commands/settings/provider_github.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/cli/src/commands/settings/provider_github.rs)
- Modify: [crates/ui/cli/src/commands/settings/mod.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/cli/src/commands/settings/mod.rs)
- Modify: [crates/ui/cli/src/tests.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/cli/src/tests.rs)

### Controller route and policy wiring

- Create: [crates/ui/web-api/src/routes/settings_provider_github.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/settings_provider_github.rs)
- Modify: [crates/ui/web-api/src/routes/mod.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/mod.rs)
- Modify: [crates/ui/web-api/src/router.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/router.rs)
- Modify: [crates/ui/web-api/src/settings.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/settings.rs)
- Modify: [crates/ui/web-api/db_access_policy.toml](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/db_access_policy.toml)
- Modify: [crates/ui/web-api/src/integration_tests/settings.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/integration_tests/settings.rs)

### Version checks, fetches, dispatch, replay

- Modify: [crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs)
- Modify: [crates/ui/web-api/src/routes/software_items/mod.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/software_items/mod.rs)
- Modify: [crates/shared/scheduler-engine/src/executors/fetch_releases.rs](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors/fetch_releases.rs)
- Modify: [crates/ui/web-api-queries/src/queries/update_dispatch.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_dispatch.rs)
- Modify: [crates/ui/web-api-queries/src/queries/update_triggers.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_triggers.rs)
- Modify: [crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs)
- Modify: [crates/ui/web-api-queries/src/queries/update_batches/mod.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/update_batches/mod.rs)
- Modify: [crates/ui/web-api/src/routes/service_ws/handler/reconnect.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/service_ws/handler/reconnect.rs)
- Modify: [crates/ui/web-api/src/routes/service_ws/handler/mod.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/service_ws/handler/mod.rs)
- Modify: [crates/ui/web-api/src/routes/service_ws/handler/updates.rs](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/service_ws/handler/updates.rs)

### Plugin UX and docs

- Modify: [crates/plugins/releases/github/src/config.rs](/Users/andreyyantsen/Development/uptrakit/crates/plugins/releases/github/src/config.rs)
- Modify: [docs/api/http-web-api.md](/Users/andreyyantsen/Development/uptrakit/docs/api/http-web-api.md)
- Modify: [docs/end-user/plugin-configs.md](/Users/andreyyantsen/Development/uptrakit/docs/end-user/plugin-configs.md)
- Modify: [docs/development/plugin-guidelines.md](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-guidelines.md)

## Task 1: Shared Provider Settings Foundation

- Add `provider_settings.rs` with:
  - `GlobalProviderSettingKey`
  - `GitHubProviderDefaults`
  - `load_github_provider_defaults(...)`
  - `upsert_github_provider_defaults(...)`
  - `supports_github_provider_defaults(...)`
  - `apply_github_provider_defaults_for_plugin(...)`
- Add shared URL validation in `shared-types`, not `shared-db`.
- Keep `GITHUB_PROVIDER_PREFIX` as `global_provider_github.`.
- Define the GitHub auth-token AAD once and reuse it from `reencrypt.rs`.
- Add shared-db tests for:
  - key round-trip
  - blank/missing field fallback
  - non-opt-in plugin bypass
  - encrypted-at-rest auth token persistence
- Add controller re-encryption coverage for `global_provider_github.auth_token`.

Verification:

```bash
cargo test -p uptrakit-shared-db --features migration,db-sqlite provider_settings -- --nocapture
cargo test -p uptrakit-controller github_provider_setting_gets_upgraded -- --nocapture
```

## Task 2: Provider Settings HTTP Contract, Client, and CLI

- Add dedicated provider settings DTOs in `web-api-types`.
- `UpdateGitHubProviderSettingsRequest` must implement `Validate`.
- Use the shared `provider_validation` helper for `api_base_url`.
- Add OpenAPI client support.
- Add CLI `settings provider-github show|set|clear`.
- Keep the parser test aligned with the current CLI shape:
  - `args.command` is `Option<Commands>`
  - use `Some(Commands::Settings { ... })`

Verification:

```bash
cargo test -p uptrakit-web-api-types settings_provider_github -- --nocapture
cargo test -p uptrakit-openapi-client settings_provider_github -- --nocapture
cargo test -p uptrakit-cli settings_provider_github_show_parses -- --nocapture
```

## Task 3: Controller Route and Permissions

- Add `GET` / `PUT /api/v1/global-settings/providers/github`.
- Use `manage_global_settings`.
- Return masked secret state and support tri-state token updates.
- Register the route and classify it in `db_access_policy.toml`.
- Reconcile the current baseline policy drift in the same file as part of this
  task before treating `verify_db_access_policy.py` as a feature gate. Current
  baseline issues already include stale `service_ws` entries and
  non-`service_ws` gaps such as
  `plugin_configs.rs::load_active_agent_service_for_host`.
- Extend `warn_unrecognised_keys(...)` to trust `GlobalProviderSettingKey`.
  Use `GlobalProviderSettingKey::from_db_key(key).is_some()` for exact-match allowlisting, consistent with `SettingKey`.
- In web-api integration tests:
  - use `register_and_get_token(...)` for a real authenticated user
  - use a directly minted reduced-permission JWT for the 403 case on
    bearer-token routes, because a freshly minted test JWT will decode
    correctly, will not appear in the test-instance denylist, and will
    therefore reach the permission check and return 403

Verification:

```bash
cargo test -p uptrakit-web-api --features db-sqlite github_provider_settings_forbids_missing_permission -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite github_provider_settings_round_trip_masks_keeps_and_clears_token -- --nocapture
python3 ci/verify_db_access_policy.py
```

## Task 4: Version Check and Fetch Materialization

- Load provider defaults once per top-level operation.
- Thread them into synchronous helpers instead of re-querying per assignment.
- Add `Option<&GitHubProviderDefaults>` where needed:
  - `VersionCheckContext::build_assignment(...)`
  - `classify_role_assignments(...)`
- Extract pure pre-execution builders so controller-fetch paths are testable without real upstream calls:
  - builder from `collect_and_run_controller_fetches(...)`
  - builder from `run_controller_side_fetch_releases(...)`
- Keep `is_controller_fetch_site(...)` routing logic based on existing
  resolved config semantics; do not let provider credential fallback affect
  execution-site classification.
- Use the existing `NoopSchedulerNotifier` fixture in scheduler-engine tests.

Verification:

```bash
cargo test -p uptrakit-web-api --features db-sqlite version_check_context_build_assignment_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite build_controller_fetch_jobs_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite dispatch_agent_version_checks_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite classify_role_assignments_applies_github_provider_defaults_for_single_host_checks -- --nocapture
cargo test -p uptrakit-scheduler-engine build_controller_fetch_groups_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-scheduler-engine send_agent_fetch_release_assignments_applies_github_provider_defaults -- --nocapture
```

## Task 5: Update Dispatch, Batch Dispatch, and Replay

- Update `build_plugin_assignment(...)` to accept provider defaults.
- Update `dispatch_update_to_agent(...)` to accept provider defaults and thread
  them to both internal `build_plugin_assignment(...)` call sites.
- Update `load_target_for_dispatch(...)` so the optional
  `fetch_releases_config` used for attestation enrichment also receives
  provider-default materialization.
- Update `load_role_plugins_ordered(...)` for the new signature, passing `None` for hook roles.
- Thread defaults through:
  - `trigger_update_for_host(...)`
  - `dispatch_next_queued_for_host(...)`
  - `create_batch(...)`
  - `prepare_reconnect_replay(...)`
  - `prepare_reconnect_updates_on_connect(...)`
  - `prepare_pending_replay_messages(...)`
  - `recover_owned_updates_on_connect_with_dispatch_mode(...)` only if provider defaults need to be carried through replay preparation state
  - `build_execute_payload(...)`
  - `build_plugin_assignment_nullable(...)`
- Carry replay-loaded provider defaults through `PendingUpdateRecords` (or an
  equivalent replay-preparation carrier) so `build_execute_payload(...)` stays
  synchronous while using one load per reconnect preparation pass.
- Add regression coverage for:
  - direct dispatch payload materialization
  - queued/batch payload dispatch
  - interactive-resolution path
  - replay payload path
  - `load_target_for_dispatch(...)` fetch config enrichment path

Verification:

```bash
cargo test -p uptrakit-web-api-queries --features db-sqlite build_plugin_assignment_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite dispatch_update_to_agent_payload_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite dispatch_next_queued_for_host_payload_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite trigger_update_for_host_uses_provider_defaults_for_interactive_resolution -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite load_target_for_dispatch_applies_github_provider_defaults_to_fetch_config -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite build_execute_payload_replay_applies_github_provider_defaults -- --nocapture
```

## Task 6: GitHub Form UX and Documentation

- Update GitHub config help text in `GitHubConfig::form_schema()`.
- The post-redesign authoring type here is `FormFieldDescriptor`, not legacy extension-framework `FieldDef`.
- The API still exposes plugin form fields as
  `uptrakit_web_api_types::plugin_configs::FieldDef` after route-boundary
  conversion; the docs should reflect that distinction.
- Document:
  - provider settings endpoints
  - permission model
  - tri-state secret semantics
  - fallback order
  - explicit opt-in model for future provider consumers

Verification:

```bash
cargo test -p uptrakit-plugin-releases-github form_schema_mentions_provider_fallback_for_blank_credentials -- --nocapture
```

## Final Verification

```bash
cargo fmt --all
cargo test -p uptrakit-shared-db --features migration,db-sqlite provider_settings -- --nocapture
cargo test -p uptrakit-controller github_provider_setting_gets_upgraded -- --nocapture
cargo test -p uptrakit-web-api-types settings_provider_github -- --nocapture
cargo test -p uptrakit-openapi-client settings_provider_github -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite github_provider_settings_forbids_missing_permission -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite github_provider_settings_round_trip_masks_keeps_and_clears_token -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite version_check_context_build_assignment_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite build_controller_fetch_jobs_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite dispatch_agent_version_checks_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite classify_role_assignments_applies_github_provider_defaults_for_single_host_checks -- --nocapture
cargo test -p uptrakit-scheduler-engine build_controller_fetch_groups_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-scheduler-engine send_agent_fetch_release_assignments_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite build_plugin_assignment_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite dispatch_update_to_agent_payload_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite dispatch_next_queued_for_host_payload_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite trigger_update_for_host_uses_provider_defaults_for_interactive_resolution -- --nocapture
cargo test -p uptrakit-web-api-queries --features db-sqlite load_target_for_dispatch_applies_github_provider_defaults_to_fetch_config -- --nocapture
cargo test -p uptrakit-web-api --features db-sqlite build_execute_payload_replay_applies_github_provider_defaults -- --nocapture
cargo test -p uptrakit-plugin-releases-github form_schema_mentions_provider_fallback_for_blank_credentials -- --nocapture
cargo test -p uptrakit-cli settings_provider_github_show_parses -- --nocapture
python3 ci/verify_db_access_policy.py
```

## Alignment Notes

This plan is intentionally aligned to the post-redesign codebase:

- no references to the removed `crates/shared/extension-framework`
- plugin-side form authoring uses `FormFieldDescriptor`
- API-side form DTOs remain `FieldDef`
- current batch dispatch paths use `create_batch(...)`, not the old nonexistent `trigger_batch_updates(...)`
- reconnect replay currently starts from `prepare_reconnect_replay(...)` /
  `prepare_reconnect_updates_on_connect(...)`, not the removed
  `deliver_pending_updates(...)` path
