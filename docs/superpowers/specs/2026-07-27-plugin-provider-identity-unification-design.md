# Plugin / Surface-Provider Identity Unification

**Date:** 2026-07-27
**Status:** Approved (user interview 2026-07-27)
**Scope:** `crates/plugins/infrastructure/core` (macro + ops struct + catalog), the five surface-bearing plugin
crates, `crates/ui/surface-proxy` (registry admission, catalog item accessor, `controller_local` consumers),
`crates/ui/web-api` (`routes/surfaces.rs` visibility lookup), `crates/plugins/infrastructure/registry` (guard
test), docs.
**Sequencing:** Requires the plugin-type-id rename (bead epic
`uptrakit-spec-2026-07-20-surfaces-id-naming-convention-design`, retired at the beads migration
2026-08-16; full text at `pre-beads-archive`; landed as `e10261875`) — verified
present. Intent coupling: the pending contribution-monotonicity spec
(`2026-07-27-plugin-contribution-monotonicity-design.md`, NO_PLAN) edits the same neighborhood (proxmox
`plugin.rs`, registry guard tests) and explicitly defers plugin identity to this spec; either landing order
works, but whichever lands second rebases its guard-test edits.

## Problem (verified reality, 2026-07-27)

Two identifiers name the same plugin, related by an unenforced convention, re-derived differently by every
consumer — one of them incorrectly:

- **type_id** — `PluginTypeId` (`crates/shared/types/src/plugin_type_id.rs`): unvalidated `Cow<'static, str>`
  newtype, keys `PluginCatalog`. Dotted-kebab grammar enforced only by the catalog guard test
  (`crates/plugins/infrastructure/registry/tests/surface_id_naming_guard.rs`).
- **provider_id** — free `&'static str` authored per plugin in `declare_plugin!`'s `surfaces:` arm
  (`PluginSurfaceRegistrationOps.provider_id`, `crates/plugins/infrastructure/core/src/descriptor.rs:217`).
  `$type_id` and `$provider_id` are independent macro inputs — nothing ties them
  (`macros.rs` surfaces arm; `__declare_unified_surface_ops_static!` assigns the literal straight through).
  Carried on the wire in `ProviderIdentity { provider_id: String, provider_kind, provider_namespace }`
  (`crates/shared/surfaces/src/protocol.rs:1027-1032`).

Convention `provider_id == "plugin." + type_id` holds at all five surface-bearing plugins (docker, proxmox,
webhook, telegram, email — inventoried by workspace sweep). Consumers, each different:

- `crates/ui/web-api/src/routes/surfaces.rs:71` — `plugin_ops.get(&PluginTypeId::new(&item.provider_id))`,
  **no strip**: the catalog is keyed by bare type ids, so the lookup always misses, falls through
  `.unwrap_or(true)`, and `is_plugin_visible_to_user` has **never run in production** for any plugin-backed
  surface (verified by direct read; the other four callers of the predicate resolve descriptors correctly).
- `crates/ui/surface-proxy/src/proxy/controller_local/notification_settings.rs:28-31` —
  `strip_prefix("plugin.")` then `strip_prefix("notifications.")`, compared to `channel_type`.
- `crates/ui/surface-proxy/src/proxy/controller_local/notifications.rs:22` —
  `provider_id == format!("plugin.notifications.{channel_type}")`.
- `crates/ui/surface-proxy/src/proxy/controller_local/docker.rs:14` —
  `matches!(provider_id, "plugin.releases.docker" | "releases.docker")` — accepts a legacy bare form; the
  convention has already drifted once.

Load-bearing environment facts (all verified):

- `provider_id` is **never persisted**: no DB column carries it (DB stores type ids —
  `plugin_configs.plugin_type`, `instance_plugin_setting.plugin_type_id`, …); the surface registry is
  in-memory; no audit row, SSE topic, or URL embeds it (`/surfaces/{surface_id}` keys on surface_id).
- Frontend and CLI treat `provider_id` as opaque: display, equality against the server-returned provider
  list, and pass-back as `target_provider_id`. No prefix parsing anywhere
  (`frontend/src/lib/{surfaces,components/surfaces}/`, `crates/ui/cli/src/commands/surfaces.rs`).
- Services self-construct `service.uptrakit-mqtt.<uuid>` / `service.uptrakit-agent-ssh.<uuid>`; **nothing
  enforces that shape**. `validate_registration_basics` (`crates/ui/surface-proxy/src/registry.rs:599`)
  rejects `provider_kind != source_kind`, so a remote service cannot claim `Plugin` kind — but the registry's
  provider map is keyed by `provider_id` across kinds, so a service could squat a plugin's id string.
- `bootstrap_builtin` has zero production callers — `ProviderKind::BuiltIn` is reserved/test-only today.
- `provider_namespace` is hardcoded `"plugin"` / `"service"` per source and read by no production logic.
- No `ProviderId` type exists anywhere; every carrier is `String` / `&'static str`.

## Decisions (settled — user interview 2026-07-27; do not reopen)

| #   | Decision              | Resolution                                                                                                                                                                                                                                                                      |
| --- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Identity model        | **Drop the `plugin.` prefix and derive.** For `ProviderKind::Plugin`, `provider_id` IS the plugin type id string (`infrastructure.proxmox`). The authored `provider_id` field is deleted from the macro and the ops struct.                                                     |
| D2  | Visibility activation | The corrected lookup **activates** `is_plugin_visible_to_user` for plugin-backed surfaces in this spec, rewritten explicitly (kind branch + direct lookup), with per-tier tests. Named behavior change. Broader enforcement semantics stay with the visibility spec ("Spec 3"). |
| D3  | Namespace rule        | Admission **enforces** provider-id namespaces per kind: `Service` registrations must start `service.`; `BuiltIn` must start `builtin.`. Fail-closed. Compatible with both shipped satellites (verified).                                                                        |
| D4  | Typed representation  | **Kind-gated accessor**, not a `ProviderId` newtype: `fn plugin_type_id(&self) -> Option<PluginTypeId>` returning `Some` iff `provider_kind == Plugin`. Identity conversion — nothing is parsed, so the `FromStr` rule does not apply (rationale below). Wire stays `String`.   |

## Design

### 1. Derivation is structural, not asserted (answers brief Q1 + Q5)

Delete `PluginSurfaceRegistrationOps.provider_id` (`descriptor.rs:214-220`) and the `provider_id` input of
`declare_plugin!`'s `surfaces:` arm entirely. The single **production** wire-conversion call site —
`registration.to_wire(ops.provider_id)` in `PluginCatalog` aggregation
(`crates/plugins/infrastructure/core/src/catalog.rs:447`) — iterates descriptors and has the enclosing
`PluginDescriptor` in scope; it passes `descriptor.type_id` instead. Test callers enumerated by grep
(2026-07-27): the registry bootstrap test at `crates/ui/surface-proxy/src/registry.rs:2348` also reads
`surface_ops.provider_id` (updates with the field deletion), and 18 plugin-crate/core test sites pass the
`"plugin.…"` literal to `to_wire(…)` directly (proxmox `plugin.rs` ×6, email ×3, telegram ×3, webhook ×2,
docker `tests.rs` ×3, `registration.rs:324`) — each re-points to the bare type id; the plan re-runs this
grep rather than trusting the counts. `to_wire(&self, provider_id: &str)`
(`registration.rs:175`) keeps its signature; `provider_namespace: "plugin"` and `provider_kind: Plugin`
(`registration.rs:180`) are unchanged.

There is no second string to drift: the macro arm becomes `surfaces: { registrations }`, and every
`"plugin.…"` literal in the five plugin crates is deleted rather than rewritten. This is the structural
answer the brief asked for — the equality assertion becomes a guard against _manual_ construction paths only
(§6). No plugin legitimately needs a provider id unrelated to its type id, and no plugin owns several
providers (inventory: exactly one `provider_id` per `surfaces:` arm, five total, all convention-conforming).

### 2. The `plugin.` prefix carried no information (answers brief Q2)

`ProviderKind::Plugin` is present on every registration and admission-enforced against the registration
source, and `provider_namespace` already says `"plugin"`. Contrarian-verified: no audit row, SSE payload,
persisted value, URL, frontend, or CLI logic depends on the prefix; the three `controller_local` sites that
_do_ depend on it are exactly the consumers this spec rewrites (§4). Stale-tab `target_provider_id` mismatch
after deploy is transient (reload refetches the catalog) and identical in kind to the rename that just
landed.

### 3. Typed representation (answers brief Q3)

`SurfaceCatalogItem::plugin_type_id(&self) -> Option<PluginTypeId>` in
`crates/ui/surface-proxy/src/registry.rs` — `Some(PluginTypeId::new(&self.provider_id))` iff
`self.descriptor.provider_kind == ProviderKind::Plugin`, else `None` (`SurfaceCatalogItem` carries no
top-level kind field; the discriminant lives on the nested descriptor — the same field the current
`routes/surfaces.rs` filter reads). Placement rationale: `uptrakit-surfaces` does not depend on
`uptrakit-shared-types` (verified in its manifest), and adding that edge for one accessor is not warranted;
`uptrakit-surface-proxy` already depends on `uptrakit-shared-types`, and every consumer lives in
`surface-proxy` or downstream of it (`web-api` depends on `surface-proxy`).

**`FromStr` regime statement:** after D1 the provider_id → type_id mapping for plugins is the identity
function gated on an enum discriminant — there is no grammar to parse and no failure mode a
`Parse{Type}Error` could represent, and the accessor goes through the type's own shipped constructor
(`PluginTypeId::new` / its `From<&str>` impl — the repo's existing infallible API; `PluginTypeId::FromStr`
is already `Infallible`), so no ad-hoc `parse(&str)` method is introduced anywhere. The AGENTS.md `FromStr` rule ("Use `FromStr` for all
string-to-type conversions") governs _parses_; this design deliberately lands in the _no-conversion_ regime,
which is why no `FromStr` impl, no `ProviderId` newtype, and no ad-hoc `parse()` method is added. If a future
change reintroduces a derived encoding (any prefix), that change must add the typed parse.

### 4. Consumer rewrites (lockstep, all four)

- `routes/surfaces.rs:64-78`: replace the filter body with a call to an extracted private fn (the testable
  seam — see §Testing) — non-`Plugin` kinds pass through unchanged; `Plugin` kind resolves via the §3
  accessor and `plugin_ops.get(&type_id)`. **Lookup miss → hide the surface and `tracing::warn!`**
  (fail-closed). Justification for flipping the `.unwrap_or(true)` default: plugin
  registrations enter the registry only through catalog bootstrap, so a `Plugin`-kind item whose provider id
  is unknown to the catalog is an internal inconsistency, not a legitimate state — passing it through was the
  bug this spec exists to fix.
- `controller_local/notification_settings.rs:28-31`: replace the double `strip_prefix` with a comparison
  against `notification_plugin_type(channel_type)` (`uptrakit_shared_types::notification_plugin_type` —
  the existing helper), i.e. derive the expected id from the channel type once, typed, instead of
  string-surgery on the provider id.
- `controller_local/notifications.rs:22`: same helper — `notification_plugin_type(channel_type).as_str() ==
provider_id` replaces the `format!("plugin.notifications.{channel_type}")` comparison. Using the shared
  helper at both sites removes the last hand-rolled category derivation from `controller_local`.
- `controller_local/docker.rs:14`: single comparison `provider_id == PLUGIN_TYPE_RELEASES_DOCKER` — the
  file's **existing** local const (`docker.rs:6`, value `"releases.docker"`, currently unused by the match) —
  deleting the two-form legacy match. Not `plugin_ids::RELEASES_DOCKER`: `ci/check_plugin_semantic_boundary.py`
  forbids `plugin_ids::` references outside `crates/plugins/` (the boundary the
  `notification_plugin_type` helper's own doc comment documents), so the shared const is unreachable from
  `surface-proxy` production code.

These three `controller_local` sites gate action dispatch (and, for notification_settings, an audit
emission); they receive a bare `&str` without kind in scope. That is safe: `validate_registration_basics`
already rejects `ControllerLocal`-transport interactions from non-`Plugin` sources, and D3 makes plugin-id
squatting by services unregistrable — so a bare type-id comparison in `controller_local` is unambiguous.

### 5. Visibility activation (D2 — named behavior change)

With the lookup fixed, `is_plugin_visible_to_user` (`crates/ui/web-api/src/visibility.rs:23`) runs for
plugin-backed surfaces on `GET /api/v1/surfaces`. Per-tier outcome (must be tested exactly this way):

- Tenant-scoped plugins: visible — predicate always passes; **no change**.
- Instance-scoped plugin, enabled: visible to all — **no change**.
- Instance-scoped plugin, disabled: hidden from users without `ManageGlobalSettings` (**new, intentional
  hardening** — previously leaked through), still visible to `ManageGlobalSettings` holders (predicate
  design; unchanged by this spec).
- Instance-scoped plugin, **no `instance_plugin_setting` row** (the production default for any future
  instance-scoped surface plugin): treated as disabled — `InstancePluginSnapshot::enabled()` returns `false`
  for absent rows (`web-api-queries/src/instance_plugin_settings.rs:55-60`, fail-closed by design). Pinned
  as its own test tier so the default is a checked contract before such a plugin exists.
- `Plugin`-kind item unknown to the catalog: hidden for everyone + warn (**new**, fail-closed, §4).

Adjacent known issue, explicitly out of scope: whether a _visible_ disabled-instance-plugin surface is fully
functional for admins (dispatch-side gating) is the instance-scoped-gating / visibility spec's territory —
this spec changes only the listing filter's resolution and leaves the admin tier's behavior byte-identical.

### 6. Admission namespace rule (D3 — answers brief Q4)

`validate_registration_basics` gains one shape check on `registration.provider.provider_id`, keyed on
`source_kind`:

- `Service` → must start with `"service."` (both shipped satellites already comply — verified constructors in
  `mqtt-runtime` and `agent-ssh-runtime`).
- `BuiltIn` → must start with `"builtin."` (zero production callers today; reserves the namespace before one
  appears). Existing `bootstrap_builtin` **test fixtures** use `controller.builtin*` ids with
  `provider_namespace: "controller"` (`registry.rs` tests, e.g.
  `bootstrap_builtin_registers_surface_through_registry_path`) — their ids migrate to `builtin.…`; their
  namespace strings are untouched (namespace cleanup is deferred).
- `Plugin` → must **not** start with `"service."` or `"builtin."` (bootstrap-only path; equality with the
  descriptor's type id is guaranteed structurally by §1 and guarded by §7).

Violations push a `SurfaceProviderRejectionReason` with the existing `InvalidTransport` code (the code
already covers kind/tenant identity misuse at this boundary — e.g. `provider_kind … not allowed for this
registration source`; no new wire-visible rejection-code variant is added). Fail-closed on the ambiguous
case: anything not matching the required prefix is rejected, never normalized. `validate_surface_identifier`
(the permissive per-ID charset validator) is untouched — this rule is admission-side and applies to the
provider id only.

Compatibility tightening, named: a hypothetical third-party _service_ provider registering a
non-`service.`-prefixed provider id would now be rejected at admission. Both first-party satellites comply;
the rule is security-motivated (a service must not be able to occupy a plugin's identity in the
provider-keyed registry map). Test-migration deliverable (named, not incidental churn): the **primary**
bare-id `Service`-kind fixture site is `web-api`'s `service_ws` handler tests — `test_surface_registration("provider-a")`
and siblings (`provider-b`, `provider-mqtt`, `provider-system`) across `handler/{tests.rs,test_support.rs,session_authenticated.rs,embedded.rs}`
— plus the `registry.rs` unit-test fixtures; prefer centralizing the `service.` prefix in the
`test_support.rs` registration helper so one edit covers most callers. A second cluster the helper edit does
**not** cover: bare-literal equality/passback assertions against the registered id
(`target_provider_id: Some("provider-a")` sites in `session_authenticated.rs`, the separate
`test_surface_registration` redefinition in `integration_tests/surfaces_routes.rs`, and
`routes/surfaces.rs` test literals) — these fail loudly at assert-time, and the mandatory re-grep for the
full set (rather than trusting this list) finds them.

### 7. Guard (defense in depth behind the structural fix)

Extend `crates/plugins/infrastructure/registry/tests/surface_id_naming_guard.rs`: build a real
`PluginCatalog` (`build_catalog()`) and iterate **`PluginCatalog::surface_registrations()`** — a
`PluginSurfaceOps` trait method (`catalog.rs:440`), so the test imports the trait
(`use uptrakit_plugin_infrastructure_registry::PluginSurfaceOps`, re-exported at the crate root) — the wire
registrations the production aggregation site (`catalog.rs:447`) actually emits — asserting each
registration's `provider.provider_id` equals the owning descriptor's `type_id`, `provider_kind == Plugin`,
and `provider_namespace == "plugin"`. The test must **not** call `to_wire()` itself: the existing guard
file's idiom (walking `all_descriptors()` and calling `(ops.registrations)()` on pre-wire types) never
reaches the aggregation site, and a test that reconstructs the wire form supplies the very value it asserts —
a tautology that cannot catch a regression at `catalog.rs:447`. Post-§1 the equality cannot fail via
`declare_plugin!`; the guard covers manual construction paths and the aggregation site itself.

The same guard file also asserts **no descriptor `type_id` starts with `"service."` or `"builtin."`** — on
the existing `all_descriptors()` walk (every plugin, not just surface-bearing ones, so the code matches the
"no descriptor" claim) — the
type-id grammar does not otherwise reserve D3's admission roots, and without this assertion a future plugin
named e.g. `service.foo` would pass the catalog naming guard yet be rejected at `register_plugin` admission,
silently dropping its surfaces at controller startup, far from the authoring site. The assertion ties the
two independently-evolving grammars together at the boundary where the id is authored.

Feature honesty (mandatory, per ADR-0031's guard): the populating command is the scoped run
`cargo test -p uptrakit-plugin-infrastructure-registry --features notifications-email,notifications-telegram,notifications-webhook`
(default features keep `agent-infra` off so proxmox registrations exist); the test presence-gates per
provider family on the observed catalog and must also pass green under `--all-features` (proxmox legitimately
absent). RED demonstration: perturb a **value** (feed a manual ops construction with a mismatched id) — never
delete a registration (dead-code deny fires before the assertion).

## Compatibility

- **No DB migration** — provider ids are not persisted anywhere (§Problem). The rename plan's spent
  compatibility budget is not re-spent: this change touches no persisted value and no wire _schema_
  (`ProviderIdentity` fields unchanged; only the plugin-provider id _values_ change, in-memory,
  controller-internal — plugins compile into the controller and the frontend/CLI are data-driven).
- **Wire docs:** `asyncapi.yaml` does not enumerate `ProviderIdentity` fields (surface-registration schema is
  not expanded there), so no asyncapi edit; the provider-id namespace rules (D3) and the plugin-id == type-id
  identity are documented in `docs/api/wire-protocol.md`'s surface-registration section and
  `docs/development/surfaces.md`.
- **Satellite skew:** old service binaries already send `service.`-prefixed ids — admitted before and after.
  Plugin provider ids never cross the service wire outbound as registration input.
- **Stale browser tab:** a pre-deploy tab submitting `target_provider_id: "plugin.…"` post-deploy gets
  `InvalidProvider` once; reload self-heals. Same class as the landed rename; accepted.

## Testing

- **Guard test** (§7) with the exact populating command and RED-by-value rule.
- **Visibility activation tests** in `web-api`, split by what real plugins can drive. **Constraint
  (verified 2026-07-27):** the sole instance-scoped plugin (`enhancement.dashboard-icons`) declares no
  `surfaces:` arm, and all five surface-bearing plugins are tenant-scoped — so the two instance-scope tiers
  of §5 **cannot** be driven through any real plugin registration. Therefore:
  - Extract the per-item filter decision into a private fn (e.g.
    `fn plugin_surface_visible(item, plugin_ops, snapshot, user) -> bool`) that the route's filter calls,
    and unit-test **all five §5 tiers** through that fn with fixture descriptors — `visibility.rs`'s
    existing test module already builds an `INSTANCE_PLUGIN_DESCRIPTOR` fixture with
    `PluginScope::Instance` and drives `is_plugin_visible_to_user` across enabled/disabled × permission
    permutations; reuse that fixture shape. The tests exercise the same fn the production filter calls, so
    a regression in the extracted logic cannot pass while the route stays green.
  - Route-level tests (shared `TestApp` harness) cover the tiers real data can drive: tenant-scoped plugin
    surface present (positive case) and an unknown-`Plugin`-kind provider id hidden (bootstrap a synthetic
    `Plugin`-kind registration with a catalog-unknown id). Primitives exist —
    `build_test_state_with_plugin_surfaces()` (`test_harness/mod.rs:332-361`, raw-state; used by four
    `service_ws` dispatch tests) and `TestApp::with_stub_surfaces` (`mod.rs:86-122`) as the constructor
    prior art; the named deliverable is the `TestApp`-level assembly bridging them (they have mismatched
    signatures today: raw `Arc<AppState>` vs `&TestApp`-taking `upsert_instance_plugin_setting()`,
    `fixtures.rs:236-263`). Assert on presence/absence of the **specific surface** in the response, never
    counts, for both a non-admin and a `ManageGlobalSettings` user.
- **Admission rule tests** in `surface-proxy`: service registration with bare id rejected
  (`InvalidTransport` reason present), `service.`-prefixed accepted; builtin analog; plugin registration with
  a `service.`-prefixed id rejected. Existing bare-id service fixtures migrated (§6).
- **Consumer tests:** existing `controller_local` dispatch tests (notification settings/channels, docker) and
  plugin descriptor/wire tests re-pointed at bare provider ids — enumerate referencing tests workspace-wide
  by grep (survey found fixture literals in `surface-proxy` test regions, `web-api` service-ws handler tests,
  `releases/docker/src/tests.rs`, `web-api-queries/src/instance_plugin_settings.rs:404-409`), including
  runtime `.expect()` field reads, not just compile breaks.
- Success and failure paths covered per the testing standard; no upstream-crate behavior tested.

## Verification (mechanical)

Site-class sweep for the deleted prefix (exact-literal grep alone is insufficient):

1. Quoted literals: `grep -rn '"plugin\.' --include='*.rs'` → only historical data in
   `docs/superpowers/{specs,plans}` history (excluded) and the two doc examples being updated.
2. Composed forms / derivations: `grep -rn 'strip_prefix("plugin\|format!("plugin\|concat!("plugin\|starts_with("plugin' --include='*.rs'` → zero hits.
3. Bare-word pass over comments/help text: `grep -rn '\bplugin\.\(infrastructure\|notifications\|releases\)\b'`
   repo-wide including `*.md`, top-level docs included, spec/plan history excluded → zero hits.
4. Presence: the five bare type ids appear as provider ids in the guard test's observed catalog (survivor
   strings non-zero).
5. Canonical gates (full list per `docs/development/quality-gates.md`): `cargo fmt --all`;
   `cargo check` and `cargo clippy --all-targets` in **both** feature worlds
   (`--no-default-features --features db-sqlite` and `--all-features`);
   `cargo test --all-features` (with `frontend/build/` present); the scoped guard command (§7);
   `markdownlint` on touched docs. `cargo deny check` is not triggered — this change adds no
   dependencies and edits no manifests.

## Deliverables

**Code:** macro arm + ops-struct field deletion and catalog call-site change (§1); five plugin-crate
`declare_plugin!` edits (literal deletions); accessor (§3); four consumer rewrites (§4); admission rule +
fixture migration (§6); guard-test extension (§7); test additions (§Testing).

**Docs (non-optional):**

- New ADR `docs/adr/00XX-plugin-provider-identity.md` — verify next free number at implementation time
  (0031 is the latest on disk; the pending contribution-monotonicity spec claims 0032, so expect 0033):
  D1–D4, the no-parse regime statement, the namespace rules (including the reserved-root tie to the type-id
  grammar), and an explicit statement that plugin providers are **singletons keyed by type_id** — per-tenant
  or per-instance plugin provider identities (the way services mint `service.<name>.<uuid>` per instance)
  are a recorded non-goal that would require a future derived-encoding migration plus the typed parse §3
  reserves for that case. The ADR distinguishes the two axes sharing the word "instance": instance-_scoped_
  (`PluginScope::Instance`, a visibility axis — orthogonal, unaffected) vs per-instance _identity_ (the
  minting axis, the non-goal).
- `docs/development/surfaces.md` — "Unified plugin registration model" section: `surfaces: { provider_id,
registrations }` → `surfaces: { registrations }`; provider-identity paragraph (provider id == plugin type
  id for `Plugin` kind; namespace rules for `Service`/`BuiltIn`).
- `docs/security/surfaces.md` — admission namespace rule (fail-closed provider-id namespaces per kind) and
  the activated visibility filter's fail-closed unknown-id behavior.
- `docs/api/wire-protocol.md` — surface-registration provider-identity value rules (no schema change; state
  that explicitly).
- `docs/development/plugin-system.md` (`plugin.` example at :571) and `docs/development/notifications.md`
  (`plugin.slack` examples at :151, :358) — update examples to the bare-id model.
- `CONTEXT.md` — extend the _Surface Provider_ entry: a plugin provider's id is its plugin type id.
- No `asyncapi.yaml` change (justified in §Compatibility).

**No new dependencies** (external or workspace edges).

## Non-goals / deferred

- Surface visibility _semantics_ and broader enforcement (Spec 3) — this spec activates the existing
  predicate at the listing filter only.
- Notification `channel_type` strings — channel identity, not plugin identity (rename plan's exclusion
  stands).
- `provider_namespace` field cleanup (derive from kind / drop) — display-only today; touching it is churn
  with no consumer.
- `ProviderId` newtype over the wire carriers — rejected as over-modeling for an in-memory identity
  (D4 alternative).
- Tightening `validate_surface_identifier` (per-ID charset) — unchanged, per ADR-0031 C3.
- Re-opening the completed type-id rename.
