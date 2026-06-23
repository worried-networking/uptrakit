# file_digest Consistency — Design Spec

**Date:** 2026-06-23 **Status:** Draft **Target:** `crates/shared/config-reload/` (loader, status, audit
event) + `crates/core/controller-runtime/src/boot/{init,reload}.rs` + `crates/shared/web-api-types/` (digest
rustdoc/OpenAPI). `pki::sha256_hex` left untouched. **Related:** ADR 0008 (graceful reload), follow-up from
the controller boot decomposition

## Problem

Two `file_digest` implementations compute the config-file digest for the `GET /api/v1/instance/config-state`
endpoint, in **divergent formats**:

| Impl                                                                | Success                           | Error           |
| ------------------------------------------------------------------- | --------------------------------- | --------------- |
| `boot::init::file_digest` (seeds initial `ConfigFileState`)         | `sha256:<hex>`                    | `size:<N>` stub |
| `boot/reload.rs::file_digest` (updates `ConfigFileState` on reload) | plain `<hex>` (`pki::sha256_hex`) | `""`            |

`ConfigFileState.digest` therefore changes format across its own lifecycle — `sha256:abc` at boot, then plain
`abc` after the first reload — and `pending_digest` is plain hex while the seeded `digest` is prefixed.

**Why it matters (more than cosmetic):** the two fields exist so an operator/UI can answer "does the config on
disk differ from what's running?" — a `digest` vs `pending_digest` comparison. With mixed formats that
comparison is always unequal even for an identical file (`sha256:abc` ≠ `abc`), so the drift signal is broken
for any consumer that compares them. Today only the display endpoint reads these (no Rust compares them), but
the comparison is the obvious intended use and a UI can trivially get it wrong.

**Root cause:** `boot_config` calls `TomlConfigLoader::load` (which reads the file) and then calls
`file_digest(&path)` — a **second, redundant read** of the same file. The `size:`/`""` fallbacks exist only to
paper over that second read racing. The reload bridge similarly re-reads the path.

## Goal

One canonical config-file digest — single format, single SHA-256-hex code path, owned by the crate that owns
`ConfigFileState`. The boot seed hashes the bytes the loader already read (no extra read); the reload paths
use one unified `file_digest(path)` reader. Model the one genuinely-unknowable case honestly (pending read
error → `None`) rather than with a fabricated stub.

## Chosen approach (primary recommendation)

**One `sha256:<hex>` digest format defined in config-reload; boot seeds from the loader's already-read bytes;
the reload bridge re-reads via a single unified `file_digest` for both the pending and applied paths; collapse
the two controller-runtime `file_digest` copies.** No reload-event surgery — the bridge owns the reload-time
reads (a display-only digest does not justify threading a field through `ReloadAuditEvent::Applied` +
`process_request`; the sub-ms re-read race is operationally irrelevant since the coordinator just loaded the
file).

### Canonical format

`sha256:<lowercase-hex>` — always. The `sha256:` prefix is self-describing (algorithm-tagged,
forward-compatible) and disambiguates a real digest from any future degraded value. **No `size:` stub and no
empty-string fallback** — the error case is handled structurally (below), not by a sentinel string.

### SHA-256-hex helper home (scoped dedupe)

The duplicate to remove is the **config-digest** logic (two copies in controller-runtime). Home the helper in
`config-reload`, which owns `ConfigFileState`, `LoadedConfig`, and the pending-path `file_digest`:

- `pub(crate) fn sha256_hex(bytes: &[u8]) -> String` in `config-reload` — implement as
  `uptrakit_shared_types::hex::encode(Sha256::digest(bytes))` (i.e. `sha2::Sha256` + the project's
  `hex::encode`, **matching `pki::sha256_hex` byte-for-byte**; do NOT use the `format!("…{:x}", finalize())`
  form from the soon-deleted `init::file_digest`). Wrap it in a private
  `fn config_digest(bytes: &[u8]) -> String { format!("sha256:{}", sha256_hex(bytes)) }` — the **one** place
  the config-digest format is defined.
- `pub fn file_digest(path: &std::path::Path) -> Result<String, rootcause::Report>` (pending + applied
  re-reads) lives in the loader module; **re-export it from `config-reload/src/lib.rs`**
  (`pub use loader::file_digest;`, mirroring the existing `pub use loader::{LoadedConfig, TomlConfigLoader}`)
  so it is reachable — otherwise `unreachable_pub = "deny"` fires.
- Add `sha2 = { workspace = true }` to `crates/shared/config-reload/Cargo.toml`. (`sha2 = "0.10"` is already
  in the workspace `[workspace.dependencies]` — no new pin, no registry addition.)

**Do NOT add `sha256_hex` to `uptrakit-shared-types`.** That crate is `publish = true` and deliberately
zero-dependency (its `hex.rs` replaced the external `hex` crate to keep it light); adding `sha2` there would
widen a published value-crate's public dependency surface for a 4-line helper with no cross-crate consumer.

**Leave `crate::pki::sha256_hex` (controller-runtime) untouched** — it serves two PKI-domain call sites
(`pki.rs:180`, `pki.rs:735`). Honest dedupe scope: after this change the codebase has **one config-digest
path** (config-reload) but still **two identical SHA-256-hex primitives** (`config-reload::sha256_hex` and
`pki::sha256_hex` — same `sha2 + hex::encode` body). That residual duplication is accepted deliberately: the
only crate that could host a shared primitive below both is `uptrakit-shared-types`, and burdening that
published zero-dep value crate with `sha2` is not worth collapsing a 4-line fn across a domain boundary. (An
earlier framing proposed exactly that single workspace-wide `sha256_hex`; rejected on the publish-weight
grounds above.)

### Digest computed at the single read point (loader)

`crates/shared/config-reload/src/loader.rs`:

- `TomlConfigLoader::load` already does `let raw = std::fs::read_to_string(path)`. Compute
  `digest = config_digest(raw.as_bytes())` and return it:

```rust
pub struct LoadedConfig {
    pub config: RuntimeConfig,
    pub warnings: Vec<String>,
    pub digest: String, // "sha256:<hex>" of the file bytes just read
}
```

- The private `config_digest` helper (above) is the one place the format is defined.
- **Update the sole internal construction site** — `loader.rs:48` `Ok(LoadedConfig { config, warnings })` →
  add `digest`. `LoadedConfig` is `#[non_exhaustive]`, so no external struct-literal exists; `validate_only`
  (loader.rs:61) goes through `Self::load` and is unaffected (it never destructures the struct). The extra
  SHA-256 it now computes-and-discards is microseconds on a one-shot CLI path — negligible.
- **Byte source (Unix caveat):** hash `raw.as_bytes()`. On the supported deployment platforms (Linux/macOS)
  `read_to_string` performs no newline translation, so this is byte-identical to `std::fs::read` for a valid
  UTF-8 file (config is always UTF-8 — `read_to_string` errors otherwise, failing the load). Thus the loader
  path (`raw.as_bytes()`) and the pending/applied-path `file_digest` (`std::fs::read`) produce the same digest
  for the same file. NB this byte-identity does **not** hold on Windows (text-mode `\r\n`→`\n` translation),
  which is not a deployment target; the cross-path consistency test (Verification) must write its fixture in
  binary mode (`std::fs::write`) so it is deterministic even there.

**Why `LoadedConfig.digest` rather than re-reading at boot:** boot already holds the validated bytes, so
hashing them is **infallible** — the boot seed has no read-error branch and needs no fallback (the very
`size:`/`""` ambiguity this spec removes). Calling `file_digest(&config_path)` at boot instead would
reintroduce a fallible re-read whose error would force an awkward boot-time fallback. The single `digest`
field on the (already `#[non_exhaustive]`, internally-constructed) `LoadedConfig` is the cheap price for that.

### Pending-path read (the one genuinely-fallible case)

On `FileChanged`, the watcher saw a change but the coordinator has **not** loaded it yet, so the bridge must
read the file _now_ and that read can race (file changed again / deleted). Provide, in config-reload, a
fallible reader that returns the project error type (idiomatic; consistent with `TomlConfigLoader::load`, and
preserves _why_ it failed for logging):

```rust
/// Reads `path` and returns its `sha256:<hex>` digest.
///
/// # Errors
/// Returns the read error if the file cannot be read (transient race: the change
/// that triggered this may have been superseded or the file removed).
pub fn file_digest(path: &std::path::Path) -> Result<String, rootcause::Report>;
```

The bridge (fire-and-forget, no error channel) computes the digest **before** the `send_modify` (keep file I/O
out of the watch closure) and applies all fields in **one** `send_modify` for atomicity (watchers must never
observe a half-updated `ConfigFileState`):

```rust
let pending = match uptrakit_config_reload::file_digest(path) {
    Ok(d) => Some(d),
    Err(e) => { tracing::warn!(%path, error = %e, "pending digest unavailable"); None }
};
channels.file_state_tx.send_modify(|s| {
    s.pending_digest = pending;                       // None on read error
    s.pending_detected_at = Some(now);                // ALWAYS — a change WAS detected
});
```

So on error the endpoint truthfully shows "change detected at T, digest unknown" (`pending_digest = null`)
rather than a fabricated `size:512`. `pending_digest` is already `Option<String>` — no type change; the `warn`
log lives only at the call site (the fn does not log, avoiding double-logging).

### Applied path: bridge re-reads via the unified helper

On a successful file-sourced reload (`Applied` + `Sighup`/`FileWatch`), the bridge's status-watch arm
(~reload.rs:243) already re-reads `config_path` — keep that, but route it through the **unified**
`uptrakit_config_reload::file_digest`, so the applied digest uses the same `sha256:<hex>` format as the boot
seed and the pending digest:

Compute first, then one `send_modify` (same atomicity rule as the pending arm):

```rust
let applied = uptrakit_config_reload::file_digest(&channels.config_path)
    .inspect_err(|e| tracing::warn!(error = %e, "applied digest re-read failed; keeping last digest"))
    .ok(); // None on the transient-race error
channels.file_state_tx.send_modify(|s| {
    if let Some(d) = applied {
        s.digest = d;
    } // on None: leave s.digest unchanged (last-good) — the coordinator just loaded the file;
      // a re-read error is a transient race and must not blank the displayed digest.
    s.loaded_at = now;
    s.pending_digest = None;
    s.pending_detected_at = None;
});
```

No change to `ReloadAuditEvent`, `process_request`, or the coordinator. The non-file `Applied` source
(SIGHUP-with-no-file-change is still file-sourced; DB `settings_version` bump / Boot are not) keeps its
existing behavior — the current code only updates `s.digest` under the `Sighup | FileWatch` source arm,
leaving the non-file path untouched, which is correct. The second `Applied` arm in the bridge (the audit-emit
arm, ~reload.rs:332) is unchanged — it never touched the digest.

**Cost accepted:** the coordinator just read the file to load it; the bridge re-reads it microseconds later
for the digest. For a display-only field this sub-ms race (file mutated again in that window) is operationally
irrelevant and was already the pre-existing behavior — this change only unifies the _format_ of that re-read,
it does not add a read.

**Deliberate asymmetry:** `pending_digest: Option<String>` models "unknown" as `None`, but `digest: String`
(non-optional) keeps its last-good value on an applied-read error rather than blanking — the stale window is
observable only via the `warn` log. This asymmetry is intentional: `pending_digest` is genuinely absent until
a change is detected, whereas `digest` always reflects the last successfully-hashed config and a transient
re-read error should not erase it. Not changing `digest`'s type (would ripple into the wire type +
config-state endpoint for no operational gain).

### Result: delete both controller-runtime copies

- Delete `boot::init::file_digest` (mod.rs ~189) — `boot_config` uses `loaded.digest` for the initial
  `ConfigFileState`.
- Delete `boot/reload.rs::file_digest` (~384) and its `# Divergence` doc — the bridge calls
  `uptrakit_config_reload::file_digest` for both the pending path and the applied re-read.

## Alternatives considered

- **Minimal in-place fix (~3–5 lines):** edit only the existing `boot/reload.rs::file_digest` to emit
  `sha256:<hex>` and replace its `""` fallback with `size:<N>` (matching `boot::init::file_digest`), leaving
  both copies in controller-runtime. This achieves the _observable_ goal (consistent endpoint format) with no
  new dep, no public API, no crate move. **Rejected on the user's explicit location decision** — the digest
  format should be owned by the crate that owns `ConfigFileState` (`config-reload`), not duplicated in
  controller-runtime. (Both contrarian passes flagged this as the smaller scope; it is recorded here so the
  tradeoff is explicit. If the team later prefers minimalism over the ownership principle, this is the
  fallback — see the review summary's residual-decision note.)
- **Keep a `file_digest(path)` fn in controller-runtime** — same as above; leaves authority split from the
  type. Rejected for the same reason.
- **`size:<N>` / empty-string error stub** — `size:N` is not a content hash (same-size content change is
  invisible); empty is ambiguous. Rejected in favor of `Result`/`None` (pending) and last-good (applied).
- **Bare `<hex>` (no prefix)** — user chose the prefixed form; `sha256:` is self-describing. Rejected.
- **Thread the digest through `ReloadAuditEvent::Applied`** (loader computes it, coordinator emits it, bridge
  consumes it — no reload-time re-read) — strictly single-read, but requires changing `process_request`'s
  return type, adding an `Option<String>` field to a `#[non_exhaustive]` cross-subsystem event, the no-op
  return tuple, both bridge match arms, and an event-digest pinning test. **Rejected (user decision):** too
  much machinery for a display-only field whose re-read race is operationally irrelevant; the bridge re-read
  is the lighter, equally format-consistent path.

## Constraints (snapshot-bound)

- **Error handling** (`coding-standards.md`): `TomlConfigLoader::load` keeps returning `rootcause::Report` on
  real load failure; the boot digest is computed on the success path from bytes already in hand (no new
  fallible step). `file_digest(path)` returns `Result<_, rootcause::Report>` — callers adjudicate (bridge:
  pending → `warn` + `None`; applied → `warn` + keep last-good). No sentinel strings, no masked error, no
  `unwrap`/`expect` in production. **No change to `ReloadAuditEvent`, `process_request`, or the coordinator**
  — the reload-event type is untouched.
- **New dependency:** add `sha2 = { workspace = true }` to `crates/shared/config-reload/Cargo.toml` (NOT
  shared-types — see the helper-home section). `sha2 = "0.10"` is already in `[workspace.dependencies]`; no
  new pin, no registry addition.
- **Visibility:** `sha256_hex` + `config_digest` are `pub(crate)`/private in config-reload (no cross-crate
  audience); `file_digest(path)` and `LoadedConfig.digest` are `pub` in config-reload (consumed by
  controller-runtime). `unreachable_pub = "deny"` respected.
- **Behavior change (externally observable):** the `config-state` endpoint's `digest`/`pending_digest` values
  become consistently `sha256:`-prefixed, and `pending_digest` becomes `null` (was `""`/`size:N`) when a
  pending file can't be read. This is the intended fix; documented below.

## Verification

- **Unit tests** (config-reload): `config_digest`/`sha256_hex` returns `sha256:<64-hex>` for a known vector
  (format-pinning so the prefix can't silently regress); `file_digest` returns `Ok(sha256:…)` for a readable
  temp file and `Err` for a missing path; `TomlConfigLoader::load` populates `LoadedConfig.digest`.
- **Format consistency**: assert the boot seed (`LoadedConfig.digest`) and `file_digest(path)` on the same
  file produce the identical `sha256:<hex>` string — the two paths must never diverge again (the whole point).
  Write the fixture with `std::fs::write` (binary) so the assertion is deterministic regardless of platform
  newline handling.
- Standard gates: `cargo fmt`, `cargo check`/`clippy` (`--no-default-features --features db-sqlite` and
  `--all-features`), `cargo test --all-features`, doctests (`--exclude uptrakit-mqtt-runtime`),
  `cargo deny check`, markdownlint.
- Boot path touched (config load) → Docker integration suite (`-p uptrakit-integration-tests -- --ignored`) as
  the behavioral guard.
- No reverse-proxy tests (no proxy code).

## Documentation deliverables

- **`ConfigFileState` rustdoc** (`config-reload/src/status.rs`) — update the `digest` / `pending_digest` field
  docs to state the canonical `sha256:<hex>` format and that `pending_digest` is `None` when a detected change
  can't be read. **Document the `digest` last-good behavior:** on an applied-path re-read error `digest`
  retains its previous value (it is never blanked); a _persistent_ re-read failure (e.g. the config file was
  moved/removed after load) therefore leaves a stale digest displayed, signalled only by repeated
  `applied digest re-read failed` warn logs. Operators should treat sustained warns as "displayed digest may
  be stale." (Accepted over a new `Option`/staleness field — that would ripple into the wire type + endpoint
  for a display-only signal.)
- **`LoadedConfig.digest` rustdoc** + the pending-path `file_digest` rustdoc — new public API; document the
  `sha256:<hex>` format and the `Err`-on-unreadable semantics.
- **`FileStateView.digest` rustdoc** (`crates/shared/web-api-types/src/instance_config_state.rs:57`, currently
  "hex SHA-256 or size stub") + its utoipa/OpenAPI description on the `instance_config_state` route — update
  to `sha256:<hex>` and note `pending_digest` nullability. (Externally observable API value change.)
- **`sha2` dep line** in `config-reload/Cargo.toml` — note in the commit; no separate doc.
- **No new ADR.** This is a consistency fix within the ADR 0008 reload design, not a new architectural
  decision. ADR 0008 does not specify the digest format, so no amendment is required. Stated explicitly to
  satisfy the doc-impact check.
- **No** README / runbook / user-guide changes — the field is operator-facing telemetry whose format becomes
  consistent; no workflow changes.

## Out of scope / deferred

- Adding a computed `drift_detected: bool` to the endpoint (server-side loaded-vs-pending comparison). This
  spec makes the comparison _correct_ (one format) but leaves the comparison itself to the UI. Revisit if a
  server-side signal is wanted.
- Broader consolidation of other ad-hoc hashing in the codebase beyond `pki::sha256_hex` (e.g. plugin/docker
  digest handling — unrelated domain).

## Open questions

None outstanding — format (`sha256:`-prefixed), location (config-reload / shared-types), `Applied`-path
threading, and `None`-on-unreadable-pending are all settled.
