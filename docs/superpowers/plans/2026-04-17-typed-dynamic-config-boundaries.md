# Typed Dynamic Config Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tighten dynamic config/public JSON boundaries so object-shaped config is represented explicitly, finite patch fields get typed parsing at
the Rust boundary without changing their current wire contract, and SMTP settings snapshots stop being rebuilt from ad hoc `HashMap<String, Value>`
getter chains.

**Architecture:** Keep persistence storage JSON-backed, but move the Rust-side boundary to validated wrapper types and typed settings snapshots. The
first phase preserves external wire shapes for the named REST contracts while moving parsing, validation, and response construction onto typed Rust
structures. This plan runs after the plugin-extension typing track for `crates/plugins/notifications/email/src/surfaces.rs`; its email-surface work is
limited to settings/config shape cleanup on top of that earlier typed boundary.

**Tech Stack:** Rust workspace crates, `serde`, `serde_json`, web-api DTOs, email notification surfaces, shared settings store/raw settings helpers,
cargo package tests/checks

---

## File Structure

### Public request/response contracts

- Modify:
  [`crates/shared/web-api-types/src/notifications/channels.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/notifications/channels.rs)
  Responsibility: replace raw JSON-object config fields with typed object wrappers that preserve the current wire shape.
- Modify:
  [`crates/shared/web-api-types/src/software_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/software_items.rs)
  Responsibility: keep the existing `icon_url` omit/null/string wire shape but add typed patch parsing around it, wrap `config_override` object
  semantics, and explicitly document intentional dynamic `latest_release_metadata`.
- Modify any software-item query builder surfaced by:
  `rg -n "HostPluginRoleSummary|InvalidConfigOverride|config_override" crates/ui/web-api-queries/src/queries/software_items` Responsibility: keep
  response-side `config_override` conversion explicit so legacy non-object payloads fail through the existing invalid-override path instead of via an
  opaque serde mismatch.

### Settings snapshots

- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
  Responsibility: replace `smtp_from_*_map` and the per-field getter family with serde-driven typed snapshot deserialization.
- Modify:
  [`crates/ui/web-api/src/notifications/dispatcher.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/dispatcher.rs)
  Responsibility: consume typed SMTP settings state rather than rebuilding it from raw maps.
- Modify: [`crates/ui/web-api-auth/src/settings_store.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-auth/src/settings_store.rs)
  Responsibility: expose typed loading helpers on top of raw settings storage so callers stop rebuilding snapshots manually.
- Modify: [`crates/shared/db/src/raw_settings.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/db/src/raw_settings.rs) Responsibility:
  provide reusable typed load/decode helpers without removing raw JSON persistence.

### Verification commands

- `cargo fmt --all`
- `cargo test -p uptrakit-web-api-types`
- `cargo check -p uptrakit-web-api-types`
- `cargo test -p uptrakit-notification-plugin-email`
- `cargo check -p uptrakit-notification-plugin-email`
- `cargo check -p uptrakit-web-api`
- `rg -n "HashMap<String, serde_json::Value>|get_string\\(|get_port\\(|get_tls_mode\\("` `crates/plugins/notifications/email/src/surfaces.rs`
  `crates/ui/web-api/src/notifications/dispatcher.rs`
- `rg -n "load_typed_settings_by_prefix|load_typed_global_settings_by_prefix|decode_prefixed_settings"`
  `crates/plugins/notifications/email/src/surfaces.rs` `crates/ui/web-api/src/notifications/dispatcher.rs`
  `crates/ui/web-api-auth/src/settings_store.rs` `crates/shared/db/src/raw_settings.rs`

### Task 1: Introduce Typed JSON Object Wrappers For Notification Contracts

**Files:**

- Modify:
  [`crates/shared/web-api-types/src/notifications/channels.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/notifications/channels.rs)
- Test:
  [`crates/shared/web-api-types/src/notifications/channels.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/notifications/channels.rs)

- [ ] **Step 1: Add a failing round-trip test for the new object wrapper**

Add:

```rust
#[test]
fn notification_channel_config_object_round_trips_as_plain_json_object() {
    let config = JsonObjectMap::try_from(serde_json::json!({ "url": "https://example.com/hook" }))
        .expect("object config");
    let value = serde_json::to_value(&config).expect("serialize");
    assert_eq!(value, serde_json::json!({ "url": "https://example.com/hook" }));
}
```

- [ ] **Step 2: Run the targeted DTO test**

Run:

```bash
cargo test -p uptrakit-web-api-types notification_channel_config_object_round_trips_as_plain_json_object -- --exact
```

Expected: FAIL because `JsonObjectMap` does not exist yet.

- [ ] **Step 3: Add the object-only wrapper and use it in the three channel contracts**

Implement a wrapper like:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct JsonObjectMap(serde_json::Map<String, serde_json::Value>);

impl TryFrom<serde_json::Value> for JsonObjectMap {
    type Error = ValidationError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Object(map) => Ok(Self(map)),
            _ => Err(ValidationError {
                field: "config",
                message: "must be a JSON object".to_string(),
            }),
        }
    }
}
```

Then replace:

```rust
pub config: serde_json::Value
```

with:

```rust
pub config: JsonObjectMap
```

and:

```rust
pub config: Option<JsonObjectMap>
```

- [ ] **Step 4: Run the web-api-types package tests**

Run:

```bash
cargo test -p uptrakit-web-api-types
cargo check -p uptrakit-web-api-types
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/web-api-types/src/notifications/channels.rs
git commit -m "refactor: type notification channel config objects"
```

### Task 2: Replace `icon_url` Patch JSON And Tighten Software Item Overrides

**Files:**

- Modify:
  [`crates/shared/web-api-types/src/software_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/software_items.rs)
- Modify any software-item query builder surfaced by:
  `rg -n "HostPluginRoleSummary|InvalidConfigOverride|config_override" crates/ui/web-api-queries/src/queries/software_items`
- Test:
  [`crates/shared/web-api-types/src/software_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/software_items.rs)

- [ ] **Step 1: Add a failing typed `icon_url` patch-parsing test**

Add:

```rust
#[test]
fn update_software_item_icon_url_patch_parses_set_clear_and_keep() {
    assert!(matches!(
        IconUrlPatch::from_json(None).expect("keep"),
        IconUrlPatch::Keep
    ));
    assert!(matches!(
        IconUrlPatch::from_json(Some(&serde_json::Value::Null)).expect("clear"),
        IconUrlPatch::Clear
    ));
    assert!(matches!(
        IconUrlPatch::from_json(Some(&serde_json::json!("https://example.com/icon.png")))
            .expect("set"),
        IconUrlPatch::Set(url) if url == "https://example.com/icon.png"
    ));
}
```

- [ ] **Step 2: Run the targeted test**

Run:

```bash
cargo test -p uptrakit-web-api-types update_software_item_icon_url_patch_parses_set_clear_and_keep -- --exact
```

Expected: FAIL because `IconUrlPatch` does not exist yet.

- [ ] **Step 3: Keep the wire shape, add typed patch parsing, and tighten overrides**

Keep the public DTO field as:

```rust
pub icon_url: Option<serde_json::Value>
```

so the existing omit/null/string contract stays intact.

Add a typed parser next to the DTO:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum IconUrlPatch {
    Keep,
    Set(String),
    Clear,
}
```

with a helper like:

```rust
impl IconUrlPatch {
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, ValidationError> {
        // None => Keep, Null => Clear, String => Set, everything else => validation error
    }
}
```

Tighten the object-only `config_override` contracts in the same file with the same `JsonObjectMap` wrapper used for notification config:

```rust
pub config_override: Option<JsonObjectMap>
```

Apply that change to:

```rust
HostPluginRoleAssignment
UpdateHostAssignmentRequest
HostPluginRoleSummary
```

For the response-side `HostPluginRoleSummary` path, do not rely on a blind derive from arbitrary stored `serde_json::Value`. Update the query-layer
mapping so legacy non-object overrides still surface through the existing invalid-config-override behavior instead of a generic serde failure.

Keep `latest_release_metadata` intentionally dynamic and annotate that in the type docs:

```rust
/// Intentionally left dynamic: payload shape is plugin-defined at the REST boundary.
pub latest_release_metadata: Option<serde_json::Value>,
```

- [ ] **Step 4: Run the DTO suite**

Run:

```bash
cargo test -p uptrakit-web-api-types
```

Expected: PASS, including the existing icon URL validation tests adapted to the new patch parser and the `config_override` object-shape tests updated
to use the shared object wrapper.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/web-api-types/src/software_items.rs crates/ui/web-api-queries/src/queries/software_items
git commit -m "refactor: type software item patch boundaries"
```

### Task 3: Replace SMTP Map Rebuilds With Typed Settings Snapshots

**Files:**

- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
- Modify:
  [`crates/ui/web-api/src/notifications/dispatcher.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/dispatcher.rs)
- Modify: [`crates/ui/web-api-auth/src/settings_store.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-auth/src/settings_store.rs)
- Modify: [`crates/shared/db/src/raw_settings.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/db/src/raw_settings.rs)

- [ ] **Step 1: Add a failing typed snapshot test in email surfaces**

Add:

```rust
#[test]
fn smtp_settings_snapshot_deserializes_from_prefixed_map() {
    let map = HashMap::from([
        ("smtp.host".to_string(), serde_json::json!("mail.example.com")),
        ("smtp.port".to_string(), serde_json::json!(587)),
        ("smtp.tls_mode".to_string(), serde_json::json!("starttls")),
    ]);

    let snapshot = smtp_snapshot_from_prefixed_map("smtp.", &map).expect("snapshot");
    assert_eq!(snapshot.host.as_deref(), Some("mail.example.com"));
    assert_eq!(snapshot.port, Some(587));
}
```

- [ ] **Step 2: Run the package check/test to prove the helper is missing**

Run:

```bash
cargo test -p uptrakit-notification-plugin-email smtp_settings_snapshot_deserializes_from_prefixed_map -- --exact
```

Expected: FAIL because `smtp_snapshot_from_prefixed_map` does not exist.

- [ ] **Step 3: Add the shared typed-loading helper and replace getter chains**

Keep the ownership split explicit:

- `raw_settings.rs` owns the reusable prefix-to-typed decode primitive because plugin crates such as `uptrakit-notification-plugin-email` can depend
  on `uptrakit-shared-db` but not on `uptrakit-web-api-auth`.
- `settings_store.rs` owns auth-layer wrappers that call the raw helper and translate both DB and decode failures into the existing `AuthError`
  surface.
- `notifications/dispatcher.rs` should use the typed `settings_store` wrappers.
- `email/src/surfaces.rs` can call the raw helper directly because it lives below the auth layer.

In `raw_settings.rs`, extend the existing `RawSettingsError` enum so decode failures are grounded in the shared-db API instead of being smuggled
through `?`:

```rust
#[error("failed to decode settings payload: {0}")]
Decode(String),
```

Then update the existing enum definition to include that variant and keep the existing `Database(...)` case intact, then add the decode helper:

```rust
pub fn decode_prefixed_settings<T>(
    prefix: &str,
    values: &HashMap<String, serde_json::Value>,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let object = values
        .iter()
        .filter_map(|(key, value)| key.strip_prefix(prefix).map(|trimmed| (trimmed, value)))
        .map(|(trimmed, value)| (trimmed.to_string(), value.clone()))
        .collect::<serde_json::Map<String, serde_json::Value>>();
    serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|error| report!(RawSettingsError::Decode(error.to_string())))
}
```

Do not redefine `RawSettingsError` from scratch in the implementation; extend the existing type in place.

In `settings_store.rs`, add typed wrappers on top of the raw loader and map the shared-db errors explicitly into the existing auth error surface:

```rust
pub async fn load_typed_settings_by_prefix<T>(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    prefix: &str,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, prefix)
        .await
        .map_err(|error| report!(AuthError::Internal(error.to_string())))?;
    uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, &raw)
        .map_err(|error| report!(AuthError::Internal(error.to_string())))
}
```

```rust
pub async fn load_typed_global_settings_by_prefix<T>(
    db: &DatabaseConnection,
    prefix: &str,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, prefix)
        .await
        .map_err(|error| report!(AuthError::Internal(error.to_string())))?;
    uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, &raw)
        .map_err(|error| report!(AuthError::Internal(error.to_string())))
}
```

In `email/src/surfaces.rs`, replace:

```rust
fn smtp_from_tenant_map(...)
fn smtp_from_global_map(...)
fn get_string(...)
fn get_port(...)
fn get_tls_mode(...)
```

with:

```rust
fn smtp_snapshot_from_prefixed_map(
    prefix: &str,
    map: &HashMap<String, serde_json::Value>,
) -> uptrakit_shared_db::raw_settings::Result<SmtpSettingsSnapshot> {
    uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, map)
}
```

In `notifications/dispatcher.rs`, keep the existing mixed SMTP+Telegram bag shape, but build the SMTP portion from typed snapshots and merge it back
into the current generic bag rather than changing the Telegram path.

- [ ] **Step 4: Run package checks**

Run:

```bash
cargo test -p uptrakit-notification-plugin-email smtp_settings_snapshot_deserializes_from_prefixed_map -- --exact
cargo check -p uptrakit-notification-plugin-email
cargo check -p uptrakit-web-api
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/notifications/email/src/surfaces.rs crates/ui/web-api/src/notifications/dispatcher.rs crates/ui/web-api-auth/src/settings_store.rs crates/shared/db/src/raw_settings.rs
git commit -m "refactor: deserialize smtp settings through typed snapshots"
```

### Task 4: Final Compatibility Verification

**Files:**

- Modify any DTO docs/tests surfaced by verification.

- [ ] **Step 1: Run the wire-shape verification set**

Run:

```bash
cargo fmt --all
cargo test -p uptrakit-web-api-types
cargo check -p uptrakit-web-api-types
cargo test -p uptrakit-notification-plugin-email
cargo check -p uptrakit-notification-plugin-email
cargo check -p uptrakit-web-api
```

Expected: PASS.

- [ ] **Step 2: Confirm the old map-rebuild family is gone**

Run:

```bash
rg -n "get_string\\(|get_port\\(|get_tls_mode\\(" crates/plugins/notifications/email/src/surfaces.rs crates/ui/web-api/src/notifications/dispatcher.rs
rg -n "load_typed_settings_by_prefix|load_typed_global_settings_by_prefix|decode_prefixed_settings" crates/plugins/notifications/email/src/surfaces.rs crates/ui/web-api/src/notifications/dispatcher.rs crates/ui/web-api-auth/src/settings_store.rs crates/shared/db/src/raw_settings.rs
```

Expected: no legacy `get_*` helper matches remain in the typed snapshot path, and the positive grep shows the new typed loader/decode helpers are
actually in use.

- [ ] **Step 3: Commit any last compatibility cleanups**

```bash
git add crates/shared/web-api-types/src/notifications/channels.rs crates/shared/web-api-types/src/software_items.rs crates/ui/web-api-queries/src/queries/software_items crates/plugins/notifications/email/src/surfaces.rs crates/ui/web-api/src/notifications/dispatcher.rs crates/ui/web-api-auth/src/settings_store.rs crates/shared/db/src/raw_settings.rs
git commit -m "chore: finish typed dynamic config boundary track"
```

## Self-Review

- Spec coverage: Task 1 covers notification config wrappers. Task 2 covers the software item patch/config boundaries. Task 3 covers SMTP/settings
  typed snapshots. Task 4 covers the preserved wire-shape verification pass.
- Placeholder scan: no unfinished-plan markers remain.
- Type consistency: `JsonObjectMap`, `IconUrlPatch`, and `smtp_snapshot_from_prefixed_map` are used consistently.
