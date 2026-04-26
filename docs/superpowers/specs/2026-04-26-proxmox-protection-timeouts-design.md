# Proxmox Protection Timeouts Design

## Goal

Split controller-side Proxmox pre-update protection timeouts by protection mode,
make the built-in defaults safer for real-world backup duration, and allow
optional timeout overrides at both the global Proxmox protection settings level
and the per-software-item override level.

## Requirements

### Functional requirements

- snapshot protection and backup protection must use different timeout values
- built-in defaults:
  - snapshot timeout: 120 seconds
  - backup timeout: 900 seconds
- global Proxmox update protection settings must allow overriding:
  - snapshot timeout
  - backup timeout
- per-software-item Proxmox update protection overrides must allow overriding:
  - snapshot timeout
  - backup timeout
- timeout fields must be mode-aware in the UI:
  - when mode is `snapshot`, show only `Snapshot timeout`
  - when mode is `backup`, show `Backup timeout`
  - when mode is `do_nothing`, show neither timeout field
  - for per-item overrides, keep `Backup Target` visible only for `backup`
- empty per-item timeout values must inherit the corresponding global timeout for the selected mode
- empty global timeout values must fall back to the built-in defaults
- the global UI must make the built-in default obvious even when the field value is empty
- the per-item UI must make it explicit that an empty timeout field means the system-wide value will be used

### Non-functional requirements

- preserve the existing controller-side pre-update protection flow
- keep timeout resolution inside the Proxmox policy layer rather than spreading fallback logic across UI and runtime code
- preserve the current mode and backup-target behavior
- keep the schema explicit and queryable instead of hiding timeouts inside an opaque JSON blob

## Current State

The current Proxmox protection policy stores only:

- `mode`
- `backup_target_key`

This shape exists in:

- SeaORM entities for `proxmox_protection_defaults` and `proxmox_protection_item_overrides`
- `ProtectionPolicy` in the Proxmox plugin policy store
- `ProxmoxProtectionPolicyRecord` in the shared infrastructure boundary
- typed surface save requests for global defaults and per-item overrides

At runtime, both snapshots and backups use a single hard-coded
`PROTECTION_WAIT_TIMEOUT` of 120 seconds in the Proxmox update-protection
implementation. This causes legitimate long-running backups to be marked failed
before dispatch even when Proxmox eventually completes the backup successfully.

## Design Decision

Use nullable timeout columns in the existing Proxmox protection policy tables.

### Why this approach

- fits the existing global-default plus per-item-override model
- keeps policy state in one storage layer
- makes fallback semantics explicit
- minimizes churn compared with inventing a new settings store or serializing a wider policy blob into JSON

### Rejected alternatives

#### JSON policy blob

Rejected because it would reduce schema clarity and increase blast radius
across persistence and typed contracts without solving a real problem for this
feature.

#### Separate timeout storage outside the protection tables

Rejected because it would split one policy across multiple stores and make
preload, save, and fallback resolution harder to reason about.

## Persistence Model

Add the following nullable columns to both `proxmox_protection_defaults` and `proxmox_protection_item_overrides`:

- `snapshot_timeout_seconds`
- `backup_timeout_seconds`

This requires a new forward migration for already-migrated databases. Updating
the original create-table migration is not sufficient because existing
deployments already have `seaql_migrations` entries for the current named
Proxmox protection-table migration.

### Semantics

- `NULL` in a per-item row means inherit the corresponding global timeout for that mode
- `NULL` in a global row means use the built-in default for that mode
- only the timeout for the selected mode is operationally relevant during update execution
- unused mode-specific timeout values may remain stored on the row; runtime resolution only reads the field for the active mode

### Built-in defaults

- snapshot: `120`
- backup: `900`

## Policy Model

Extend the persisted policy rows and the effective runtime policy model to
carry:

- `mode`
- `backup_target_key`
- `snapshot_timeout_seconds: Option<i64>`
- `backup_timeout_seconds: Option<i64>`

The shared typed policy record used by controller-side update protection should
expose the same fields.

### Effective timeout resolution

Given an effective policy scope:

1. resolve mode using existing item-override then global-default precedence
2. if mode is `snapshot`:
   - use item `snapshot_timeout_seconds` when present
   - otherwise use global `snapshot_timeout_seconds` when present
   - otherwise use built-in `120`
3. if mode is `backup`:
   - use item `backup_timeout_seconds` when present
   - otherwise use global `backup_timeout_seconds` when present
   - otherwise use built-in `900`
4. if mode is `do_nothing`, timeout resolution is skipped

Important implementation constraint:

- timeout inheritance must be merged per field, not by selecting either the
  item row or the global row wholesale
- mode can continue to use existing item-override then global-default
  precedence
- timeout fields must be resolved independently so an item override row with an
  empty timeout still inherits the global timeout for that mode

The shared typed policy record consumed by controller-side protection should
represent this already-merged effective policy so the runtime path does not need
to know whether a value came from item scope, global scope, or built-in
defaults.

## Surface Request Model

Extend the typed save requests for Proxmox protection surfaces with optional timeout fields:

- global defaults save request:
  - `snapshot_timeout_seconds: Option<i64>`
  - `backup_timeout_seconds: Option<i64>`
- per-item override save request:
  - `snapshot_timeout_seconds: Option<i64>`
  - `backup_timeout_seconds: Option<i64>`

Preload responses must also include both timeout fields so the forms can round-trip empty and explicit values.
These timeout fields should remain numeric in the typed request/response
boundary.

## UI Design

### Global settings surface

Surface: `Proxmox Update Protection`

Fields:

- `Proxmox Configuration`
- `Default Protection Mode`
- `Snapshot timeout`
- `Backup timeout`
- `Backup Target`

Field-type requirement:

- `Snapshot timeout` and `Backup timeout` must use numeric form fields so the
  surface form submits numbers rather than strings

Visibility rules:

- `Snapshot timeout` visible only when mode is `snapshot`
- `Backup timeout` visible only when mode is `backup`
- `Backup Target` visible only when mode is `backup`

Empty-value behavior:

- empty `Snapshot timeout` means use built-in default `120 seconds`
- empty `Backup timeout` means use built-in default `900 seconds`

UX requirement:

- when the global timeout field is empty, the form must still clearly show the
  built-in default through placeholder text and/or help text rather than
  silently relying on backend behavior

### Per-software-item surface

Surface: `Proxmox Update Protection`

Fields:

- `Proxmox Configuration`
- `Override Mode`
- `Snapshot timeout`
- `Backup timeout`
- `Backup Target`

Field-type requirement:

- `Snapshot timeout` and `Backup timeout` must use numeric form fields so the
  surface form submits numbers rather than strings

Visibility rules:

- `Snapshot timeout` visible only when mode is `snapshot`
- `Backup timeout` visible only when mode is `backup`
- `Backup Target` visible only when mode is `backup`
- no timeout field visible for `inherit_global` or `do_nothing`

Empty-value behavior:

- empty `Snapshot timeout` means use the system-wide snapshot timeout
- empty `Backup timeout` means use the system-wide backup timeout

UX requirement:

- the per-item timeout help text must explicitly state that leaving the field empty will use the system-wide value

## Save And Preload Behavior

### Preload

Global preload should return:

- selected `plugin_config_id`
- `mode`
- `backup_target_option`
- `snapshot_timeout_seconds`
- `backup_timeout_seconds`

Per-item preload should return:

- `software_item_id`
- selected `plugin_config_id`
- `mode`
- `backup_target_option`
- `snapshot_timeout_seconds`
- `backup_timeout_seconds`

If there is no stored row:

- global preload returns `do_nothing` plus empty timeout values
- per-item preload returns `inherit_global` plus empty timeout values

### Save

Global save:

- validates tenant and plugin-config scope as today
- validates the selected backup target only in `backup` mode
- parses timeout fields as optional integers from numeric form inputs
- stores `NULL` for omitted or empty timeout fields

Per-item save:

- preserves existing `inherit_global` semantics by deleting the per-item row
- validates assignment and backup target only when applicable
- parses timeout fields as optional integers from numeric form inputs
- stores `NULL` for omitted or empty timeout fields

## Validation Rules

- timeout values are optional
- when present, timeout values must be positive integer seconds
- `0` is invalid
- negative values are invalid
- non-numeric values are invalid
- mode-inactive timeout inputs should not affect runtime behavior
- backup target remains required only in `backup` mode

Validation failures should be surfaced through the existing surface action error path with specific user-facing messages.

## Runtime Behavior

Replace the single shared protection wait timeout with separate resolved values.

### Snapshot path

`prepare_snapshot_protection()` should wait using the resolved snapshot timeout.

### Backup path

`prepare_backup_protection()` should wait using the resolved backup timeout.

### No orchestrator change

The update orchestrator contract does not need to change. It already treats
controller-side protection success or failure generically. Only the timeout fed
into Proxmox task polling changes.

## Migration And Bootstrap Paths

This feature must update all relevant schema paths used by the project:

1. add a new forward Proxmox controller migration that alters existing
   protection-policy tables to add the new nullable columns
2. update the original Proxmox protection-table create migration so fresh
   databases are created with the new columns from the start
3. update the lightweight controller-owned SQL bootstrap path used in
   `surface_proxy` tests and controller-owned surface setup

Fresh-database safety requirement:

- because fresh databases run the full ordered Proxmox migration list, the new
  forward migration must be safe when the columns already exist
- the forward migration must therefore either check for column existence before
  adding the new columns or otherwise behave as a no-op on fresh databases that
  already got the columns from the updated create migration

Without updating all three, upgraded databases, fresh databases, and
controller-owned test/bootstrap paths can drift from each other.

## Testing Strategy

### Policy and persistence

- policy-store tests for global and per-item round-trip of timeout fields
- fallback tests for:
  - explicit per-item timeout
  - inherited global timeout
  - built-in default when global is empty
  - item override row present with `NULL` timeout still inheriting the global
    timeout instead of falling through directly to built-in default

### Surface handlers

- preload tests proving empty persisted values round-trip as empty form values
- save tests proving empty submitted values become `NULL`
- validation tests for invalid timeout input
- mode-sensitive tests confirming backup target validation remains limited to backup mode
- form-schema coverage proving timeout inputs are numeric fields so typed save
  requests receive numbers, not strings

### Runtime protection

- tests proving snapshot path uses the resolved snapshot timeout
- tests proving backup path uses the resolved backup timeout
- regression test covering backup timeout above 120 seconds so backup mode is not locked to the old constant

### Schema coverage

- migration coverage for the new nullable columns on both fresh schema creation
  and forward upgrade from an existing schema
- controller-owned bootstrap coverage so the test-created tables include the new columns

## Out Of Scope

- changing audit-table schema to persist the resolved timeout value used for a run
- introducing timeout configuration for non-Proxmox controller protection providers
- adding duration widgets or unit selectors in the UI beyond integer seconds
- changing update-history summary text for successful or failed protection beyond existing behavior

## Expected Outcome

After this change:

- snapshots keep their existing default two-minute timeout
- backups default to fifteen minutes instead of two minutes
- operators can optionally override both values globally
- software items can optionally override both values while inheriting cleanly when left empty
- the UI makes fallback semantics explicit instead of hidden
- long-running valid backups are no longer forced to fail at the old shared
  120-second ceiling
