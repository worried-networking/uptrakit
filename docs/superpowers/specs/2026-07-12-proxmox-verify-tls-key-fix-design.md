# Proxmox `verify_ssl` → `verify_tls` Config-Key Fix — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/plugins/infrastructure/proxmox/src/{agent/plugin.rs,config.rs}`. No ADR, no deps, no wire, no
doc change.

## Problem

Audit `audit-2026-07-11` L1052 (MEDIUM · stability · plugins-infra · verified): the proxmox agent reports
`"verify_ssl": true` in the `PluginConfigReport` config JSON (during bootstrap and host-sync token
regeneration), but `ProxmoxConfig`'s field is `verify_tls` with `#[serde(default = "default_true")]` and **no
alias**. The unknown `verify_ssl` key is **silently dropped** on deserialization and `verify_tls` falls back to
its default. Today both are coincidentally `true`, so behavior matches — but it is a loaded latent bug: the day
the agent reports `false` (self-signed PVE certs are the norm for the auto-provisioned `https://node:8006`
endpoint — the end-user doc even says "Set to `false` for self-signed certificates", `docs/end-user/proxmox.md:65`)
the value silently would not apply, and every stored auto-provisioned config already carries a dead `verify_ssl`
key that misleads anyone reading the JSON.

## Verified current reality (byte-checked, 2026-07-12)

- **Two emit sites, the only two `verify_ssl` occurrences in the crate**, both `"verify_ssl": true` inside a
  `json!`/`serde_json::json!` macro in `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs`:
  - `:127` — bootstrap: `PluginConfigReport { plugin_type: "infrastructure_proxmox", name: "pve-{host_id}",
    config: json!({ "api_url", "api_token", "verify_ssl": true }) }`.
  - `:214` — host-sync (`on_host_synced`): `report_plugin_config = Some(PluginConfigReport { … config:
    serde_json::json!({ "api_url", "api_token", "verify_ssl": true }) })`.
- `ProxmoxConfig` (`config.rs`): exactly four fields — `api_url: String`, `api_token: SecretString`,
  `#[serde(default = "default_true")] verify_tls: bool` (`:36-37`), `#[serde(default, skip_serializing_if =
  "Vec::is_empty")] node_filter: Vec<String>`; `fn default_true() -> bool { true }` (`:15`); **no**
  `#[serde(alias = …)]` on the field. The struct has **no `#[serde(deny_unknown_fields)]`** (and
  `PluginConfigReport.config` is a raw `serde_json::Value`), so the extra `verify_ssl` key is silently **dropped**,
  not a hard error — confirming the finding's premise. `verify_tls` has no `skip_serializing_if`, so it always
  serializes back out (no round-trip inconsistency).
- Same-plugin-family precedent for emitting a config from the struct: `routeros/src/plugin.rs:168`
  `serde_json::to_value(RouterOsConfig { … }).map_err(...)?`.
- **The reported config is deserialized into `ProxmoxConfig`** end-to-end: `surfaces.rs:1969`
  `serde_json::from_value::<ProxmoxConfig>(pc.config)`. So the `verify_ssl` key genuinely flows into this struct
  and is dropped there — the fix is a real end-to-end correctness fix, not cosmetic.
- The TLS client already reads the field correctly (`client.rs:52/64` `if config.verify_tls`), the surface form
  field already uses `verify_tls` (`surfaces.rs:265`), and the docs already document `verify_tls`
  (`end-user/proxmox.md:55/65`, `development/proxmox-plugin.md:72/77`). Only the agent **emit** key is wrong.

## Approach (chosen — emit via the struct + heal alias; drift-proof root cause, YAGNI)

The audit flagged the *class* of bug: a hand-maintained JSON string key (`"verify_ssl"`) drifting from the
struct's real serde field name (`verify_tls`). Fixing the string literal alone resolves this one instance but
leaves the same trap — a future `ProxmoxConfig` field rename would silently drift again from the two `json!`
literals. So the emit side is fixed at the **source of truth**: build the struct and serialize it.

1. **Emit via `serde_json::to_value(ProxmoxConfig { … })`** at **both** sites (`agent/plugin.rs:127`, `:214`),
   replacing the hand-written `json!({...})` literals. The emitted key set now derives from `ProxmoxConfig`'s own
   `#[derive(Serialize)]` field/serde names, so it can never drift from the struct again. This matches the
   established same-plugin-family precedent `crates/plugins/package-managers/routeros/src/plugin.rs:168`
   (`serde_json::to_value(RouterOsConfig { … }).map_err(...)?`).

   ```rust
   config: serde_json::to_value(ProxmoxConfig {
       api_url: creds.api_url,
       api_token: SecretString::new(creds.api_token),   // PveCredentials.api_token is String
       verify_tls: true,                                 // value unchanged (see scope boundary)
       node_filter: vec![],                              // skip_serializing_if = "Vec::is_empty" → omitted
   })
   .map_err(|e| report!(PluginError::PluginInternal(format!(
       "failed to serialize proxmox plugin_config: {e}"))))?,
   ```

   This produces the byte-identical payload shape emitted today (`{api_url, api_token, verify_tls}` — `node_filter`
   omitted, verified: `ProxmoxConfig` has exactly those 4 fields, `SecretString` is `#[serde(transparent)]` with a
   *derived* `Serialize` that emits the **plaintext token** — the `***` mask lives only in `Debug`/`Display`,
   which serde never calls (`secret_string.rs:27-30`; the crate's own `serde_roundtrip` test asserts this). So
   `api_token` serializes as the real bare-string token exactly as today's `json!` embedded `creds.api_token` — no
   auth-breaking mask regression). **Implementation note:** the bootstrap site is inside a
   `pve_credentials.map(|creds| PluginConfigReport { … })` closure; `to_value` is fallible (`Result`), so
   restructure that closure to propagate the error — e.g. `if let Some(creds) = pve_credentials { … to_value(…)?
   … }` or `.map(|creds| -> Result<_> { … }).transpose()?`. (`to_value` on these scalar/String/Vec fields
   effectively never fails, but the `Result` must be handled structurally, not `.unwrap()`ed.)

2. **Add `#[serde(alias = "verify_ssl")]`** to the `verify_tls` field (`config.rs:37`), coexisting with the
   existing `#[serde(default = "default_true")]`:

   ```rust
   #[serde(default = "default_true", alias = "verify_ssl")]
   pub verify_tls: bool,
   ```

   The `to_value` change fixes the **emit** side (new configs); the alias **heals the read** side for
   already-stored configs still carrying the legacy `verify_ssl` key. Precisely, it heals the **`false`** case:
   since the agent only ever emitted `true` and `verify_tls` defaults to `true`, no value has actually been lost
   yet (every stored config reads `true` pre- and post-fix) — the alias's real beneficiaries are a
   manually-authored `verify_ssl: false` config and robustness once the deferred value-fix ever emits `false`
   before legacy configs are rewritten. It is one attribute, repo-idiomatic (precedent: `wire/messages.rs:48`,
   `wire/payloads.rs:587/613`), additive and harmless (serde `alias` accepts either name) — cheap future-proofing.

## Tests (extend the existing `config.rs` test module)

- **Renamed key binds a `false`** (the case that was silently dropped): deserialize
  `{"api_url":"…","api_token":"…","verify_tls": false}` → assert `verify_tls == false`.
- **Legacy key heals via the alias**: deserialize `{…,"verify_ssl": false}` → assert `verify_tls == false`.
- **Default preserved**: absent key → `true` (the existing `verify_tls_defaults_to_true` test at `config.rs:332`
  already covers this — keep it).

Covers success (both key spellings) and the default path. No `start_paused` (no tokio-time API). Do not test
serde's own alias/default machinery in isolation — these tests assert *our* struct's contract on the *agent's
actual payload shape*, which is legitimate internal-logic coverage, not upstream-crate testing.

## Scope boundary — this change is BEHAVIOR-NEUTRAL today

**This fix changes nothing observable in current behavior.** Pre-fix, `verify_ssl` is dropped and `verify_tls`
defaults to `true`; post-fix, `verify_tls: true` binds directly — the effective value is `true` either way, for
every config in existence. It is **hygiene + drift-proofing + closing a latent trap** (the day a `false` is
reported), **not** a functional TLS fix. In particular, the genuine user-facing issue — a self-signed PVE
`https://node:8006` endpoint failing verification because the auto-provisioned config carries `verify_tls: true`
— **exists both before and after this change and is untouched here.** Do not frame this as fixing proxmox TLS
onboarding.

The agent still **hardcodes** the value `true` at both sites — this fix corrects only the **key name** (now
derived from the struct) so whatever value is reported binds. Whether the auto-provisioned config *should* report
`verify_tls: false` for a self-signed endpoint is a **separate behavior question with the actual user-visible
impact** — **out of scope**, a distinct follow-up finding about the *value*. Sequencing note: the alias and the
emit-shape only start to matter functionally once that value-fix lands a `false`; until then this is pure
hygiene. Do not change the hardcoded value here.

**`to_value` shape trade-off (accepted):** deriving the emitted keys from the struct means a *future* non-`skip`
`ProxmoxConfig` field with a default would silently start appearing in the emitted config JSON (the `json!`
literal only emitted what was typed). This is accepted: name-drift (the bug that bit) is eliminated, and a stray
extra key is benign (no `deny_unknown_fields`; the harmful direction is the reverse — a key present but not
read). Treat any future non-`skip` field addition as a deliberate change to the agent's emit contract.

## Deliverables

- `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs` — replace both `json!({...})` emit literals
  (`:127`, `:214`) with `serde_json::to_value(ProxmoxConfig { … })` (drift-proof; restructure the bootstrap
  `.map` closure to propagate the `Result`).
- `crates/plugins/infrastructure/proxmox/src/config.rs` — add `alias = "verify_ssl"` to the `verify_tls` serde
  attribute + the three deserialization tests.

### Documentation deliverables

- **No doc change.** The docs already document `verify_tls` correctly (`docs/end-user/proxmox.md:55/65`,
  `docs/development/proxmox-plugin.md:72/77`) — the bug was agent-emit-only, so the docs need no correction.
- **No ADR** (bug fix). **No wire/OpenAPI/frontend/dependency change** — the plugin config JSON is a plugin-
  internal blob (not a wire-protocol type); the DB-stored config shape changes only for *new* auto-provisioned
  configs (legacy ones are read via the alias), and no user-facing API surface changes.

## Alternatives considered

- **Hand-rename the `json!` string key `"verify_ssl"` → `"verify_tls"` (keep the `json!` literals)** — rejected:
  it fixes *this* instance but leaves the two `json!` literals hand-maintaining key names that must stay in sync
  with `ProxmoxConfig`'s serde names — i.e. it preserves the exact bug *class* the audit flagged (a string key
  drifting from the struct field). `to_value(ProxmoxConfig{..})` eliminates the class (keys derive from the
  struct), matching the routeros precedent, for a marginally larger diff.
- **Emit change only, no alias** — rejected: leaves already-stored auto-provisioned configs carrying the dead
  `verify_ssl` key (their value stays dropped). The one-attribute alias heals them at no cost. (Since the agent
  only ever emitted `true` so far, no `false` was actually lost yet — but the alias makes the transition robust
  and future-proofs any manually-authored config.)
- **Rename the `ProxmoxConfig` field to `verify_ssl` instead** — rejected: `verify_tls` is already the name used
  by the client, the surface form, and the docs; the agent emit site is the sole outlier, so it is the thing to
  fix.
- **Change the hardcoded value to `false`** — rejected: out of scope (a value question, not the key drift; would
  be a behavior change needing its own analysis of the auto-provisioned endpoint's cert).

## Out of scope

Other unspecced immediate-Medium findings (core-mqtt-scheduler L911, plugins-infra L1042 frontend SSE param-nav,
ui-cli-surface-proxy L1105 orphaned files, web-api-routes L1226) — separate specs. No change to the hardcoded
`true` value, the TLS-client-construction logic, or the surface form field.
