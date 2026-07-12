# Instance-Scoped Surface Interaction Gating — Design

Harden the two global-settings surface **read** interactions that rely solely on their surface-descriptor
permission gate, and add a structural guard so no instance-scoped surface interaction can ship ungated again.

Primary justification: **consistency** — two `DataLoad` reads are ungated while their write siblings on the
same surface (and the entire proxmox reference) gate at the action level; asymmetric gating is a latent trap
for the next author who copies a read interaction as a template. Defense-in-depth is a **secondary** benefit,
with a bounded scope (see Problem). Severity: no live authorization bypass. Behavior change: **none** for
permitted users.

## Problem

Surface authorization is **two-layer**. On invoke, the controller enforces _both_ the surface-descriptor
permission and the interaction permission:

- `crates/ui/web-api/src/routes/surfaces.rs:243` — `enforce_required_permission(resolved.descriptor.required_permission …)`
- `crates/ui/web-api/src/routes/surfaces.rs:263` — `enforce_required_permission(resolved.interaction.required_permission …)`

And `resolve_surface_action` binds an interaction to its **home surface** — the interaction is looked up
_within_ the named surface, and `resolved.descriptor` is that surface's descriptor
(`crates/ui/surface-proxy/src/registry.rs:477`–`492`). An attacker therefore **cannot** pair a weakly-gated
surface id with a strongly-scoped interaction id: the interaction must be a member of the surface it is
invoked through. Read (`GET /surfaces/{id}`) enforces the surface-descriptor gate as well
(`surfaces.rs:184`).

Consequence: there is **no live bypass** today. Every "ungated" interaction is still protected by its home
surface's `required_permission`.

**The reads are asymmetric with their write siblings.** Two global-settings _read_ interactions carry **no
action-level permission** and depend entirely on the surface-descriptor gate, while every write sibling on
the same surface gates itself:

| Plugin   | Interaction                                                                       | Home surface (gate)                                                                   | Action-level gate |
| -------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | ----------------- |
| email    | `get_global_smtp` (`crates/plugins/notifications/email/src/plugin.rs:1046`)       | `notifications.email.global_smtp` — `manage_global_settings` (`plugin.rs:1017`)       | **none**          |
| telegram | `get_global_telegram` (`crates/plugins/notifications/telegram/src/plugin.rs:622`) | `notifications.telegram.global_settings` — `manage_global_settings` (`plugin.rs:606`) | **none**          |

Their write siblings already carry the action-level gate _and_ sit under the same gated surface
(`save_global_smtp`, `test_global_smtp_email`, `save_global_telegram` — all `manage_global_settings`).
The reads are the only asymmetric interactions.

Reads disclose instance-scoped SMTP host/port/username and Telegram global-fallback metadata (the password
and `bot_token` are masked at the query layer, but the endpoint config is still instance-scoped and gated
by `ManageGlobalSettings` per [ADR-0006](../../adr/0006-instance-scoped-plugins.md)).

**What the action gate does and does not mitigate.** Adding an interaction-level gate adds a redundant check
that survives two future regressions: (a) the interaction being **moved onto a second, weaker surface**, and
(b) a **Universal-targeted provider registered for the same surface id** with a weaker descriptor gate — in
both cases the interaction still carries its own `manage_global_settings` requirement. It does **not** protect
against a regression in the surface-descriptor _enforcement path_ itself: if the code that reads
`required_permission` breaks, a second string sitting on the interaction is read by the same broken code and
helps nothing. So the defense-in-depth value is real but bounded to authoring/registration mistakes, not
enforcement-engine bugs — the honest reason to ship is consistency, with (a)/(b) as a bonus.

### Root cause

There is no structural invariant enforcing _"an instance-scoped surface interaction gates itself at the
action level."_ Gating is hand-authored per interaction; two `DataLoad` reads were simply missed. Nothing
catches the omission at compile or test time.

### Reference: proxmox already conforms

Proxmox is the correct pattern. Both of its `manage_global_settings` surfaces gate **every** interaction at
the action level:

- `proxmox.settings.update-hooks` (`crates/plugins/infrastructure/proxmox/src/plugin.rs:443`) —
  `preload-global-defaults`, `load-backup-target-options`, `save-global-defaults` all `ManageGlobalSettings`.
- `proxmox.settings.resource-scaling` (`plugin.rs:642`) — `preload-scaling-global-defaults`,
  `save-scaling-global-defaults` all `ManageGlobalSettings`.

The fix brings email/telegram up to the proxmox standard and then _locks that standard in place_ with a guard.

## Approach

Three additive changes. No new dependency, no wire/API/config/frontend change, no behavior change for users
who legitimately hold `ManageGlobalSettings` (the only users who can render or save these surfaces today).

1. **Gate the two reads** at the action level, matching proxmox's typed form.
2. **Add a class-guard test** over the real plugin catalog asserting the invariant for
   `manage_global_settings` surfaces, so this specific omission cannot recur in any compiled-in first-party
   plugin (coverage boundary spelled out in change 3; the general typed-field fix is deferred — Out of scope).
3. **Document the invariant** in the canonical surface-security doc.

### Idiomatic notes

- Surface descriptors store `required_permission` as a free-form `Option<String>` (see the
  [Shared surface runtime](../../../AGENTS.md) subsystem stub — `required_permission` on descriptors and
  interactions is enforced server-side before dispatch). Proxmox uses the typed source
  `Permission::ManageGlobalSettings.to_string()`; the notification plugins use the raw literal
  `"manage_global_settings"`. **Use the typed form** for the two new gates to match proxmox and avoid
  literal drift. `Permission` is `uptrakit_shared_types::Permission`
  (`crates/shared/types/src/permissions.rs`; `ManageGlobalSettings.as_str()` → `"manage_global_settings"`),
  the same source proxmox imports as `use uptrakit_shared_types::Permission;`. Both notification crates
  already depend on `uptrakit-shared-types` in `Cargo.toml` but do **not** yet import `Permission` — add the
  `use` (zero new dependency). Their other permission strings stay raw literals here (see Out of scope).
- The guard is a plain `#[test]`, not a new CI script. The real catalog is assembled once by
  `all_descriptors()`; a unit test that iterates its surfaces is the idiomatic, lowest-friction enforcement.
  Escalate to a standalone CI check only if a second, structurally different surface-gate rule ever needs
  enforcing.

### Invariant scope (deliberate)

The guard targets **only** surfaces whose descriptor gate is `manage_global_settings` (the instance scope).
It does **not** require every interaction under _any_ gated surface to self-gate — that would flag the
`view_notifications` reads (`list`, `get_smtp`) which are a separate, out-of-scope decision. Keeping the
invariant tied to the instance scope matches the chosen fix and avoids scope creep.

## Changes

### 1. `crates/plugins/notifications/email/src/plugin.rs` — gate `get_global_smtp`

In the `get_global_smtp` `DataLoad` interaction (currently `plugin.rs:1044`–`1054`, which sets only
`result_schema`), add the action-level gate:

```rust
let mut i = surfaces::InteractionDescriptor::new(
    surfaces::InteractionId::new("get_global_smtp")
        .expect("literal interaction id is valid"),
    surfaces::InteractionKind::DataLoad,
    "Get Global SMTP Defaults",
    surfaces::InteractionTransport::ControllerLocal,
);
i.required_permission = Some(Permission::ManageGlobalSettings.to_string()); // added
i.result_schema = Some(surfaces::SchemaContract::Any);
i
```

### 2. `crates/plugins/notifications/telegram/src/plugin.rs` — gate `get_global_telegram`

In the `get_global_telegram` `DataLoad` interaction (currently `plugin.rs:620`–`630`), add the same line:

```rust
let mut i = surfaces::InteractionDescriptor::new(
    surfaces::InteractionId::new("get_global_telegram")
        .expect("literal interaction id is valid"),
    surfaces::InteractionKind::DataLoad,
    "Get Global Telegram Settings",
    surfaces::InteractionTransport::ControllerLocal,
);
i.required_permission = Some(Permission::ManageGlobalSettings.to_string()); // added
i.result_schema = Some(surfaces::SchemaContract::Any);
i
```

Note: `get_global_telegram` is the `pre_load_interaction_id` of the `save_global_telegram` form
(`telegram/src/plugin.rs:659`). The pre-load invoke already flows through the `manage_global_settings`
surface gate, so any user who can render/save the form also passes the new action gate — no UX regression.
The email save form pre-loads `get_global_smtp` the same way (`email/src/plugin.rs:1208`).

### 3. `crates/plugins/infrastructure/registry/` — class-guard test

`all_descriptors()` (`crates/plugins/infrastructure/registry/src/registry.rs:27`) is the authoritative,
single-source list of compiled-in plugins; a `PluginCatalog` built from it exposes every first-party
surface via `PluginCatalog::surface_registrations()`
(`crates/plugins/infrastructure/core/src/catalog.rs:385`). Add a test in the registry crate (the only crate
that sees all first-party plugin surfaces) that asserts the instance-scope invariant.

**Feature-gate (load-bearing — the guard is vacuous without it).** The email and telegram descriptors are
`#[cfg(feature = "notifications-email"/"notifications-telegram")]`-gated inside `all_descriptors()`
(`registry.rs:63`–`67`), and the registry crate's `default = ["daemon"]` enables **neither**
(`registry/Cargo.toml`). Under a plain `cargo test -p uptrakit-plugin-infrastructure-registry` the catalog
contains **zero** notification `manage_global_settings` surfaces (only proxmox, already conforming), so a
naive loop finds no offenders and passes **green even before the fix** — the claimed RED state never occurs.
Two guards against this:

1. **Compile the test only when both features are on:**
   `#[cfg(all(feature = "notifications-email", feature = "notifications-telegram"))]`, and run it under
   `cargo test --all-features` (or `--features notifications-email,notifications-telegram`). This is the
   feature set the CI `cargo test --all-features` gate already uses.
2. **Assert the target surfaces are actually present** before asserting they are clean, so an empty or
   feature-stripped catalog fails **loud** instead of passing vacuously.

**Struct shape (verified):** `surface_registrations()` returns `Vec<SurfaceRegistration>`
(`crates/shared/surfaces/src/protocol.rs`). `SurfaceRegistration` has **no** `descriptor`/`interactions`
fields — it carries `provider` + `surfaces: Vec<RegisteredSurface>`. The `descriptor: SurfaceDescriptor`
and `interactions: Vec<InteractionDescriptor>` fields live one level deeper, on `RegisteredSurface`. The
guard therefore iterates two levels (matches the existing flatten in
`crates/ui/surface-proxy/src/registry.rs` and the catalog test
`registrations[0].surfaces[0].descriptor` in `catalog.rs`):

```rust
// Name says what it actually checks: one specific permission string, not "all instance scopes".
#[cfg(all(feature = "notifications-email", feature = "notifications-telegram"))]
#[test]
fn manage_global_settings_surfaces_gate_every_interaction() {
    use uptrakit_plugin_infrastructure_registry::build_catalog;
    use uptrakit_plugin_infrastructure_core::{CatalogConfig, InstancePluginStates};

    const INSTANCE_GATE: &str = "manage_global_settings";

    // Reuse the production construction path — do not hand-roll a second catalog assembly.
    // (Descriptors stay registered regardless of instance-disable, so all_disabled() is fine;
    //  the only thing that hides these surfaces is the feature gate, handled by #[cfg] above.)
    let catalog = build_catalog(&CatalogConfig::default(), InstancePluginStates::all_disabled())
        .expect("catalog builds from all_descriptors()");

    let mut gated_surface_ids: Vec<String> = Vec::new();
    let mut offenders: Vec<String> = Vec::new();
    for registration in catalog.surface_registrations() {
        for surface in &registration.surfaces {
            if surface.descriptor.required_permission.as_deref() != Some(INSTANCE_GATE) {
                continue;
            }
            let surface_id = surface.descriptor.surface_id.as_str();
            gated_surface_ids.push(surface_id.to_string());
            for interaction in &surface.interactions {
                if interaction.required_permission.as_deref() != Some(INSTANCE_GATE) {
                    offenders.push(format!("{surface_id} :: {}", interaction.interaction_id.as_str()));
                }
            }
        }
    }

    // Non-vacuous: the two notification surfaces this guard exists to protect MUST be in the catalog,
    // else the loop above is asserting over an empty set and would pass no matter what.
    for expected in ["notifications.email.global_smtp", "notifications.telegram.global_settings"] {
        assert!(
            gated_surface_ids.iter().any(|s| s == expected),
            "guard is vacuous — expected {INSTANCE_GATE} surface {expected} not found in catalog \
             (run under --all-features); present: {gated_surface_ids:?}"
        );
    }

    assert!(
        offenders.is_empty(),
        "{INSTANCE_GATE} surfaces must gate every interaction at the action level; ungated: {offenders:?}"
    );
}
```

Construction path is unambiguous: `build_catalog(&CatalogConfig::default(),
InstancePluginStates::all_disabled())` (`crates/plugins/infrastructure/registry/src/lib.rs:108`, wrapping
`PluginCatalog::new(all_descriptors(), …)`) — the same pair already used by every catalog-construction test
in `catalog.rs`. Confirm the exact import paths for `CatalogConfig`/`InstancePluginStates` (re-exported from
the core crate) during coding. The registry crate is the correct home — it keeps the guard next to
`all_descriptors()` and its `Cargo.toml` already carries a `[dev-dependencies]` section.

**RED demonstration:** revert either action-gate line (change 1/2) and run
`cargo test --all-features -p uptrakit-plugin-infrastructure-registry` — the offenders assertion goes RED.
Run with default features and the test does not compile/run at all (by design), so it can never green-on-empty.

**Coverage boundary (state in the doc too):** the guard covers **compiled-in first-party** surfaces reachable
through `all_descriptors()`. Runtime, service-registered provider surfaces (`UiSurfaces` capability) are not
in the catalog and are **not** covered by this test — they are covered by the live two-layer enforcement at
invoke time, not by the guard. The guard also checks the single literal `manage_global_settings`; a future,
differently-named instance-scoped permission would not be covered (see Out of scope for the typed-field fix
that would generalize it).

### 4. `docs/security/surfaces.md` — document the invariant

Add, under the existing **Permission model** section (`docs/security/surfaces.md`; the AGENTS.md stub's
`#action-level-permissions` anchor does not resolve verbatim — the live heading is `## Permission model`):

- The **two-layer enforcement** fact: invoke checks both the surface-descriptor and the interaction
  `required_permission`, and `resolve_surface_action` binds the interaction to its home surface (no
  cross-surface pairing).
- The **instance-scoped self-gating invariant**: any interaction on a `manage_global_settings`
  (instance-scoped) surface must _also_ declare `required_permission = manage_global_settings` at the action
  level — do not rely on the surface gate alone. Cite the registry guard test as the enforcement mechanism,
  and state its **coverage boundary**: it covers compiled-in first-party surfaces (via `all_descriptors()`,
  under `--all-features`) and the single `manage_global_settings` literal only; runtime provider-registered
  surfaces are covered by live two-layer enforcement, not the guard.

## Tests

- **Guard test (deliverable, change 3)** — the primary regression guard. **Must run under
  `--all-features`** (or `--features notifications-email,notifications-telegram`): the notification
  descriptors are feature-gated in `all_descriptors()`, so under default features the test is compiled out
  and cannot green-on-empty. Its presence assertion fails loud if the two target surfaces are absent. RED
  before the fix (offenders = the two reads) under those features, GREEN after; reverting either action-gate
  line re-reds it. This _is_ the negative-path coverage for the invariant.
- **Per-plugin sanity (optional, low value):** the email/telegram crates already have surface tests
  (e.g. `telegram_global_settings_surface_keeps_preload_form_behavior`,
  `telegram/src/plugin.rs:847`). A one-line assertion in each that the global surface's interactions all
  carry `manage_global_settings` is redundant with the class-guard; prefer the single authoritative guard
  and skip per-plugin duplication unless a maintainer wants local coverage.
- No new time-dependent or async tests (no `tokio::time` usage) — `start_paused` not applicable.

## Documentation deliverables

- **`docs/security/surfaces.md`** — required (change 4). Documents two-layer enforcement + the
  instance-scoped self-gating invariant + the guard test as enforcement.
- **`AGENTS.md`** (root, "Shared surface runtime" subsystem stub) — optional one-line addition noting the
  self-gating invariant with a link to `docs/security/surfaces.md`. The existing rule ("Surface permissions
  are enforced at read/invoke time") already covers _enforcement_; the new fact is an _authoring_ invariant.
  Add the one-liner only if the maintainer wants it surfaced at the index level; canonical home is
  `docs/security/surfaces.md`. Keep under the AGENTS.md size budget; no inventory/counts.
- **No new ADR.** This is not a new architectural decision — it implements
  [ADR-0006](../../adr/0006-instance-scoped-plugins.md)'s existing `ManageGlobalSettings` instance gate
  _consistently_. State this explicitly in the PR description rather than authoring an ADR.
- No README, API-doc, wire-protocol, or frontend-doc impact (no externally observable behavior, surface, or
  config change).

## Out of scope / deferred

- Generalizing the invariant to **all** gated surfaces — e.g. requiring the `view_notifications` reads
  (`list`, `get_smtp`) to self-gate at the action level. Separate decision; different threat model
  (tenant-scoped reads, not instance-scoped).
- **The real structural fix: typing the descriptor gate.** The root cause is that `required_permission` is a
  free-form `Option<String>` on `SurfaceDescriptor`/`InteractionDescriptor`
  (`crates/shared/surfaces/src/{surface,interaction}.rs`) — it can be misspelled, omitted, or drift from the
  `Permission` enum with nothing catching it at construction. Making it `Option<Permission>` (typed, validated
  at construction) is what would _eliminate_ this class of bug, and would also let the guard iterate an
  `is_instance_scoped()` predicate instead of one literal string — generalizing it to every instance-scoped
  permission, current and future. That is a larger change (touches the surface contract + its wire
  serialization) and is **deferred, not solved, by this spec**; this spec makes the two known offenders
  conform and locks in the specific `manage_global_settings` case. File the typed-field change as the
  follow-up structural fix. Closing this spec does **not** resolve the root cause.
- Promoting the guard from a unit test to a standalone CI script — unnecessary until a second, structurally
  different surface-gate rule needs enforcing.
- Converting the notification plugins' _other_ raw `"manage_global_settings"` string literals to the typed
  `Permission::ManageGlobalSettings.to_string()` form wholesale — only the two new gates adopt the typed
  form here; a crate-wide literal→typed sweep is a separate drive-by.
- **Docker** (`2026-07-12-docker-switch-tag-tenant-isolation-design.md`) and **Proxmox**
  (`2026-07-11-proxmox-match-tenant-isolation-design.md`) tenant-isolation IDOR fixes — different bug class
  (cross-tenant DB reads/writes), already tracked as their own specs.
- Query-layer masking review of `get_global_smtp` / `get_global_telegram` response payloads — passwords and
  bot tokens are already masked; no change proposed.
