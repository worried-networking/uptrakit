# PHS Base-OS Compatibility — Design

Date: 2026-08-19
Status: Draft (pending spec review)
Origin: promoted from bead `uptrakit-w9121` ("PHS base-OS compatibility: gate incompatible updates, notify on
mismatch, safe few-click OS upgrade"). That bead's ground-truth investigation and 14-branch stress-tested decisions
are the seed material; this spec covers its phases 2–4. Phase 5 (guided OS upgrade) and the upstream PR are deferred
(see Deferred section, each with a tracking bead).

## Problem

Proxmox helper-scripts (PHS) update scripts declare a supported base OS via `var_os`/`var_version` header variables
(e.g. `var_os=debian`, `var_version=13`). Upstream `misc/build.func` enforces this at runtime with
`check_container_os_guard()`: it compares the container's `/etc/os-release` `ID`/`VERSION_ID` against the script's
declaration and refuses to update on mismatch.

uptrakit is blind to all of this today:

1. **No host OS identity.** Agents report only the human-readable `PRETTY_NAME` (`read_os_version()`,
   `crates/shared/agent-core/src/host_info.rs:137-172`); the machine-readable `ID`/`VERSION_ID` pair is never read,
   sent, or stored. `hosts` has `os_type`/`os_version` display columns only
   (`crates/shared/db/src/entity/host.rs:6-24`).
2. **No requirement capture.** PHS discovery fetches and analyzes each script (`analyze_phs_script`,
   `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs:327`) but `PhsScriptAnalysis`
   (`discovery.rs:156-186`) has no `var_os`/`var_version` fields — `grep var_os` over the crate returns nothing.
3. **False success.** The synthesized update command is `sudo PHS_SILENT=1 TERM=xterm /usr/bin/update`
   (`plugin.rs:74`). Under silent mode, upstream's guard _function_ returns 1, but every call site invokes it as
   `check_container_os_guard || return 0` — the whole process **exits 0** when the update is skipped. uptrakit
   records a Completed update that never ran. (Known gotcha: bd memory `gotcha-phs-usr-bin-update-in-headless-mode`.)

So a user on an outdated container base OS sees "update available", clicks update, gets "Completed", and nothing
happened — silently, forever.

## Goals

- Track each Linux host's machine-readable OS identity (`ID` + `VERSION_ID`).
- Capture each PHS item's declared base-OS requirement and store it per host/software-item link.
- Block dispatch of updates that are destined to fail the upstream guard, with a truthful HTTP 409 and a visible
  reason — before any command runs on the host.
- Let the user explicitly acknowledge a mismatch and proceed anyway — and have the acked dispatch engage upstream's
  own bypass (`var_ignore_os_mismatch=1`) so the update actually runs instead of being refused host-side.
- Detect the upstream silent skip when it does happen and record the update as **Failed** with a `recovery_hint`,
  never Completed.
- Notify (edge-triggered) when an item becomes base-OS incompatible.

## Non-goals (this spec)

- Guided base-OS upgrade flow (synthesized "Base OS" software item, embedded upgrade script) — deferred, bead
  `uptrakit-def-base-os-upgrade-path`.
- Upstream PR to make the silent-mode skip distinguishable by exit code — deferred, bead
  `uptrakit-def-phs-upstream-guard-exit`.
- Base-OS requirements for non-PHS plugins. The data model is plugin-agnostic (`source` field), but only PHS
  populates it in this cycle.
- Gating version _checks_ or discovery — only update dispatch is gated.

## Terminology

Canonical terms added to `CONTEXT.md` in this cycle: **Host OS Identity**, **Base-OS Requirement**, **Base-OS Ack**.
Use them exactly; in particular do not say "compatibility" bare — plugin runtime compatibility
(`DetectHostCompatibility`) is a different axis (see "Relation to existing mechanisms").

## Ground truth (verified 2026-08-19, against a fetched copy of `misc/build.func`)

Upstream `community-scripts/ProxmoxVE`, `misc/build.func`:

- `check_container_os_guard()` (~line 4025) **passes only** when the container's `ID` equals `var_os` AND its
  `VERSION_ID` equals `var_version` or extends it on a dot boundary (`cur_ver == rec_ver` or
  `cur_ver == rec_ver.*`). _Every_ other combination is refused — including a host **newer** than the requirement
  (the guard computes a "downgraded target" flag for messaging, then blocks anyway). If either side of the compare
  is empty (`var_os`/`var_version` undeclared, or the container's os-release unreadable), the guard returns 0 and
  the update proceeds — an undeclared requirement is inert, not blocking.
- On mismatch in silent mode it prints
  `msg_error "Container OS ${cur_os} ${cur_ver} does not match the recommended ${rec_os} ${rec_ver} — skipping update."`
  and returns 1; the caller (`start()`) swallows it: `check_container_os_guard || return 0`. **Process exit code is 0
  either way** — detection must key on output text, never exit status.
- The same `|| return 0` shape guards `runtime_script_status_guard` (~line 3966), which has **two** exit-0
  refusals with identical mechanics: a retired script prints
  `msg_error "This script is no longer available in community-scripts."` (`:3966`) and a disabled one prints
  `msg_error "This script is currently disabled in community-scripts."` (`:3985`; its own machine bypass is
  `var_ignore_disable`, parallel to the OS one) — the second and third false-Completed signatures.
- Bypasses: env `var_ignore_os_mismatch=1|yes|true|on` (checked at `:4049`, _after_ the compare and after the
  guard's early `return 0` branches — on mismatch it warns, then continues), or the persistent file
  `/usr/local/community-scripts/ignore-os-mismatch` containing exactly `"${rec_os,,} ${rec_ver}"` (upstream
  lowercases the OS id when writing it) — honored only while it matches the _current_ recommended pair, i.e. it
  self-re-arms when the script's target OS changes. The file branch is only offered interactively (whiptail,
  non-silent); the env var is the machine-usable bypass. Our Base-OS Ack copies the file's keying (the lowercased
  requirement pair) and its re-arming semantic, and an acked dispatch engages the env bypass (see Acked dispatch
  below).
- **One command, possibly many scripts.** A container's `/usr/bin/update` may reference multiple CT scripts —
  uptrakit's own parser deliberately supports and deduplicates multiple slugs per file (`parse_phs_scripts`,
  spec D4 bare-slug identity) — and every PHS-discovered item on a host shares the identical dispatch command
  constant. Running it executes the whole file, and _each_ sourced script runs its own guards. Ack scope, bypass
  blast radius, and failure attribution are therefore designed against this N-scripts reality, never a 1:1
  assumption (see Acked dispatch and Truthful failure).

## Design overview

Three phases, each independently shippable; nominal order A → B → C, with one sanctioned reordering (the
fallback carve-out in the ordering note below):

| Phase                       | Deliverable                                                                                                                                                                              |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A — Foundation              | Host OS Identity end-to-end (agent → wire → DB); Base-OS Requirement capture (PHS parse → wire → `host_software_items` column)                                                           |
| B — Gate + truthful failure | Dispatch-time 409 gate with upstream-mirror compare; Base-OS Ack endpoint + acked-dispatch bypass injection; output-signature fallback flipping silent skips to Failed + `recovery_hint` |
| C — Notification            | Edge-triggered `BaseOsIncompatible` notification event                                                                                                                                   |

Ordering note: the output-signature fallback inside Phase B may still land first as its own increment — it keys on
the dispatched command constant and the recorded output, and its attribution rule degrades gracefully: with no
stored requirement yet (pre-Phase A), every signature falls back to the single-PHS-item rule (see Truthful
failure). It alone converts today's false-Completed outcomes into truthful Failures on single-script hosts — the
overwhelmingly common shape. Two consequences of landing it first are accepted and stated: (a) signature 1's
`recovery_hint` must be increment-appropriate — until the ack endpoint exists (Phase B), the hint names the base-OS
mismatch and points at the item's base-OS state without promising an ack affordance; Phase B updates the constant
to the ack wording; (b) failures that were previously silent Completeds become visible `UpdateFailed`
notifications (`event_types.rs:24`) with no remedy path in that increment — truthful and intended, called out in
the increment's release note.

### Phase A — Foundation

#### Host OS Identity (agent side)

`collect_host_info` (`crates/shared/agent-core/src/host_info.rs`) gains parsing of `/etc/os-release` `ID` and
`VERSION_ID` (Linux only; `None` elsewhere). Values are passed through verbatim after stripping quotes — no
normalization, no enum: distro IDs are an open set.

Wire: `HostInfo` (`crates/shared/wire/src/payloads.rs:64`) gains `os_id: Option<String>` and
`os_version_id: Option<String>`. `WireValidate` impl (`wire_validate_impls.rs:185-195`) extends its
`check_opt_string_len` list with both fields against `MAX_SHORT_STRING_LEN` (no new limit constant needed — same
tier as the existing `os_type`/`os_version`). Additive optional fields ⇒ old agents interop cleanly (absent = NULL).
Regen `crates/shared/wire/asyncapi.yaml` via `./scripts/regen-asyncapi.sh` and update
`docs/api/wire-protocol.md`.

DB: migration `m20260819_000001_add_host_os_identity.rs` (seq adjusted to next free at implementation time;
pattern per newest existing `m20260812_000003_encrypt_instance_plugin_setting_config.rs`) adds nullable `os_id`, `os_version_id` TEXT columns to
`hosts`; entity + host upsert path updated. Existing `os_type`/`os_version` (PRETTY_NAME) stay as display fields.

#### Base-OS Requirement (PHS side)

New shared value type in `uptrakit-shared-types`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct BaseOsRequirement {
    /// os-release `ID`-style identifier declared by the installer, e.g. "debian".
    pub os: String,
    /// os-release `VERSION_ID`-style version, e.g. "12". May be empty upstream; empty = version-unconstrained.
    pub version: String,
    /// Which plugin/mechanism declared it, e.g. "phs".
    pub source: String,
    /// When the declaration was captured.
    pub declared_at: time::OffsetDateTime,
}

impl BaseOsRequirement {
    pub fn new(os: String, version: String, source: String, declared_at: time::OffsetDateTime) -> Self { /* … */ }
}
```

(Derive line and `cfg_attr` schema gates copied from the crate's existing `DiscoveryTarget` — the type must
participate in both openapi and schemars codegen. `DiscoveryTarget` itself carries neither `#[non_exhaustive]` nor
a constructor (a pre-existing gap, not imitated here); this type adds both. `time::OffsetDateTime` is the workspace's sole datetime type;
`chrono` is not a workspace dependency and is not introduced. The `new()` constructor is mandatory, not optional:
the coding-standards `#[non_exhaustive]`-struct rule requires every such public struct in a shared crate to expose
a constructor or `Default`, and this one is constructed outside its defining crate — by the PHS plugin's
`analyze_phs_script` path.)

- `PhsScriptAnalysis` (`discovery.rs:156-186`) gains the analysis outcome; `analyze_phs_script` parses the
  `var_os=`/`var_version=` header lines with the same line-scanning style as the existing field extraction.
  Quoted and unquoted values both accepted. Outcome mapping: parsed non-empty `var_os` ⇒ `Declared` (an empty
  `var_os=""` must not become a requirement with `os: ""` — see evaluation rule 2); no `var_os` line at all, or
  an explicitly empty one ⇒ `HeaderAbsent` (upstream treats both as guard-inert undeclared); a `var_os` line
  present but unparseable ⇒ `ParseFailed`.
- Discovery targets carry the requirement to the controller. Version-check results do **not**: PHS version
  detection is a local helper (`PHS_DETECT_VERSION_CMD` runs `/usr/local/bin/uptrakit-phs-version`,
  `plugin.rs:52-53`) that never fetches or analyzes CT scripts, so a version-check enrichment leg would have no
  producer — discovery is the sole analysis and refresh path, on its existing event-driven + 6h cadence.
  Wire: the PHS-emitted discovery payload item gains `base_os_analysis: Option<BaseOsAnalysis>` (serde-defaulted
  `None`; exact wire struct nesting decided at plan time from the existing `DiscoveryResults` item shape):

  ```rust
  #[non_exhaustive]
  pub enum BaseOsAnalysis {
      /// Script fetched and parsed; it declares this requirement.
      Declared(BaseOsRequirement),
      /// Script fetched and scanned; no usable var_os/var_version header exists (absent or empty var_os).
      HeaderAbsent,
      /// Header lines are present but could not be parsed (malformed value, or a parser regression).
      ParseFailed,
      /// Forward-compat catch-all: a variant from a newer producer this build does not know.
      #[serde(other)]
      Unknown,
  }
  ```

  Persistence rule (five outcomes, replacing the earlier bool+`Option` tri-state): `Declared` ⇒ upsert;
  `HeaderAbsent` ⇒ **clear** the stored requirement (upstream really removed the declaration — a stale stored
  pair must not keep blocking or keep an ack live); `ParseFailed` ⇒ leave the stored value untouched and
  `tracing::warn!` (one upstream reformat or a parser bug must not silently null the fleet's requirements);
  `Unknown` ⇒ leave the stored value untouched and `tracing::warn!` (a newer agent sent a variant this
  controller does not know — degrade, never destroy state); `None` (script fetch failed, or an old agent
  omitting the field) ⇒ leave untouched. `WireValidate`: bound `os`, `version`, `source` by
  `MAX_SHORT_STRING_LEN` via the `Declared` payload. Forward-compat shape (binding): the coding-standards
  wire-enum rule requires every wire-serialized enum that may gain variants to carry a catch-all, and the
  top-level precedent already does exactly this — `ControllerMessage`/`ServiceMessage`
  (`crates/shared/wire/src/messages.rs`) are `#[non_exhaustive]` data-carrying wire enums each ending in a unit
  `Unknown` variant marked `#[serde(other)]`, documented as "log a warning and continue". `BaseOsAnalysis`
  mirrors that shape exactly: internally tagged serde (`#[serde(tag = "...", rename_all = "snake_case")]`, the
  same representation `ServiceMessage` uses — `#[serde(other)]` requires a tagged form, and `Declared`'s struct
  payload serializes as a map, satisfying internal tagging's map-merge requirement) plus the unit `Unknown`
  catch-all above; verify against `WireValidate`/schemars codegen at plan time. The catch-all must live on
  `BaseOsAnalysis` itself because the enum is nested inside the
  `DiscoveryResults` payload — the outer `ServiceMessage::Unknown` catches only unknown _top-level_ tags;
  without a local catch-all, one unknown nested variant would fail deserialization of the entire
  `DiscoveryResults` message on an older controller. Regen asyncapi + wire-protocol doc.

- DB: same migration family adds `base_os_requirement` (JsonBinary, nullable) to `host_software_items` (entity
  `crates/shared/db/src/entity/host_software_item.rs`) and `base_os_acks` (JsonBinary, nullable) to `hosts` — a
  JSON array of acknowledged requirement-pair strings (see Phase B; per-(host, pair) scope mirroring upstream's
  per-pair, per-container ignore-file keying). Each pair entry is a _comparison
  key_ consumed by string equality only, but the column holds a set of them, hence a JSON array rather than a
  TEXT scalar. Per the repo's structured-JSON-column idiom (documented on `access_grant.rs` — raw
  `serde_json::Value` on the entity, typed only at the query-module boundary, no `FromJsonQueryResult`), both
  entity columns are `Option<serde_json::Value>`; query modules parse them into `BaseOsRequirement` /
  `Vec<String>`.
- Persistence point (single): discovery reconcile writes the requirement on create and refreshes it on
  re-discovery, applying the analysis-outcome rule above. This is a metadata column, not a version column — the
  ADR-0037 "never overwrite non-NULL `installed_version`" rule does not extend to it; latest declaration wins.
  There is deliberately no version-check leg (see the wire bullet), so requirement staleness is bounded by the
  discovery cadence — acceptable for metadata whose gate fails open.

#### Compatibility evaluation (pure function, shared)

One pure function, home in `uptrakit-shared-types` next to `BaseOsRequirement`:

```rust
#[non_exhaustive]
pub enum BaseOsCompatibility {
    /// No requirement stored, or no host identity — never block on ignorance.
    Unknown,
    /// The upstream guard will pass (including its guard-inert empty-field cases).
    Compatible,
    /// The upstream guard will refuse. `kind` shapes messaging only — every kind blocks and is ack-able.
    Mismatch { host: String, required: String, kind: BaseOsMismatchKind },
}

#[non_exhaustive]
pub enum BaseOsMismatchKind { DifferentDistro, HostOlder, HostNewer, Undetermined }

pub fn evaluate_base_os(host_os_id: Option<&str>, host_version_id: Option<&str>,
                        req: Option<&BaseOsRequirement>) -> BaseOsCompatibility
```

(`#[non_exhaustive]` per the project's cross-boundary enum convention — same treatment as `HostCompatibility` and
`PluginRole`.)

Rules (each is one test fixture — see Testing):

1. `req` is `None`, or host identity is `None` ⇒ `Unknown` (fail open).
2. Empty `req.os` **or** empty `req.version` ⇒ `Compatible` — upstream's guard returns 0 when either side is
   undeclared, and this verdict exists to predict the guard; guard-inert means the update will run. (The parse
   layer already refuses to store an empty `var_os`; this is defense in depth, and empty `req.version` is a real
   upstream state.)
3. `req.os != host os_id` (case-insensitive exact compare) ⇒ `Mismatch` with kind `DifferentDistro`.
4. Same distro: the verdict is **upstream's own predicate, verbatim** — pass iff host `VERSION_ID` equals
   `req.version` or starts with `req.version + "."` (dot-boundary prefix; host `12.5` satisfies required `12`).
   No numeric parsing, no ordering, no semver in the verdict.
5. Anything else ⇒ `Mismatch`. `kind` is best-effort wording only: numeric dot-segment compare where both sides
   parse (host < required ⇒ `HostOlder`, host > required ⇒ `HostNewer`), `Undetermined` when either side has a
   non-numeric segment. The kind never changes blocking behavior.

**The verdict mirrors upstream exactly.** The gate exists to predict `check_container_os_guard`; any divergence
produces either a false 409 (we block, guard would pass) or a false Failed (we allow, guard refuses host-side and
the fallback records Failed). The dominant real-world case — a Debian 13 host running a script still declaring
`var_version=12` — is a _newer_ host, and upstream blocks it; so do we, ack-ably. Likewise required-longer
(`13` vs required `13.1`) and non-numeric inequality block, because the upstream prefix test fails for them.
Direction awareness survives only in `kind`, which selects message wording and lets the ack dialog distinguish
"host is newer than the script targets — acking is usually safe" from "host is older — the update may genuinely
fail". (One representational nuance, not a divergence: an unreadable host os-release makes upstream's guard
proceed, while we return the distinct `Unknown` variant — both are non-blocking, so dispatch behavior matches;
`Unknown` merely preserves "we don't know" for the UI instead of asserting compatibility.)

**No `semver`, no version parse in the verdict**: os-release `VERSION_ID` values (`12`, `24.04`, `9.4`) are not
semver, and the upstream predicate needs only string equality + dot-boundary prefix. The numeric-segment compare
feeding `kind` is ~10 lines of wording-only code.

### Phase B — Dispatch gate + truthful failure

#### Gate

`load_target_for_dispatch` (`crates/ui/web-api-queries/src/queries/update_dispatch.rs:858-965`) gains one
precondition alongside the existing inline `report!(TriggerUpdateError::...)` checks: evaluate
`evaluate_base_os(...)` from the already-loaded host row + link row. On `Mismatch`, consult the host's
`base_os_acks` set: if it contains the current requirement pair (`"{req.os} {req.version}"` with `os` lowercased —
the same content and keying as upstream's ignore-file, which writes `${rec_os,,}`; the space-joined pair is
theoretically ambiguous if an OS ID ever contained a literal space, an accepted risk inherited from upstream's own
scheme — os-release `ID`/`VERSION_ID` values are
space-free tokens in practice), the gate **passes with the ack engaged** — the loaded dispatch target carries a
`base_os_acked: bool` so command synthesis injects upstream's bypass (next subsection). Otherwise fail with a new
variant:

```rust
#[error("host base OS {host} does not satisfy required {required} for this update")]
BaseOsMismatch { host: String, required: String },
```

No `ack_available` field: every blocking mismatch is ack-able by design, so the variant's presence _is_ the
affordance. Mapping chain: `TriggerUpdateError` → `map_trigger_error`
(`crates/ui/controller-core/src/update/controller.rs:604-624`) → new `UpdateDispatchError` variant →
`impl From<Report<UpdateDispatchError>> for ApiError` (`crates/ui/web-api/src/api_error/mappings.rs:1021-1077`) ⇒
**HTTP 409**. ⚠️ That last mapping does **not** self-enforce: `UpdateDispatchError` is `#[non_exhaustive]`
(`crates/ui/controller-core/src/update/mod.rs:59-61`) and the `mappings.rs:1021-1077` match already carries the
convention-required wildcard arm falling back to a `tracing::warn!`-logged HTTP 500 — so adding the variant
compiles cleanly and maps to the wrong status (500, not 409) unless an explicit
`BaseOsMismatch ⇒ ApiError::new(StatusCode::CONFLICT, …)` arm is added _before_ the wildcard. That arm is an
explicit work item, not a compiler-forced one. The `TriggerUpdateError` matches, by
contrast, ARE compiler-forced: `trigger_audit_classification()` / `batch_trigger_audit_classification()` in
`update_dispatch.rs` and the direct `impl From<Report<TriggerUpdateError>> for ApiError` (`mappings.rs:491`) are
all exhaustive with no wildcard, so adding the variant breaks the build until each handles it.
The response uses the existing `ApiError` body shape (`{ error, code }` — it carries no other
structured fields, and this design does not extend it): `code: "base_os_mismatch"` is the machine discriminant the
frontend branches on; the `error` message names both identity pairs for display; the pairs are also independently
available to the UI from the link's `base_os_requirement` and the host's identity fields in their regular API
responses. `Unknown` and `Compatible` never block.

The gate lives once, in `load_target_for_dispatch`, but its failure surfaces through **six** distinct channels
(raw call sites: the `validate_update_preconditions` wrapper at `update_dispatch.rs:985` — shared by the first
three channels below — plus `update_batches/dispatch.rs:162`, `service_ws/handler/updates/dispatch.rs:344`, and
`updates/replay.rs:46`) — each channel gets defined semantics (no silent divergence):

- **Single synchronous trigger** (`routes/software_items/updates.rs`): HTTP 409 + ack flow as above.
- **Batch creation** (`update_batches/mod.rs:116`): existing partial-success contract — the item is skipped with
  the mismatch message in `BatchSkippedItem.reason`; no 409 (batch responses are 200 by contract). The UI shows the
  reason; acking happens from the item's own view.
- **Background dispatch of a queued item** (`service_ws/handler/updates/dispatch.rs:344` via
  `fail_dispatch_target_load`): the pending record is finalized **Failed** — truthful, since dispatch is refused —
  with the mismatch message in `output` and `recovery_hint` populated (extend `fail_dispatch_target_load` to map
  `BaseOsMismatch` to a hint naming the ack affordance). This path hits the gate only when compatibility changed
  _between queueing and dispatch_ (requirement or host identity moved); the user acks from the item view and
  re-triggers. This preserves the "explicit ack" goal — the ack is never bypassed, merely asynchronous.
- **Queued progression / recovery dispatch** (`dispatch_next_queued_for_host`,
  `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:162` — reached from `dispatch_next_in_batch`
  (batch progression), `restart_progression.rs:112`, `boot/recovery.rs:124`, and `scheduler/mod.rs:252`): this is a
  **separate, parallel implementation** of "dispatch next queued" living in `web-api-queries`, distinct from the
  `service_ws` one above. Its inline error branch already finalizes the record Failed with the real message in
  `output` (`"dispatch failed: {e}"`) but never sets `recovery_hint`. In scope: extend that branch to populate
  `recovery_hint` for `BaseOsMismatch` with the same hint mapping as `fail_dispatch_target_load`, so batch
  progression, restart, boot recovery, and scheduler redispatch match every other channel's contract.
- **Reconnect replay** (`updates/replay.rs:46`): same Failed-with-`recovery_hint` outcome, but the error branch
  here calls `fail_unreplayable_pending_update` (`replay.rs:391`), **not** `fail_dispatch_target_load` — and today
  that function writes a hardcoded generic reason and discards the load error entirely. In scope: extend
  `fail_unreplayable_pending_update` to accept the load error, persist its real message in `output`
  (falling back to the current generic text for non-gate errors), and populate `recovery_hint` for
  `BaseOsMismatch` with the same hint mapping as `fail_dispatch_target_load`. Without this, replay-path gate hits
  would produce exactly the reason-less silent failure this design exists to eliminate.
- **Service-triggered single update over WS** (`ServiceTriggerUpdate`, handled by `handle_service_trigger_update`
  in `service_ws/handler/update_tracking.rs` → `trigger_update_for_host` → `validate_update_preconditions` →
  `load_target_for_dispatch` — a direct call site, _not_ routed through `UpdateDispatcher`/`ApiError`): the handler
  already replies `ControllerMessage::Error(ErrorPayload { code: BadRequest, message: err.to_string() })`, so the
  `BaseOsMismatch` Display message (host + required identity) reaches the service actor with no structural change.
  Semantics: generic WS error code, human-readable mismatch message; no ack affordance over the wire — acking is a
  UI/API action. MQTT-triggered updates arrive on this path; the MQTT service's host-batch sibling
  (`handle_service_trigger_host_batch_update`) delegates to batch creation and is covered by that bullet.

A host may additionally carry upstream's own persistent ignore-file — then the guard would pass even though
uptrakit still shows a mismatch. The gate is deliberately conservative and never reads host state: it blocks until
acked in uptrakit. Acceptable divergence; one sentence in the update docs.

#### Acked dispatch — bypass injection

A controller-side ack alone cannot make the update run: the dispatched command is upstream's own script, and its
guard re-checks on the host — an acked-but-stock dispatch would be refused there, and the fallback would record it
Failed (the ack would merely convert a 409 into a Failed). An acked dispatch therefore engages upstream's machine
bypass:

- New constant next to `PHS_INSTALL_CMD` (`plugin.rs:74`):
  `PHS_INSTALL_CMD_ACKED = "sudo PHS_SILENT=1 TERM=xterm var_ignore_os_mismatch=1 /usr/bin/update"`.
- When the gate passed with the ack engaged **and** the resolved execute_update config's `update_command`
  string-equals `PHS_INSTALL_CMD`, command synthesis substitutes `PHS_INSTALL_CMD_ACKED` — a const-for-const swap
  keyed on exact equality, never string surgery on arbitrary commands. A user-customized command is never touched
  (the swap simply doesn't apply; the upstream guard then decides, and the fallback reports truthfully).
- The swap sits on the shared path that renders the resolved config into the dispatch wire payload, so every
  dispatch channel (single trigger, batch, queued progression, replay, service WS) gets it identically; the exact
  function is a plan-time pick.
- Sudoers already permits it: the PHS entry is `NOPASSWD: SETENV:` on `/usr/bin/update` — the same entry that
  passes `PHS_SILENT=1`/`TERM` through today. The grant is generated from the plugin's own declaration:
  `required_sudo_commands()` builds the `/usr/bin/update` entry `.with_setenv()`
  (`crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs:421-438`; the sudoers shape is documented in the
  same file's doc comment, `:69-72`). Same command path, no allowlist change, no agent change.
- Upstream honors `var_ignore_os_mismatch=1` at `:4049` — on mismatch it warns, then continues (see Ground truth).
- **Blast radius is the container, not the item — and wider than upstream's file.** The bypass env is
  process-wide and `/usr/bin/update` may source several scripts (see Ground truth): an acked dispatch bypasses
  the OS guard for **every** script in that container run, not only the acked item's. Upstream's own ignore-file
  is _narrower_: it is honored only while its content equals the _currently-sourced_ script's recommended pair
  (`build.func:4058-4061`) — a sibling script declaring a different pair is still refused — whereas
  `var_ignore_os_mismatch` bypasses every pair in the run. The per-(host, pair) ack set mirrors the file's
  keying, but the injected env cannot express pair granularity, so the gate closes the gap controller-side:
  **when the ack-engaged swap would apply, the gate additionally evaluates every PHS-managed link on the host**
  (the same batched link-load + config-resolution pass the finalize path specifies as attribution input 3 —
  reuse it) **and refuses with 409 if any sibling's declared pair evaluates `Mismatch` and is not in the acked
  set**, enumerating the unacked pairs and their item names in the error message. The ack dialog enumerates the
  same affected items up front, so the user acknowledges the bypass's real coverage together, and the audit
  trail (one Stateful row per acked pair) stays attributable to everything the bypass reaches. The update docs
  state the container-wide reach plainly.
- Nothing persisted changes at dispatch: an un-acked dispatch always sends the stock command, and the injected env
  var is a fixed literal — no user input reaches the command line (no-shell-injection rule).

#### Base-OS Ack endpoint

`POST /api/v1/software-items/{id}/hosts/{host_id}/ack-base-os` (exact path segment ordering aligned at plan time
with the existing trigger route shape in `crates/ui/web-api/src/routes/software_items/updates.rs`; structural
exemplar: `approve_software_item`, `crates/ui/web-api/src/routes/software_items/crud.rs:565` — copy its shape but
**not** its `State<Arc<AppState>>` extractor: the new handler declares focused sub-state(s) per the web-api
handler-state rule).

- Authorization: reuse `CanTriggerUpdates` (`crates/ui/web-api/src/middleware/action.rs:266`, action
  `updates:trigger`). Rationale (settled in grilling): acking is a decision _about triggering_ this update; a
  separate catalog action would gate the same human intent twice. No new action string.
- Behavior: load the link via
  `TenantDb::find_via_tenant_join::<host_software_item::Entity, host::Entity>` (`host_software_items` carries no
  `tenant_id` — it is a global join table, so the plain `find` helpers do not apply; existing exemplar
  `crates/ui/web-api-queries/src/queries/hosts.rs:108`); recompute `evaluate_base_os`; if the verdict is not
  `Mismatch` ⇒ HTTP 409
  ("nothing to acknowledge" — prevents stale pre-acks); else append `"{req.os} {req.version}"` (lowercased `os` —
  the exact pair the mismatch was evaluated against) to the host's `base_os_acks` set (idempotent — an
  already-present pair is a no-op success) in a `begin_immediate()` transaction with a Stateful audit emit
  (`emit_stateful` exemplar: `crates/ui/web-api/src/routes/software_items/host_assignments.rs:201`; new
  `audit-catalog.toml` entry; before/after snapshot of the host's acks field via `AuditView` — each audit row
  carries host, pair, and actor, so the override stays attributable). The route keeps the item+host path (the UI
  entry point is the item row) but the recorded fact is host-level.
- Scope: per-(host, pair) — one ack covers every item on that host declaring the same pair, by design: it
  mirrors upstream's ignore-file keying (per-pair, per-container); the injected env bypass reaches _wider_ than
  one pair, which the gate's sibling-pair check closes (see Acked dispatch). Acked
  pairs are kept forever, mirroring upstream's persistent file; no pruning — a pair re-declared later is still
  acked, and the audit log records when and by whom each pair was accepted.
- Re-arming is automatic and keyed to the _fact being overridden_: an ack stays valid across `latest_version`
  bumps as long as the declared requirement pair is unchanged (same lifetime as upstream's ignore-file), and
  re-arms the moment the requirement changes — a new pair is simply not in the set. A `HeaderAbsent` clear nulls
  the requirement and with it any chance of a pair match. No TTL, no cleanup job.
- OpenAPI: register the operation, run `./scripts/regen-api.sh`, commit `openapi.json` + generated frontend client;
  add the method to `uptrakit-openapi-client` (client-parity rule).
- Classify the new handler as `tenant-scoped` in `crates/ui/web-api/db_access_policy.toml` in the same commit
  (enforced by `python3 ci/verify_db_access_policy.py`).

#### Truthful failure (output-signature fallback)

Even with the gate, silent skips can happen (stale stored requirement, cleared requirement not yet re-analyzed,
upstream guard changes). In the update finalization path (`finalize_update_result_if_owned`,
`crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:919-968`, and its sibling
`finalize_batch_item_if_owned` at `1088+`), when the incoming final status is success **and** the update is
PHS-dispatched, scan the recorded output for the upstream skip **signature set**.

**Discriminator — the command constant, not the plugin type.** PHS is a Discoverer-only plugin: the targets it
emits carry `plugin_type: GENERIC_SHELL` (`plugin.rs:299-313`, `:401-415`), so a "plugin type is PHS" check can
never match anything. The one artifact that uniquely identifies a PHS-managed item is the command it dispatches:
the update is PHS-dispatched iff the resolved execute_update `update_command` string-equals `PHS_INSTALL_CMD`
(const equality; a user-customized command opts the item out of the fallback, correctly — we no longer know what
its output means). `PHS_INSTALL_CMD_ACKED` never enters this comparison: the acked swap is a transient render-time
substitution and is never persisted, so the resolved config always reads `PHS_INSTALL_CMD` — one constant suffices.

**Attribution — never bare.** One dispatched command may run several scripts (see Ground truth), and a skip
printed by script X must not flip item Y to Failed:

- Signature 1 embeds the recommended pair in its message
  (`… does not match the recommended ${rec_os} ${rec_ver} — skipping update.`): it flips the dispatched item to
  Failed only when that embedded pair equals the item's own stored `base_os_requirement` pair
  (case-insensitive; implemented as searching for the signature with the item's pair interpolated, not by parsing
  arbitrary output). With no stored requirement (pre-Phase A, or cleared), fall back to the single-item rule
  below.
- Signatures 2–3 carry no script identity: they apply only when the host has exactly **one** PHS-managed item
  (a single link whose resolved `update_command` equals `PHS_INSTALL_CMD`) — the 1:1 case, overwhelmingly common.
  On a multi-item host the result stays as reported and a `tracing::warn!` names the ambiguity; a guessed Failed
  is worse than a logged uncertainty.
- **Residual mitigation — the miss self-heals.** When attribution declines on a multi-item host (or the printed
  pair doesn't match a stale stored pair), the false Completed survives — but a skipped update never changes the
  installed version, so the next version check re-reports the item as update-available. The damage of an
  unattributed skip is bounded to one check cycle, not "silently, forever" as today.

**Signature set** (designed as a set from day one; each entry carries its own `recovery_hint`):

1. `does not match the recommended` — the OS-guard skip (common core of the silent- and interactive-mode
   messages), pair-attributed as above. Hint: base OS mismatch reported by the installer; ack in uptrakit
   (re-arms when the declared requirement changes) and re-trigger. (Phase-B wording — the fallback-first
   increment ships a variant without the ack reference; see the ordering note.)
2. `This script is no longer available in community-scripts.` — `runtime_script_status_guard`: upstream retired
   the script; same exit-0 mechanics. Hint: the item is no longer updatable via PHS; re-run discovery or remove
   the item.
3. `This script is currently disabled in community-scripts.` — `runtime_script_status_guard`'s second refusal:
   upstream temporarily disabled the script (its own bypass env `var_ignore_disable` is out of scope — never
   injected). **Co-occurrence guard**: the byte-identical string is also printed as a `msg_warn` on the
   bypass-_success_ path (`build.func:3975-3982` — under `var_ignore_disable` the run continues), so this
   signature matches only when the normalized output does **not** also contain
   `Bypassing disable status via var_ignore_disable`. Hint: the script is disabled upstream; retry later or
   check the community-scripts repository.

**Haystack and normalization**: scan the final joined output string being persisted — the same value
`select_best_output` produces (`result.rs:68-75`; rows are concatenated with no separator, so per-row scanning
could split a signature across a boundary; the joined value is scanned once). Before matching, strip ANSI escape
sequences and `\r` (upstream's `msg_error` colorizes its output).

Required plumbing (none of this exists today — explicitly in scope): both finalize args structs
(`FinalizeUpdateResultIfOwnedArgs`, `dispatch.rs:907-916`; `FinalizeBatchItemIfOwnedArgs`, `:1077-1085`) gain
**three** attribution inputs, resolved at **both** wire-result entry points and threaded through: the
single-result handler (`handle_update_result`, `routes/service_ws/handler/updates/result.rs`) populates
`FinalizeUpdateResultIfOwnedArgs`; the batch-result path (`handle_batch_update_result` →
`process_single_batch_result`, `routes/service_ws/handler/updates/batch.rs`) populates
`FinalizeBatchItemIfOwnedArgs` per item. In the batch path, the per-batch host set is
obtained by batch-fetching all of the message's `update_history` rows in one `.is_in(update_history_ids)` query
up front and grouping by `host_id` — today `process_single_batch_result` learns host identity via a per-item
`find_by_id` on `update_history` (`BatchUpdateItemResult` carries no `host_id`), so this prefetch both supplies
the host set and removes that pre-existing per-item lookup. The host's link load + config resolution (the
batched pass input 3 is derived from, which also carries each item's stored pair for input 2) then runs **once
per unique host per batch message** before the per-item loop and is reused across that host's items —
`process_single_batch_result` must not recompute the host PHS-count or re-query links or history rows per item
(batch-query rule):

1. the PHS-dispatched discriminant (or the resolved signature set), from the **persisted resolved
   execute_update config** (the same `resolve_effective_config()` chain dispatch uses — no dispatched-command
   persistence exists or is added; the transient acked swap makes the persisted constant the sole stable
   artifact);
2. the dispatched item's own stored `base_os_requirement` pair (signature 1's attribution key);
3. the host's PHS-managed-item count reduced to a single/multi flag (signatures 2–3's scope condition) —
   computed in **one batched pass**: load all of the host's `host_software_items` links with one
   `.is_in`-filtered query, resolve each link's execute_update config in memory, count
   `update_command == PHS_INSTALL_CMD`. Never per-link queries inside the finalize path (batch-query rule).

Both finalize functions additionally gain a `RecoveryHint` column write (their current `col_expr` chains set only
status/output/version columns). On signature hit:

- Persist status **Failed** instead, with the matching signature's `recovery_hint` (existing column,
  `entity/update_history.rs:52`).
- The signature constants live next to `PHS_INSTALL_CMD` in the PHS plugin crate and are exposed to the finalize
  path via the plugin catalog rather than hardcoded in `web-api-queries` (keeps the strings' canonical home with
  the plugin that owns the upstream contract; exact plumbing — a descriptor-level
  `failure_output_signatures()`-style hook or shared constants — is a plan-time decision, defined once).
- Exit code is **never** consulted for this (it is 0 in all cases — see Ground truth).

**Signature drift canary**: an `#[ignore]`d network test in the PHS crate fetches upstream `misc/build.func` and
asserts all three signature substrings still appear inside `msg_error` lines (including signature 1's
`${rec_os} ${rec_ver}` interpolation shape the pair-attribution depends on), **and asserts the negative on both
bypass-continue branches**: the `var_ignore_os_mismatch` warn (`:4051`, "but the script recommends …") and the
`var_ignore_disable` warn ("Bypassing disable status …") must contain **none** of the three signature
substrings — otherwise an upstream copy-edit toward the refusal wording would flip every successful acked
dispatch to Failed with no test failing. It also fetches `misc/core.func` to pin the `msg_error` colorization
(ANSI + `\r`) the normalization step depends on — stream is _not_ pinned or discriminated on (`msg_error` and
`msg_warn` both write to stderr). This is the
workspace's first
_network_-dependent ignored test — it does not slot into the Docker-based `--ignored` suite convention; the
existing precedent to follow is the standalone `#[ignore = "reason"]` form
(`crates/plugins/discovery/uptrakit-self-update/src/plugin.rs:340` uses it for a live-filesystem test). It gets its
own invocation (`cargo test -p uptrakit-plugin-discovery-proxmox-helper-scripts -- --ignored`), documented in
`docs/development/quality-gates.md` (and the exception noted in `docs/development/testing.md`) in the same commit
that adds the test, with the trigger "run when PHS is touched". Upstream drift then surfaces as a named test
failure, not as silent false-Completed regressions in the field.

**Accepted risk — output truncation**: the agent buffers up to 10 MB of output (`MAX_OUTPUT_BYTES`,
`crates/shared/agent-core/src/update.rs:36`) but the wire caps output strings at 1 MB (`MAX_OUTPUT_STRING_LEN`,
`crates/shared/wire/src/limits.rs:212`) and the WS frame at 1 MB (`MAX_WS_MESSAGE_SIZE`,
`service_ws/connection.rs:46`); an oversized result envelope is rejected and the record stays non-terminal. This
is a pre-existing hole independent of this design — tracked as `uptrakit-def-ws-oversized-update-output`
(which also fixes `limits.rs:212`'s doc comment wrongly claiming the agent already bounds output at 1 MB). For
the signatures here it is not a practical miss vector: a skipped run terminates before the update produces
bulk output, so the signature sits in a near-empty stream.

This is scoped to PHS ("update ran but self-skipped"); it does not generalize output-scraping to other plugins.

### Phase C — Notification

New `NotificationEventType::BaseOsIncompatible`, defined via the existing `wire_safe_enum!`
(`crates/shared/web-api-types/src/notifications/event_types.rs:20-31` — the macro provides the `Other(String)`
forward-compat arm).

- **Edge-triggered**: emitted when the new evaluation is `Mismatch` (any kind) **and** differs from the prior
  evaluation — where "differs" compares the mismatch kind and the required pair. This covers both
  non-blocking → blocking transitions and blocking → _differently_ blocking ones (e.g. upstream re-targets the
  script to another distro), matching the ack re-arm's notion of "new fact". Steady-state incompatibility
  (identical kind + pair) never re-fires. Prior evaluation is recomputed from the pre-update row state at the
  **two** sites where stored inputs change: discovery reconcile (the sole requirement-refresh path — see Phase A),
  and the **host identity upsert on agent connect** — a reprovisioned or upgraded container reports a new
  `ID`/`VERSION_ID` there, and without this site the most common real transition (the host itself changed) would
  never notify; the upsert recomputes over that host's links in one batched pass (`Column::Id.is_in(...)`, never
  per-row — batch-query rule). This mirrors the edge-trigger
  pattern being established by tag-series Plan 3's `discovery_override_conflict` notification (see Dependencies).
- **First-materialization suppression**: when the prior state lacked inputs (no stored requirement or no host
  identity — true for the entire fleet on the first evaluation after this feature deploys), the transition into
  `Mismatch` does **not** fire; the badge and the gate still apply. Notification is reserved for transitions
  between _evaluated_ states, which also kills the day-one notification storm. Accepted consequence: a
  newly-discovered incompatible item never fires this event on its first evaluation — the 409 gate and the badge
  carry the information.
- Payload: host name, software item name, host identity pair, required pair. All user-controlled values pass
  through `escape_html()` in HTML-capable channel bodies (notifications invariant).
- Touch list (verified): `event_types.rs` enum + `KNOWN_EVENT_TYPES`; the `NotificationEventDetails` match in
  `crates/plugins/notifications/delivery/src/event.rs:100-126`; rule matching in
  `crates/shared/web-api-types/src/notifications/rules.rs`; OpenAPI schema registration
  (`crates/ui/web-api/src/router.rs:336`); CLI hardcoded event-type enumerations — the `event_type` arg doc comment
  (`crates/ui/cli/src/commands/notifications.rs:107`) and the two "unknown event type" error strings (`:297`,
  `:323`);
  frontend `EVENT_TYPE_LABELS` `Record`s (`NotificationRulesSettings.svelte:26-27`, `NotificationLogView.svelte:14`
  — TypeScript fails closed on a missing key); regenerated API types.
- Fallback-detected skips (Phase B) need no new event: the record becomes a Failed update and takes whatever
  notification treatment failed updates already get.

### UI changes

- Link rows with a blocking mismatch show a warning `StatusBadge`
  (`frontend/src/lib/components/ui/StatusBadge.svelte`; usage exemplar `SoftwareGroupList.svelte:352,362`) —
  e.g. `tone="warning" label="Base OS"`, with the host/required pair in the detail view.
- Trigger-update flow: on HTTP 409 with `code: "base_os_mismatch"`, surface a `ConfirmDialog`
  (`frontend/src/lib/components/ConfirmDialog.svelte`), mapped onto its actual prop contract: `title` = "Base OS
  mismatch", `messagePrefix`/`entityName` name the software item, and the host-vs-required identity pairs go into
  `warnings` (its `Callout`-row list — the component has no free-form body slot). `confirmVariant` is explicitly
  **not** `'danger'` (the default) — acking is an override-and-proceed acknowledgement, not a destructive action;
  use `'primary'`, the sole other value in the component's `confirmVariant?: 'primary' | 'danger'` prop union
  (this is `ConfirmDialog`'s own type, narrower than `Button`'s full variant set). Dialog copy branches on the
  mismatch direction (host-newer: "acking is usually safe"; host-older/different-distro: stronger warning) and states
  the blast radius: confirming bypasses the OS guard for every script in that container's `/usr/bin/update` run
  (see Acked dispatch), enumerating by name every PHS item on the host whose declared pair also mismatches (the
  same sibling set the gate's sibling-pair check enforces — confirming acks all listed pairs together). The
  Rust `kind` is **not** wired to the frontend (the 409 body stays `{ error, code }`):
  the dialog derives direction client-side from the two pairs it already has (link's `base_os_requirement` +
  host identity from regular API responses) — same-distro numeric compare, ~5 lines of TypeScript. This
  duplicates wording-only logic by design: it can never change blocking behavior, so harmless divergence is
  accepted and no cross-language parity test is required (mirror of the Rust rule that `kind` is best-effort
  wording).
  Confirming calls the ack endpoint and re-triggers. Acked state shows a distinct badge ("Base OS acked") on every
  link whose pair is in the host's acked set, so the override stays visible.
- Host detail: display Host OS Identity alongside the existing PRETTY_NAME string.

### Relation to existing mechanisms (why not reuse)

- **`DetectHostCompatibility` plugin role** (`HostCompatibility` enum,
  `crates/plugins/infrastructure/core/src/traits.rs:8`; trait default `roles.rs:66`; PHS impl `plugin.rs:778`): an
  agent-side, per-plugin runtime probe — "can this plugin operate on this host at all" (binary present, update
  script installed) — consumed by the agent-core version-check retry loop. It is per-(host, plugin), evaluated
  where the plugin runs, and knows nothing about a specific declared requirement. The base-OS gate is per-link,
  evaluated controller-side at dispatch, and changes answer when the declared requirement or the host identity
  changes. Folding requirement data into the runtime probe would put dispatch policy on the agent (wrong side
  of the trust boundary — the controller owns dispatch preconditions) and would race the retry loop's caching.
- **`HostRequirements` on `RoleSlot`** (`crates/plugins/infrastructure/core/src/host_requirements.rs`): static,
  per-role, OS-_family_ granularity (Linux vs macOS), fixed at plugin authoring time. Base-OS Requirement is
  dynamic (declared per release by upstream script content) and distro-_version_ granular. Different lifetime,
  different granularity; reuse would force per-release data into a static descriptor.

Both distinctions go into `docs/development/plugin-guidelines.md` (one short paragraph) so the next author doesn't
re-conflate them.

### Error handling

Per project standards: `BaseOsMismatch` joins the existing typed enums at each boundary (`TriggerUpdateError`,
`UpdateDispatchError`, `ApiError`) — no new enums, and the chain stays typed end to end; only the final HTTP body
flattens to the project-wide `{ error, code }` `ApiError` contract (machine discriminant in `code`). The evaluation function is total
(returns `Unknown`, never errors). Ack endpoint uses the module's existing `Result<T>`/`Report` alias. No
`unwrap`/`panic!`; parse failures in `analyze_phs_script` degrade to `None` fields exactly like the existing header
extraction.

### Security considerations

- The requirement originates from fetched script content (PHS `SOURCES` allowlist governs what is fetched; no new
  fetch surface). It is data _about_ an update, never executed; stored bounded by wire limits.
- Gate is enforced controller-side in `load_target_for_dispatch` — agents and MQTT cannot bypass it; the ack is
  the only bypass and requires `updates:trigger` via the typed extractor (no inline authz).
- Ack is audited (Stateful, before/after snapshots) — an override of a safety gate must be attributable. The
  gate's sibling-pair check keeps that attribution complete: the injected bypass is container-wide, so dispatch
  refuses until every pair the bypass would reach has its own acked (and audited) entry — nothing the override
  covers escapes the audit trail.
- No secrets involved anywhere in the new columns/payloads; no logging of anything sensitive (os-release values are
  not sensitive).
- Fail-open on `Unknown` is deliberate: this is an availability/correctness guard, not a security boundary — the
  upstream guard still runs on the host, and Phase B reports its verdict truthfully. Blocking on missing data would
  brick updates for every non-Linux host and pre-upgrade agent.

### Testing

Per the vacuous-guard-test ledger rule, every rejection test asserts the _reason_, and each fixture isolates exactly
one rule:

- `evaluate_base_os` unit table (pure, no DB): one fixture per numbered rule above — no-requirement ⇒ `Unknown`;
  no-identity ⇒ `Unknown`; empty `req.os` ⇒ `Compatible` (guard-inert); empty `req.version` ⇒ `Compatible`;
  distro mismatch ⇒ `Mismatch`/`DifferentDistro` with both pairs; exact match ⇒ `Compatible`; host-longer prefix
  `12.5` vs `12` ⇒ `Compatible`; non-dot-boundary `12.5` vs `12.` / `125` vs `12` ⇒ `Mismatch`; required-longer
  `13` vs `13.1` ⇒ `Mismatch`/`HostOlder` (upstream prefix test fails); `12` vs `13` ⇒ `Mismatch`/`HostOlder`;
  `13` vs `12` ⇒ `Mismatch`/`HostNewer` (the Debian 12→13 case — blocks, ack-able); non-numeric inequality
  (`bookworm` vs `12`) ⇒ `Mismatch`/`Undetermined`; case-insensitive ID compare.
- `analyze_phs_script`: header parse fixtures — quoted, unquoted, absent, malformed ⇒ `None` (success + failure
  paths).
- Dispatch gate (TestApp harness, per REST-test rule): mismatch ⇒ 409 asserting `code == "base_os_mismatch"` and
  both pairs present in the message (not merely status); host's `base_os_acks` contains the current requirement
  pair ⇒ dispatch proceeds **and** the dispatched command is `PHS_INSTALL_CMD_ACKED` (bypass injected); a
  same-pair sibling item on the same host ⇒ also passes (set scope); acked set holds only a _different_ pair ⇒
  409 again (re-arm); requirement cleared (`HeaderAbsent`) after ack ⇒ gate no longer fires and the acked pair is
  inert;
  acked host with a user-customized `update_command` on the link ⇒ dispatch proceeds with the command untouched
  (no swap);
  `Unknown` ⇒ proceeds with stock command; sibling-pair check — dispatched item's pair acked but a sibling
  PHS item's declared pair mismatches un-acked ⇒ 409 naming the sibling's pair and item; both pairs acked ⇒
  proceeds with the swap. Per-call-site semantics: batch
  creation ⇒ item lands in `BatchSkippedItem` with the mismatch reason (200); queued-dispatch gate hit ⇒ record
  finalized Failed with `recovery_hint` populated; queued-progression gate hit (`dispatch_next_queued_for_host`)
  ⇒ same Failed + `recovery_hint` (asserts the parallel `web-api-queries` implementation got the same extension);
  reconnect-replay gate hit ⇒ same, via the extended
  `fail_unreplayable_pending_update` — assert the _real_ mismatch message lands in `output` (not the generic
  reconstruct-failure text) and `recovery_hint` is set; service WS trigger (`handle_service_trigger_update`, its
  existing test style at `update_tracking.rs:784+`) ⇒ `ControllerMessage::Error` whose message names both identity
  pairs.
- Ack endpoint (TestApp): success appends the lowercased pair to the host's `base_os_acks` + emits audit (host,
  pair, actor in the snapshot); repeat ack of the same pair ⇒ idempotent success, set unchanged; ack of a second
  distinct pair ⇒ appends without dropping the first; no-mismatch ⇒ 409; permission denied without
  `updates:trigger`.
- Finalize fallback: success-status result whose output contains the OS-guard signature embedding the item's own
  requirement pair ⇒ persisted Failed + its `recovery_hint`; OS-guard signature embedding a _different_ pair
  (multi-script container — another script skipped) ⇒ untouched + warn asserted; OS-guard signature with no
  stored requirement ⇒ single-item rule (Failed when the host's sole PHS item, untouched + warn otherwise); the
  script-retired signature on a single-PHS-item host ⇒ Failed + its distinct hint; the script-disabled signature
  on a single-PHS-item host ⇒ Failed + its distinct hint; the script-disabled string co-occurring with the
  `var_ignore_disable` bypass warn ⇒ untouched (bypass-success path, co-occurrence guard); the acked-dispatch
  bypass warn ("but the script recommends …") alone ⇒ untouched (no signature hit); retired/disabled signature
  on a multi-PHS-item host ⇒ untouched + warn; signature wrapped in ANSI color
  codes / `\r` ⇒ still detected (normalization); signature split across two output rows ⇒ detected on the joined
  string; same output on an item whose resolved `update_command` is not `PHS_INSTALL_CMD` ⇒ untouched (discriminator scope
  guard); signature absent ⇒ untouched.
- Notification edge: evaluated non-blocking → blocking fires exactly one event; unchanged incompatible state on
  the next check fires none; blocking → differently-blocking (kind or required pair changed) fires again;
  first materialization (prior state missing requirement or identity) fires none (storm suppression); host
  identity change at the agent-connect upsert fires for affected links (second site).
- Analysis-outcome write paths (discovery reconcile, the sole persistence site): `Declared` ⇒ upsert;
  `HeaderAbsent` ⇒ stored `base_os_requirement` written NULL (and a previously acked pair becomes inert per the
  gate fixture above); `ParseFailed` ⇒ stored value untouched + warn asserted; absent field (old-agent payload,
  serde default `None`) ⇒ stored requirement untouched; unknown variant tag (newer-agent payload — raw JSON
  fixture with a fabricated tag) ⇒ deserializes to `Unknown`, stored value untouched + warn asserted, and the
  enclosing `DiscoveryResults` message still processes.
- Wire: `WireValidate` bounds for the new fields; asyncapi golden test (`asyncapi_yaml_is_up_to_date`) covers the
  contract regen.
- No `start_paused` anywhere here unless a test uses `tokio::time` (none is expected to); DB tests never
  `start_paused` (SeaORM exception).

## Dependencies (cross-cycle)

- **`uptrakit-plan-2026-08-19-tag-series-2-phs-inference`** (open, gated): edits the same PHS
  `discovery.rs`/`plugin.rs` regions (script analysis, per-item target builders) that Phase A extends.
  Classification: **same-files only** — no decision or behavioral dependency; ordering exists purely to avoid
  conflicting edits. Stage: implementation only — this spec's _plan execution_ waits for tag-series Plan 2, spec
  review and plan writing do not. Wired in beads accordingly.
- **`uptrakit-plan-2026-08-19-tag-series-3-discovery-fill-notify`** (open, gated): establishes the edge-triggered
  discovery-notification + audit pattern Phase C mirrors. Classification: **pattern precedent, not a hard
  dependency** — Phase C can land first if sequencing flips; soft relation only (`bd dep relate`).
- KB precedent honored: `uptrakit-4v4a9` (asset_patterns not auto-populated) — no interaction with this design;
  noted only because both threads touch PHS discovery metadata.

## Deferred / out of scope (tracked beads)

- **Guided base-OS upgrade path** — `uptrakit-def-base-os-upgrade-path` (epic, deferred). Synthesized "Base OS"
  software item per eligible host; embedded versioned script (`/usr/local/bin/uptrakit-debian-upgrade`) executed via
  the generic shell plugin with `prefer_interactive`; protection default-require with ack-through via the Update
  Protection Controller (existing term, `CONTEXT.md`); third-party apt repos handled as **disable → upgrade →
  re-enable** (user decision), drawing structure from the reference Ansible role (`upgrade_trixie`): Debian-N guard,
  ≥650 MiB root free-space check, fully-update-current-release first, conditional pre-reboot, sources codename flip,
  dist-upgrade, post-upgrade major-version assert, autoremove/clean, reboot, `apt modernize-sources`. Warrants its
  own spec cycle when activated.
- **Upstream PR: distinguishable silent-mode skip** — `uptrakit-def-phs-upstream-guard-exit` (chore, deferred).
  Propose upstream that `start()` propagate the guard's non-zero return in silent mode (today
  `check_container_os_guard || return 0` swallows it), so machine callers get a real exit code. Until merged, the
  output-signature fallback is the only detection; after merge, exit-code detection can supplement (not replace —
  fleet scripts update lazily).
- **Stale PHS doc citations** — `uptrakit-def-phs-stale-doc-cites` (chore, deferred). Per `uptrakit-w9121` notes:
  `proxmox-plugin.md` endpoint table drift, `host-entity.md:206` stale path, PHS `config.rs` doc-comment drift,
  and `plugin.rs:777-786` (`detect_host_compatibility` doc comment claims the update script is "installed on all
  Proxmox VE nodes" — it tests for it inside containers). `host-entity.md` is fixed opportunistically in this
  cycle (touched anyway for the new columns); the rest wait.
- **Oversized update-output envelopes never finalize** — `uptrakit-def-ws-oversized-update-output` (chore,
  deferred). Agent buffers 10 MB (`MAX_OUTPUT_BYTES`) but `MAX_OUTPUT_STRING_LEN`/`MAX_WS_MESSAGE_SIZE` cap the
  wire at 1 MB; an oversized `UpdateResult` envelope is rejected at the protocol layer and the update record stays
  non-terminal. Pre-existing, independent of this design; also fix `limits.rs:212`'s wrong doc comment.

## Documentation deliverables

Implementation must update (non-optional — externally observable behavior, wire, API, and schema all change):

- `CONTEXT.md` — three glossary terms (**done in this cycle**, alongside this spec).
- `docs/api/wire-protocol.md` + `crates/shared/wire/asyncapi.yaml` (regen) — `HostInfo` and discovery payload
  additions.
- `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` (regen) — ack endpoint, 409 body,
  notification event type.
- `docs/development/autodiscovery-internals.md` — PHS `var_os`/`var_version` capture + requirement persistence
  rules.
- `docs/architecture/host-entity.md` — new host columns (+ fix its stale path cite while touched).
- Software-item / update docs (`docs/architecture/update-history-entity.md` or the docs/api update pages, whichever
  currently documents dispatch preconditions) — the 409 gate, ack semantics, and Failed-with-`recovery_hint`
  fallback.
- `docs/development/notifications.md` — new event type.
- `docs/development/plugin-guidelines.md` — the two-paragraph "which compatibility axis is which" note.
- `docs/development/quality-gates.md` + `docs/development/testing.md` — the new PHS signature-drift canary: its
  invocation, its "run when PHS is touched" trigger, and the note that it is the first network-dependent
  `#[ignore]`d test (following the standalone `#[ignore = "reason"]` precedent, not the Docker suite convention).
- **No ADR**: the gate semantics (upstream-mirror predicate, fail-open, (host, pair)-keyed ack-set re-arm) are
  reversible column-level policy fully documented here and in the docs above — nothing is hard to reverse and no
  surprising trade-off needs an architecture record.

## Snapshot conformance notes

- **No new external dependencies** — everything rides existing primitives (SeaORM, sea_query, wire macros,
  `wire_safe_enum!`, existing frontend components). Hence no version pins to declare.
- No `semver` crate: the verdict needs no version parsing at all (string equality + dot-boundary prefix, mirroring
  upstream); the numeric-segment compare exists only for message wording.
- Feature-flag rule: no new flags; all additions unconditional in the PHS/agent/web-api crates they live in.
- `update_available` string-equality precedent respected: the ack path uses plain string equality on the
  requirement pair — no version-ordering logic anywhere in the gate/ack chain.
- Migration naming, `begin_immediate()`, `TenantDb`, typed extractors, `Unvalidated<T>` for the ack body (none —
  path-params only, so no body extractor needed), audit catalog coverage, `db_access_policy.toml` classification of
  the ack handler (`tenant-scoped`, same commit): all called out inline above.
