# Enforce the Descriptor Action Gate on Provider-Origin Surface Invocation — Design

Close a latent authorization gap: the **provider-origin** (service-initiated) surface-invoke path enforces only
the _interaction_-level action gate and never the _surface-descriptor_ gate, so any interaction that relies on
its home surface's descriptor gate (rather than gating itself) is invocable by any same-tenant `UiSurfaces`
service with no action check. Fix it at the engine: make the provider-origin gate deny when **either** the
descriptor or the interaction carries a `required_action` (unless the interaction opts in via
`provider_invocable`).

> Scope history: this spec was originally scoped as "gate two ungated global-settings reads + a registry guard
> test." Review found the two reads are the visible symptom of a broader engine-level gap (six ungated
> notification `DataLoad` reads across the provider-origin path), and that per-interaction gating + a
> feature-gated literal-string guard is the fragile, symptom-level fix. The owner elected the engine-level fix
> (2026-08-09). This document is the reworked design; the earlier per-interaction/guard approach is recorded
> under [Superseded approach](#superseded-approach) for provenance.
>
> Terminology: gates are catalog **action strings** (`resource:verb`), carried as `required_action` on surface
> and interaction descriptors, parsed to a typed `Action` at registration admission, enforced by
> `AccessEngine` (HTTP path) or the provider-origin gate (service path). The instance-scope gate of
> [ADR-0006](../../adr/0006-instance-scoped-plugins.md) is today `system.settings:manage`
> (`actions::SYSTEM_SETTINGS_MANAGE`, `crates/shared/types/src/access/catalog.rs:195` — `SystemSettings`
> resource). ADR-0006 predates the action/grant migration and still names the retired `ManageGlobalSettings`
> permission; that is the same gate under the old name.

## Problem

Surface authorization has **two invoke paths**, and they enforce **different** subsets of the two-layer gate.

**HTTP path** (browser/API caller, `crates/ui/web-api/src/routes/surfaces.rs`). Enforces _both_ layers via
`enforce_required_action` → `AccessEngine`:

- `surfaces.rs:236` — descriptor gate on surface read (`GET /api/v1/surfaces/{id}`)
- `surfaces.rs:464` — descriptor gate on interaction invoke
- `surfaces.rs:483` — interaction gate on interaction invoke
- resolution binds the interaction to its **home surface** (`resolve_surface_action_for_method`,
  `crates/ui/surface-proxy/src/registry.rs:598`), so a weak surface id cannot be paired with a strong
  interaction id or vice versa

On the HTTP path there is no gap: an interaction that gates only at the descriptor level is still protected,
because the descriptor gate runs. (Full resolution order 404 → 403/500 → 405 is in
[docs/security/surfaces.md § Authorization model](../../security/surfaces.md#authorization-model).)

**Provider-origin path** (service WebSocket, `CallerOrigin::Provider`). Gated in the proxy at **one live
site** — `invoke_inner` in `crates/ui/surface-proxy/src/proxy.rs:313`. A token-identical copy of the gate
also sits in `crates/ui/surface-proxy/src/proxy/prepared.rs:56`, but that file is an **orphan**: it is not
`mod`-declared anywhere in the crate and compiles nowhere (one of the five orphans from the crate-scaffold
move — see commit `5c1c2357b`, which deliberately keeps it aligned with the live path so the divergence
cannot resurface if it is ever wired back in).

```rust
if matches!(&caller_origin, surfaces::CallerOrigin::Provider { .. })
    && resolved.interaction_required_action.is_some()   // interaction gate ONLY
    && !resolved.interaction.provider_invocable
{
    return Err(SurfaceProxyError::PermissionDenied(...));
}
```

This consults `interaction_required_action` **only**. `descriptor_required_action` is never read on this path
(verified: the only non-test reads of `descriptor_required_action` are the HTTP handler in `surfaces.rs`). So
on the provider-origin path the two-layer model collapses to **one layer — interaction gate + tenant scope**.
This matches the current wording of [docs/security/surfaces.md § Provider-origin
invocation](../../security/surfaces.md#provider-origin-invocation) ("an interaction with `required_action` is
denied … unless it sets `provider_invocable`"), so the code is faithful to the _documented_ model — but the
documented model itself under-protects: it lets a descriptor gate be silently bypassed by any
descriptor-gated-but-interaction-ungated interaction.

**A service can reach another provider's surface.** `handle_surface_action_request`
(`crates/ui/web-api/src/routes/service_ws/handler/message_processor.rs:600`–`655`) takes `surface_id`,
`interaction_id`, and `target_provider_id` verbatim from the connected service's wire payload; only tenant
scope is checked. So any enrolled same-tenant service with `UiSurfaces` can target a plugin-owned surface it
does not own. (One precondition throughout this document: the calling service must itself have registered at
least one surface — `caller_origin_for_request` (`proxy.rs:890`) resolves `Provider` origin via
`provider_id_for_service` and rejects callers with no registered surface provider. "Any same-tenant
`UiSurfaces` service" below means any such registered one.)

### The live gap (verified by blast-radius scan)

Six first-party `DataLoad` reads sit on descriptor-gated surfaces with **no interaction gate and no
`provider_invocable`**. Today, every one is invocable by any same-tenant `UiSurfaces` service via the
provider-origin path, with **no action check**:

| #   | Surface (descriptor gate)                                           | Interaction (`DataLoad`)     | File:line                                                 | Discloses                                         |
| --- | ------------------------------------------------------------------- | ---------------------------- | --------------------------------------------------------- | ------------------------------------------------- |
| 1   | `notifications.email` (`notifications:read`)                        | `channels` "List"            | `crates/plugins/notifications/email/src/plugin.rs:526`    | per-channel email config rows                     |
| 2   | `notifications.email` (`notifications:read`)                        | `smtp` "Get SMTP Settings"   | `email/src/plugin.rs:865`                                 | per-channel SMTP config                           |
| 3   | `notifications.email.global-smtp` (`system.settings:manage`)        | `smtp` "Get Global Defaults" | `email/src/plugin.rs:940`                                 | instance SMTP host/port/username, `has_password`  |
| 4   | `notifications.telegram` (`notifications:read`)                     | `channels` "List"            | `crates/plugins/notifications/telegram/src/plugin.rs:270` | **plaintext per-channel `bot_token`** (see below) |
| 5   | `notifications.telegram.global-settings` (`system.settings:manage`) | `settings` "Get"             | `telegram/src/plugin.rs:545`                              | instance Telegram fallback, `has_bot_token`       |
| 6   | `notifications.webhook` (`notifications:read`)                      | `channels` "List"            | `crates/plugins/notifications/webhook/src/plugin.rs:273`  | per-channel webhook config                        |

Existing tests prove the mechanism directly: `service_ws/handler/tests.rs:563`
(`surface_action_success_emits_success_tenant_audit_row`) sets `interactions[0].required_action = None` under
a gated descriptor and the provider-origin invoke is then **allowed**; `tests.rs:1176` denies a cross-provider
`proxmox.hosts` invoke **only** because that interaction carries `hosts:update`.

**Severity is uneven and mostly low, with one sharp edge:**

- Reads #3/#5 (the `system.settings:manage` pair, the original trigger): instance-wide SMTP host/port/username
  and boolean presence flags. Secrets are masked. Low value, but it is instance-scoped config crossing to a
  tenant service — outside what tenant co-trust is meant to cover.
- Reads #1/#2/#6: tenant's own channel config to the tenant's own co-trusted service — within the documented
  co-trust envelope; low concern.
- **Read #4 is the sharp one:** the telegram `channels` handler masks **nothing**
  (`telegram/src/surfaces.rs:84` passes `|_type, config| config.clone()`, and `list_channels` decrypts via
  `expose_secret()` before the mask fn — `crates/plugins/notifications/core/src/list_channels.rs:68`), so it
  returns **plaintext per-channel `bot_token`s** for the whole tenant. That is a tenant-scoped credential
  disclosure to any same-tenant `UiSurfaces` service. (The unmasked-on-read behavior is a _separate_ masking
  defect that also affects HTTP `notifications:read` callers — see [Out of scope](#out-of-scope--deferred);
  this spec closes the ungated provider-origin _path_, not the masking.)

**No first-party service exercises any of the six.** Grep of `agent-ssh-runtime`, `mqtt-runtime`,
`service-sdk`, and the infra plugins found zero provider-origin calls to any `notifications.*` surface. Every
infra/service surface (proxmox, docker, ssh-agent, mqtt) already gates every interaction at the interaction
level, or sets `provider_invocable` on the ones intentionally provider-invoked. So the gap is real but
latent — closing it breaks no shipped flow (see [Blast radius](#blast-radius--behavior-change)).

### Root cause

The provider-origin gate was written to enforce the interaction gate only, on the implicit assumption that any
interaction needing protection would gate itself. But the platform also lets a surface gate at the descriptor
level and leave member interactions ungated (the six reads above rely on exactly this). The HTTP path honors
that pattern; the provider-origin path does not. The fix is to make the provider-origin gate honor the
descriptor gate too — one enforcement point, covering every current and future surface, plugin- **and**
service-registered.

## Approach

Enforce the descriptor gate on the provider-origin path, symmetric with HTTP. Five changes.

1. **Fix the provider-origin gate** (both sites) to deny when the descriptor **or** the interaction carries a
   gate, unless the interaction is `provider_invocable`.
2. **Tighten registration admission** so a service cannot mint the escape hatch under a gated descriptor:
   reject `provider_invocable` on a service-registered interaction whose **home surface descriptor** carries
   `required_action` (today admission rejects only the interaction-gated combination), keeping the escape
   plugin/built-in-only for every gated shape.
3. **Regression test** at the proxy layer covering the whole class (descriptor-gated + interaction-ungated +
   not `provider_invocable` → denied; `provider_invocable` escape still allowed; interaction-gated still
   denied), plus admission tests for change 2.
4. **ADR** — this changes the documented provider-origin security model.
5. **Docs** — update `docs/security/surfaces.md § Provider-origin invocation` and the root `AGENTS.md` surface
   rule.

No new dependency, no wire/API/config/frontend change.

### Idiomatic notes

- The live gate site already has `resolved.descriptor_required_action` and `resolved.interaction_required_action`
  in scope — typed `Option<Action>`, parsed at admission, documented as "authoritative for enforcement" on the
  struct (`crates/ui/surface-proxy/src/registry.rs:1199`–`1208`). The fix reads the fields already present; it
  does **not** touch the wire `required_action: Option<String>` display fields.
- `provider_invocable` is per-interaction and defaults `false` (`crates/shared/surfaces/src/interaction.rs:199`).
  Admission (`InteractionDescriptor::validate_for_provider`, `interaction.rs:248`–`261`) rejects it on
  **service-registered** interactions only when the interaction **itself** declares `required_action` — the
  check never sees the descriptor gate. So without change 2, a service could self-register a descriptor-gated
  surface whose ungated interaction sets `provider_invocable = true` and sail through the corrected engine
  gate. Change 2 closes that at admission; the escape hatch then stays plugin/built-in-only wherever any gate
  exists — exactly where it is used today (e.g. proxmox `match`, `unmatched-guests`, the only production
  setters). Keying the runtime escape on the interaction flag (not the descriptor) preserves those semantics
  unchanged.
- There is **one live enforcement site** (`proxy.rs:313`); edit it inline — no shared helper needed for a
  single caller. `prepared.rs:56` is an orphan copy that compiles nowhere (no `mod` declaration; commit
  `5c1c2357b`), so no compiler, test, or `warnings = "deny"` gate can verify it: mirror the same predicate
  edit there per that commit's established policy (keep the orphan aligned so divergence cannot resurface if
  it is wired back in), and note the alignment in the commit message. Reviving or deleting the orphan (and
  its four siblings) is a separate decision — out of scope here.

## Changes

### 1. `crates/ui/surface-proxy/src/proxy.rs` — enforce descriptor gate

At the live site (`invoke_inner`, `proxy.rs:313`), replace the interaction-only condition (and mirror the
same edit in the orphaned `prepared.rs:56` copy — see Idiomatic notes):

```rust
if matches!(&caller_origin, surfaces::CallerOrigin::Provider { .. })
    && (resolved.descriptor_required_action.is_some()
        || resolved.interaction_required_action.is_some())
    && !resolved.interaction.provider_invocable
{
    return Err(SurfaceProxyError::PermissionDenied(
        "provider-initiated requests cannot satisfy user permission gates".to_string(),
    ));
}
```

Only the middle predicate changes (`.is_some()` on the interaction → `descriptor OR interaction`). The
`provider_invocable` escape and the error value are unchanged, so `provider_invocable` interactions on gated
surfaces stay invocable exactly as today.

One alignment note comes with it: both fields are populated from
`provider.surface_actions.get(surface_idx)` (`registry.rs:658`/`:681`), and an index miss would yield
`None`/`None`, which the gate reads as "ungated" and **allows**. Today that miss is unreachable by
construction — `parse_surface_actions` (`registry.rs:183`) pushes exactly one `SurfaceActionSet` per
`registration.surfaces` entry, fails the whole registration on any parse error, and both vectors are stored
together in one `ProviderRegistration`, with `surface_idx` derived from the same stored vec under one lock.
There is **no live fail-open**. But change 1 makes these fields security-load-bearing for the provider path,
so add a registry regression test asserting `surface_actions` stays index-aligned with `surfaces` across
registration — it guards a future refactor that stores the two separately.

### 2. `crates/shared/surfaces/src/protocol.rs` — admission rejects the escape under a gated descriptor

With change 1 alone, a service could still self-register a descriptor-gated surface whose ungated interaction
sets `provider_invocable = true` — admission accepts it today (the interaction-level check at
`interaction.rs:248`–`261` never consults the descriptor), and the engine gate would then wave the invoke
through. That reopens the closed class at the registration layer. Add the check in
`validate_interaction_provider_rules` (`protocol.rs:395`–`402`), which already has both `surface.descriptor`
and the interaction in scope and maps the existing interaction-level rejection to
`invalid_contract(...)`: for a service-kind provider, reject `provider_invocable` when the home surface
descriptor carries `required_action`, reusing `SurfaceRegistrationErrorCode::InvalidContract` with a
descriptive message (no new wire error code — the wire shape is untouched). Leave the interaction-level check
in `validate_for_provider` unchanged. Note: this check reads the wire field
`surface.descriptor.required_action: Option<String>` — the "display-only, never consult for authorization"
warning (`registry.rs:1199`) applies to **enforcement**, not admission; here only _presence_ is consulted,
and admission subsequently parses the string to a typed `Action` and rejects the registration if it does not
parse, so a present-but-invalid gate can never reach the runtime path. Also note the check keys off
`surface.descriptor.provider_kind`, a registrant-supplied field that is trustworthy only because of a two-hop
pin: `validate_surface_provider_kind` (`protocol.rs:189`) pins it to the registration's `provider_kind`, and
`validate_registration_basics` (`registry.rs:787`) pins that to the connection's trusted `source_kind` —
without the chain, a service could self-declare `Plugin` and mint the escape hatch. State both facts in the
code comment so a later simplification of either hop does not silently reopen the class.

Severity of the blocked shape is low — service-backed surfaces dispatch to the registrant, so the combination
can only disclose the registrant's own data — but it contradicts the invariant this spec establishes (escape
hatch is plugin/built-in-only wherever a gate exists), so close it where the other half of the rule already
lives. Zero first-party blast radius: the only production `provider_invocable` setters are the plugin-kind
proxmox interactions (`crates/plugins/infrastructure/proxmox/src/plugin.rs:256`/`:334`); no first-party
service registers the flag at all.

### 3. `crates/ui/surface-proxy/` — regression test

Add a proxy-level test (co-located with the existing provider-origin gate tests; the surface-proxy crate
already has a `#[cfg(test)]` module exercising `resolve_surface_action_for_method` and gate behavior). It must
assert, over a synthetic registration (no feature gates, no first-party surface ids — so it cannot rot when a
plugin surface is renamed):

- **descriptor-gated + interaction-ungated + not `provider_invocable`** → provider-origin request denied
  (the class this spec closes; RED before change 1, GREEN after);
- **descriptor-gated + interaction-ungated + `provider_invocable = true`** → allowed (escape hatch intact).
  This case's fixture **must use a plugin-kind registration** (mirroring the real proxmox setters): change 2
  makes the service-kind version of this shape unregistrable, and the proxy-level registry does not run
  admission validation, so a service-kind fixture would green-light a state no real registration can reach.
  Do not "fix" this by un-gating its descriptor — the admission-layer negative twin lives in the change-2
  tests;
- **interaction-gated + not `provider_invocable`** → denied (pre-existing behavior preserved — this is the
  `tests.rs:1176` shape);
- **ungated descriptor + ungated interaction** → allowed (no over-blocking of genuinely public surfaces);
- **HTTP-origin (`CallerOrigin::User`)** unaffected — the gate is `Provider`-only.

This replaces the superseded registry guard test: it covers all six offenders _and_ every runtime
service-registered surface (which the registry catalog could never see) in one place, keyed on the typed
`Action` presence rather than a single literal string.

### 4. New ADR — provider-origin enforces the descriptor gate

Create with `adrs new "Provider-origin surface invocation enforces the descriptor action gate"` (never
hand-number; never hand-edit `docs/adr/README.md`). Record: the prior interaction-only model and its gap, the
decision to enforce descriptor-or-interaction on the provider-origin path, the `provider_invocable` escape
semantics (runtime escape unchanged, interaction-keyed; admission tightened so services cannot set the flag
under a gated descriptor — change 2), and the blast-radius finding (six latent reads closed, no shipped
flow affected). This is a genuine security-model decision, distinct from ADR-0006 (which _defines_ the
instance scope) — ADR-0006's gate was one victim, not the subject. Keep the required ADR sections free of
placeholder tokens (`...`/`todo`/`describe`) — `adrs doctor` fails on them.

### 5. Docs

- **`docs/security/surfaces.md § Provider-origin invocation`** (`docs/security/surfaces.md:85`) — the section
  currently states provider-origin is gated by "an interaction with `required_action` … unless
  `provider_invocable`." Update it to: provider-origin is denied when **the surface descriptor or the
  interaction** carries `required_action`, unless the interaction sets `provider_invocable` — symmetric with
  the HTTP path's two-layer enforcement. Note the escape remains interaction-keyed and that admission now
  rejects the flag on service-registered interactions that are gated **or** sit on a gated descriptor
  (change 2).
- **`docs/security/surfaces.md § Authorization model`** — add one line noting both invoke paths now enforce the
  descriptor gate (previously only the HTTP path did).
- **`docs/development/surfaces.md`** (~line 107, the author-facing surface guide) — currently states
  service-initiated calls are allowed "only when the target interaction has no `required_action` or opts in
  via `provider_invocable`", and that admission rejects the flag on action-gated service interactions. Update
  both statements to the descriptor-or-interaction model and the extended admission rule — this is the doc
  plugin/surface authors read, so leaving it stale re-teaches the pattern this spec eliminates.
- **`docs/architecture/surfaces.md`** (~line 107) — states the provider-origin gate denies "an action-gated
  interaction … unless it sets `provider_invocable`". Update to descriptor-or-interaction wording.

## Blast radius / behavior change

For **provider-origin callers only**, the six `DataLoad` reads in the [live-gap table](#the-live-gap-verified-by-blast-radius-scan)
flip from allowed → denied. Verified against the whole surface catalog:

- Every proxmox/docker/ssh-agent/mqtt interaction already sets its own `required_action` (or
  `provider_invocable`), so the change is a **no-op** for them.
- No first-party connected service invokes any of the six reads (grep-confirmed). So no in-repo flow breaks.
- **HTTP callers are entirely unaffected** — the descriptor gate already ran for them.

**Residual risk:** an out-of-tree integration that today drives one of the six reads over a service connection
would start receiving `PermissionDenied`, and an out-of-tree service registering `provider_invocable` on an
interaction under a gated descriptor would now be rejected at registration (change 2). Neither shape exists
in-repo; flag for release-notes / manual QA. This is a security hardening, not a feature regression.

## Tests

- **Proxy regression test (deliverable, change 3)** — the primary guard. No feature gate required (it uses a
  synthetic registration, not the feature-gated notification descriptors), so it runs under the default
  `cargo test -p uptrakit-surface-proxy`. RED before change 1 on the descriptor-gated/interaction-ungated case,
  GREEN after. These tests **are time-dependent**: the `invoke()` path under test calls `tokio::time::timeout`
  (`proxy.rs:436`), so use `#[tokio::test(start_paused = true)]` like the sibling
  provider-origin tests (e.g. `provider_proxied/mod.rs:698`) — never real sleeps.
- **Admission tests (deliverable, change 2)** — success and failure paths: service-registered
  descriptor-gated surface with an ungated `provider_invocable` interaction → registration rejected; the same
  shape from a plugin-kind provider → accepted; ungated descriptor + ungated `provider_invocable` service
  interaction → still accepted (preserves the `interaction.rs:553` "unpermissioned service" behavior).
- **Existing provider-origin tests** — two groups will break; audit during coding, they are the canary that
  the behavior change is intentional:
  - `service_ws/handler/tests.rs:563/683/1176` — re-run under `--all-features`; `:563`
    (`surface_action_success_emits_success_tenant_audit_row`, sets `required_action = None` on an interaction
    under a **gated descriptor** and expects allow) and `:683`
    (`surface_action_provider_unavailable_emits_failed_tenant_audit_row`, same nulled-gate fixture on a
    registered provider — the provider-origin gate runs **before** the connectivity check it expects to hit,
    so its `ProviderUnavailable` assertion flips to `PermissionDenied`) need their expectations updated, or
    their fixture descriptor un-gated. (`tests.rs:472` also nulls the interaction gate but targets an
    unregistered provider, so it fails at lookup before the gate — unaffected by change 1, no update needed.)
  - `crates/ui/surface-proxy/src/proxy/tests/provider_proxied/mod.rs:553`
    (`invoke_provider_origin_can_route_to_another_provider`) and `:815`
    (`invoke_provider_origin_self_target_when_target_none`) — the shared `registration()` fixture gates the
    **descriptor** (`.required_action(actions::SOFTWARE_READ)`, `mod.rs:45`) and both tests null only the
    interaction gate, then assert provider-origin success — exactly the shape change 1 flips to denied.
    Un-gate the fixture descriptor for these routing-focused tests, or update their expectations.
  - Un-gating those service-registered fixtures loses no security coverage: the
    gated-descriptor/provider-origin denial class is owned by the change-3 regression tests, and change 2
    forbids the only alternative fixture shape (service + gated descriptor + `provider_invocable`), so
    un-gating is the consistent remediation for tests whose subject is routing/audit, not gating.

## Documentation deliverables

- **New ADR** (change 4) — required; the model change is architectural.
- **`docs/security/surfaces.md`** (change 5) — required; §§ Provider-origin invocation + Authorization model.
- **`docs/development/surfaces.md`** and **`docs/architecture/surfaces.md`** (change 5) — required; both
  currently state the interaction-only provider-origin rule and must move to descriptor-or-interaction
  wording (the development guide also documents the admission rule change 2 extends).
- **Root `AGENTS.md`** — the "Surface actions are enforced at read/invoke time" invariant currently says
  provider-origin "calls are denied for action-gated interactions unless … `provider_invocable`." Update
  "action-gated interactions" → "interactions on an action-gated surface or with an action-gated interaction"
  (or equivalent concise wording). One-line change; keep under the size budget; canonical detail stays in
  `docs/security/surfaces.md`.
- **No** README, API-doc, wire-protocol (`asyncapi.yaml`), OpenAPI, or frontend-doc impact — no wire shape,
  endpoint, or UI surface changes; the fix is an internal enforcement predicate. `./scripts/regen-api.sh` /
  `regen-asyncapi.sh` not triggered.

## Out of scope / deferred

- **Masking the notification `channels` reads.** Reads #1/#2/#4/#6 return **unmasked** channel config
  (telegram #4 leaks plaintext `bot_token`) to _any_ caller who passes the descriptor gate — including HTTP
  `notifications:read` users. This spec closes the ungated **provider-origin path** to them; it does **not**
  fix the mask function (`telegram/src/surfaces.rs:84` identity mask; `list_channels` `expose_secret()`).
  File the masking fix separately — it is a distinct defect (least-privilege within an authorized read), not a
  gating one.
- **Per-interaction gating of the six reads.** With the engine fix, adding `i.required_action` to each read is
  redundant for security (the descriptor gate now covers both paths) and reintroduces exactly the
  authoring-discipline drift this rework removed. Skip it. If a maintainer wants belt-and-suspenders
  explicitness on the two `system.settings:manage` reads, that is a cosmetic follow-up, not part of this fix.
- **Caller-scoping `provider_invocable` (owner-only vs any-service) and default-deny cross-provider
  targeting.** Both were considered and rejected here as a different design question: cross-provider
  service→plugin invocation is the documented _purpose_ of `provider_invocable` (the proxmox `match` /
  `unmatched-guests` flows are invoked by services on a plugin-owned surface, within tenant co-trust), so
  owner-scoping the escape or default-denying cross-provider targeting would break the shipped flow this flag
  exists for. An ungated descriptor + ungated interaction remains tenant-public by declaration — that is the
  documented contract, not a gap. If the co-trust envelope for enrolled services is ever narrowed (e.g.
  per-caller allowlists on the escape), that is its own ADR-worthy redesign, orthogonal to restoring two-layer
  gate symmetry.
- **Typing the `required_action` field.** The wire field is `Option<String>` on `InteractionDescriptor`
  (`crates/shared/surfaces/src/interaction.rs:87`) though the descriptor builder already accepts a typed
  `Action` and admission parses to `Action`. Making the stored/authored field `Option<Action>` end-to-end is
  an ergonomics improvement, orthogonal to this enforcement fix. Deferred.
- **Docker** (`2026-07-12-docker-switch-tag-tenant-isolation-design.md`) and **Proxmox**
  (`2026-07-11-proxmox-match-tenant-isolation-design.md`) tenant-isolation IDOR fixes — different bug class
  (cross-tenant DB reads/writes), tracked as their own specs.

## Superseded approach

The original design gated the two `system.settings:manage` reads at the interaction level and added a
feature-gated registry unit test asserting every `system.settings:manage` surface interaction equals
`SYSTEM_SETTINGS_MANAGE_STR`. Review rejected it: (1) it treated the visible symptom (two reads) while the
actual gap was engine-level and spanned six reads plus all runtime service surfaces; (2) the guard enforced
"equals this literal action" (false-positives any validly-different gate) and pinned two hardcoded surface ids
for its non-vacuity assertion (rots on rename, and deleting the assertion silently restores the vacuous pass);
(3) it could never cover runtime service-registered surfaces — pointing at "live two-layer enforcement" as the
compensating control, which on the provider-origin path did not exist. The engine fix supersedes all of it.
