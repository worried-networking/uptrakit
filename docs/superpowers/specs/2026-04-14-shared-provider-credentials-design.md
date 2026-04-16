# Shared Provider Credentials Design

## Goal

Add V1 controller-wide GitHub provider defaults so GitHub-backed plugin configs
can fall back to one shared credential bundle instead of repeating
`auth_token` and `api_base_url` in every plugin config.

## Scope

### V1

- one global provider: `github`
- fields:
  - `auth_token`
  - `api_base_url`
- scope:
  - global only
- consumers:
  - explicitly opt-in plugin types only
  - current opt-in set: `releases_github`

### Out of scope

- per-tenant overrides
- arbitrary provider references from plugin configs
- generic cross-provider abstraction for every plugin family

## Current Codebase Baseline

### Storage and settings

- Global raw settings already exist and are the best V1 persistence seam.
- Encrypted global settings are re-encrypted in `crates/core/controller/src/reencrypt.rs`.
- Tenant-scoped `plugin_type_settings` remain for per-plugin behavior defaults, not cross-plugin shared credentials.

### Plugin config authoring after the extension-framework redesign

- The legacy `crates/shared/extension-framework` crate is gone.
- Plugin config forms are now authored via
  `PluginConfig::form_schema() -> Vec<FormFieldDescriptor>` in
  `crates/plugins/infrastructure/core/src/plugin_config.rs`.
- The descriptor types are defined in
  `crates/plugins/infrastructure/core/src/surface_form_authoring.rs` and
  consumed via the `crates/plugins/infrastructure/core/src/form_schema.rs`
  re-export.
- `GET /api/v1/plugin-types` converts plugin-native `FormFieldDescriptor`
  values into API DTO `uptrakit_web_api_types::plugin_configs::FieldDef` via
  `plugin_field_to_api_field(...)` in
  `crates/ui/web-api/src/routes/plugin_configs.rs`.

This matters because the shared-provider design must not refer to the removed
extension-framework crate or legacy symbols such as `ActionDef`, `FormDef`,
`FieldDef`, or `PanelPosition` as plugin-authoring types.

## Proposed Design

### Persistence model

Store GitHub provider defaults as global raw settings:

- `global_provider_github.auth_token`
- `global_provider_github.api_base_url`

Rules:

- keep the prefix as `global_provider_github.` with the trailing dot
- encrypt `auth_token` at rest
- store `api_base_url` as plain JSON string

### Shared-db module

Add `crates/shared/db/src/provider_settings.rs` with:

- `GlobalProviderSettingKey`
- `GitHubProviderDefaults`
- `load_github_provider_defaults(db)`
- `upsert_github_provider_defaults(db, auth_token, api_base_url)`
- `supports_github_provider_defaults(plugin_type_id)`
- `apply_github_provider_defaults_for_plugin(plugin_type_id, local, defaults)`

`supports_github_provider_defaults(...)` is the opt-in boundary. V1 should return `true` only for `plugin_ids::RELEASES_GITHUB`.

### Validation

Share `api_base_url` validation from a lightweight shared-types helper instead of putting it in `shared-db`.

Reason:

- `uptrakit-web-api-types` must validate the request DTO
- `uptrakit-web-api-types` should not depend on SeaORM-heavy `uptrakit-shared-db`

The shared helper should enforce the same practical contract as GitHub plugin config validation:

- valid URL
- `https` only
- host required
- reject obvious private/loopback literal hosts

If exact DNS-backed SSRF parity with plugin runtime validation is not
implemented at the DTO layer, the docs should say that provider-setting
validation is a lighter preflight and the plugin/runtime layer remains
authoritative.

### Secret handling

`auth_token` uses tri-state update semantics:

- omitted: keep current token
- `"***"`: keep current token
- `""`: clear token
- non-empty string: replace token

`api_base_url` semantics:

- omitted: keep current value
- `""`: clear
- non-empty string: replace after validation

### AAD and re-encryption

Use one canonical AAD constant for GitHub provider auth token encryption and reuse it from both runtime read/write code and `reencrypt.rs`.

The string must not be duplicated independently across files.

## Fallback Semantics

Field-level resolution order:

1. assignment override
2. plugin config row
3. global provider defaults
4. plugin built-in default

Fallback is field-level, not config-level:

- provider defaults fill only missing/blank provider fields
- unrelated config fields are untouched
- explicit non-empty local values always win

Blank handling for provider-backed fields:

- missing
- `null`
- `""`

All three are eligible for fallback.

## Materialization Paths

Provider defaults must be applied in every current GitHub config materialization path.

### Version checks and fetches

- `crates/ui/web-api/src/routes/software_items/version_check_dispatch.rs`
  - `VersionCheckContext::build_assignment(...)`
  - controller-fetch job builder extracted from `collect_and_run_controller_fetches(...)`
  - `dispatch_agent_version_checks(...)`
- `crates/ui/web-api/src/routes/software_items/mod.rs`
  - `classify_role_assignments(...)`
- `crates/shared/scheduler-engine/src/executors/fetch_releases.rs`
  - controller-fetch group builder extracted from `run_controller_side_fetch_releases(...)`
  - `send_agent_fetch_release_assignments(...)`

### Update dispatch

- `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
  - `build_plugin_assignment(...)`
  - `dispatch_update_to_agent(...)`
  - `load_target_for_dispatch(...)` fetch-releases config materialization for attestation enrichment
  - `load_role_plugins_ordered(...)` updated for the new helper signature, passing `None` for hook roles
- `crates/ui/web-api-queries/src/queries/update_triggers.rs`
  - `trigger_update_for_host(...)`
- batch paths:
  - `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs::dispatch_next_queued_for_host(...)`
  - `crates/ui/web-api-queries/src/queries/update_batches/mod.rs::create_batch(...)`

### Reconnect replay

- `crates/ui/web-api/src/routes/service_ws/handler/reconnect.rs`
  - `prepare_reconnect_replay(...)`
- `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
  - `prepare_reconnect_updates_on_connect(...)`
- `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
  - `prepare_pending_replay_messages(...)`
  - `recover_owned_updates_on_connect_with_dispatch_mode(...)`
  - `build_execute_payload(...)`
  - `build_plugin_assignment_nullable(...)`
  - replay `fetch_config` extraction used for `enrich_release_info_with_attestation(...)`

## API Surface

Add a controller-native provider settings endpoint:

- `GET /api/v1/global-settings/providers/github`
- `PUT /api/v1/global-settings/providers/github`

Permission:

- `manage_global_settings`

Use dedicated DTOs in `crates/shared/web-api-types/src/settings_provider_github.rs`.

## UI / UX

### Provider settings UI

Use dedicated provider-settings DTOs and routes. Do not route this through removed extension-framework constructs.

### Plugin help text

Update `crates/plugins/releases/github/src/config.rs` help text so GitHub
config fields explicitly explain that blank `auth_token` and `api_base_url`
fall back to the global provider default when configured.

This still goes through `PluginConfig::form_schema()` and `FormFieldDescriptor`, then reaches the API as `FieldDef` via route-boundary conversion.

## Testing

### Shared-db

- key round-trip
- field-level fallback
- opt-in guard
- encrypted-at-rest auth token persistence

### Controller

- re-encryption coverage for `global_provider_github.auth_token`

### Web API

- auth / permission coverage
- masking / keep / clear semantics
- DTO validation

### Materialization

Use pure builder helpers for controller-fetch paths so tests can assert merged config without executing real network fetches.

### Update dispatch

Cover:

- direct dispatch payload materialization
- queued/batch dispatch path
- replay payload path
- interactive-resolution path
- `load_target_for_dispatch(...)` fetch-releases config enrichment path

## Acceptance

The design is complete when:

- GitHub provider defaults are stored globally and encrypted correctly
- `releases_github` can omit `auth_token` and `api_base_url` and still receive effective values from the provider defaults
- every current GitHub materialization path uses the same fallback logic
- the provider settings API is permissioned and uses tri-state secret semantics
- docs and plugin help text match the current post-redesign form-authoring stack
- no spec or plan text refers to the removed extension-framework crate or legacy symbols as active architecture
