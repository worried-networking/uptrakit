# CLI Controller Discovery via Zeroconf — Design

Date: 2026-07-28
Status: approved for planning

## Summary

`uptrakit auth login` learns to locate the controller on the local network via the existing
mDNS/DNS-SD advertisement (`_uptrakit._tcp.local.`) when the operator supplies no server. The
mDNS browse + TXT-record contract moves out of `uptrakit-service-sdk` into a new small published
crate `uptrakit-zeroconf` (`crates/shared/zeroconf`), consumed by the service SDK (browser), the
controller-runtime advertiser, and the CLI. The CLI's trust flow keeps the interactive TOFU
ceremony as the security gate; the mDNS-advertised CA fingerprint is used only as a consistency
cross-check, never as a trust anchor.

Decisions locked with the owner during grilling:

- Discovery fires as an auto-fallback inside `auth login` only (no standalone `discover`
  subcommand, no `--discover` flag).
- Trust model: prompt + cross-check (never auto-pin the advertised fingerprint).
- Mechanism: full extraction of a shared `uptrakit-zeroconf` crate, accepting the published
  squat-chain ceremony this entails.

## Verified current state

Line numbers are hints (verified 2026-07-28); anchor on the named symbols.

- **Advertiser** — `crates/core/controller-runtime/src/zeroconf.rs`, feature `zeroconf`:
  `run_advertiser()` registers `SERVICE_TYPE = "_uptrakit._tcp.local."` (`pub(crate)` const,
  line ~20) on the HTTPS port with TXT records built by
  `build_txt_properties(&CaPublicSnapshot, &ZeroconfSnapshot) -> Vec<(&'static str, String)>`:
  `ca_fp` (always; SHA-256 of the active **CA certificate**, from
  `CaPublicSnapshot.active_fingerprint`), optional `url` (reverse-proxy override), optional
  `pki_addr`. The builder's inputs are `uptrakit-web-api` types, so it cannot move to a shared
  crate as-is.
- **Browser** — `crates/shared/service-sdk/src/discovery.rs`, feature `zeroconf`
  (default-on, `zeroconf = ["dep:mdns-sd"]`): duplicate `SERVICE_TYPE` const (line ~14),
  `browse_mdns()` (first-match, 10 s timeout, `spawn_blocking` recv loop),
  `cache_from_mdns(addresses, port, properties) -> Option<DiscoveryCache>` (TXT parse: `url`
  override wins, else first non-loopback IP + port; loopback-only yields `None`),
  `DiscoveryCache { url, pki_addr, ca_fingerprint }` persisted to `discovery.json`. The whole
  module uses grandfathered `Result<_, String>`.
- **CLI server resolution** — `crates/ui/cli/src/commands/auth.rs::login(server_override,
insecure, tofu)`: `--server`/`UPTRAKIT_SERVER` wins; else `config.server` (prompt with
  default); else `prompt("Server URL: ")` (line ~448). `login` never returns `NotLoggedIn` —
  that error belongs to `resolve_server_and_token` used by other commands. `prompt()` has no
  TTY guard: non-interactive stdin reads EOF → empty → `"server URL is required"`.
- **CLI trust ceremony** — `auth.rs::establish_ca_trust(server, fingerprint_hint,
allow_rotation, config)`: fetches `GET {server}/api/v1/pki/ca.crt` over an
  intentionally-insecure bootstrap client, computes the fetched CA fingerprint, then branches
  **either/or** (line ~133): with `fingerprint_hint` → compare and pin **without any prompt**;
  without hint + no TTY → bail; without hint + TTY → interactive `Trust this CA? [y/N]` prompt.
  `--insecure` and `--tofu` are mutually exclusive (clap `conflicts_with`).
- **Publishing** — `uptrakit-service-sdk` is `publish = true` inside the publishable
  squat-chain (`release-plz.toml`, "Public-API library crates" section; see
  `docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md`). Any new
  workspace dep of service-sdk must itself be published each cycle.
  `crates/shared/service-sdk/tests/no_workspace_db_deps.rs` is a **banned-list** walker
  (audit-log, audit-log-derive, shared-db, tenant-db, crypto) — a dep-free new crate passes it
  with no test edit. `release-plz.toml` has no `changelog_include` array on `uptrakit-cli`;
  `uptrakit-service-sdk`'s array lists its published deps (`uptrakit-wire`,
  `uptrakit-shared-types`, `uptrakit-surfaces`).
- **crates.io** — `uptrakit-zeroconf` is unclaimed (checked `cargo search uptrakit`,
  2026-07-28).
- **Docs** — existing zeroconf docs: `docs/end-user/zeroconf-discovery.md`,
  `docs/security/zeroconf-discovery.md`, `docs/development/zeroconf-discovery.md`. CLI docs:
  `docs/end-user/cli-usage.md`. `CONTEXT.md` carries a `"discovery"` ambiguity note
  distinguishing Software Discovery from Proxmox VE Discovery (grep hit ~line 300).

## Design

### 1. New crate: `uptrakit-zeroconf` (`crates/shared/zeroconf`)

Purpose: single home for the zeroconf **wire contract** (service type + TXT keys + parse/build)
and the browse primitives. Auto-membered via the `crates/shared/*` workspace glob.

Public API (authored new; exact rustfmt shape at plan time):

- `pub const SERVICE_TYPE: &str = "_uptrakit._tcp.local.";`
- `pub const TXT_KEY_CA_FP: &str = "ca_fp";` / `TXT_KEY_URL` / `TXT_KEY_PKI_ADDR` — the three
  TXT keys as named constants, used by both build and parse.
- `pub struct DiscoveredController { pub url: String, pub pki_addr: Option<String>, pub
ca_fingerprint: Option<String> }` — plain public-field struct mirroring the shape of
  service-sdk's `DiscoveryCache` (no serde; persistence stays in service-sdk).
- `pub fn parse_txt(addresses: &[IpAddr], port: u16, properties: &[(&str, &str)]) ->
Option<DiscoveredController>` — moved body of `cache_from_mdns` (url-override precedence,
  non-loopback selection, loopback-only → `None`). Existing unit tests move with it.
- `pub fn build_txt_properties(ca_fingerprint: &str, url: Option<&str>, pki_addr: Option<&str>)
-> Vec<(&'static str, String)>` — primitive-typed version of the controller-runtime builder;
  same output shape and ordering (`ca_fp` first, then optional `url`, then optional
  `pki_addr`). Existing builder tests move here in primitive form.
- `pub async fn browse_first() -> Result<Option<DiscoveredController>>` — moved body of
  `browse_mdns()` (first `ServiceResolved` wins, 10 s deadline, best-effort
  `stop_browse`/`shutdown`).
- `pub async fn browse_all(window: Duration) -> Result<Vec<DiscoveredController>>` — collects
  events for the full window; dedups on `(url, ca_fingerprint)`; preserves first-seen order.
  Shares the receive-loop internals with `browse_first` (one private loop, two termination
  policies).

Error handling (per `docs/development/error-handling.md`; do **not** inherit the grandfathered
`Result<_, String>`):

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ZeroconfError {
    /// The mDNS daemon could not be started, browsed, or shut down.
    #[error("mDNS daemon error")]
    Daemon(#[from] mdns_sd::Error),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<ZeroconfError>>;
```

No `impl_report_conversion!` on the same variant (the `#[from]` alone; the two together are
banned by error-handling.md). A panicked browse task keeps the current behavior: `tracing::warn!`
and treat as no result — it is not an error variant. Errors are built with `report!()`/`bail!()`.

Dependencies (all already in `[workspace.dependencies]`; the crate declares every feature it
uses itself, per the dependency policy): `mdns-sd` (workspace pin `0.20`, `default-features =
false`), `tokio` with `features = ["rt", "time"]` (`spawn_blocking` + deadline), `tracing`,
`thiserror`, `rootcause`. Dev: `tokio` `["macros", "rt", "test-util"]`. **No workspace-internal
deps** — this keeps the squat-chain banned-list test green with zero edits and makes the crate
trivially publishable. The browse timing tests, if any, must note that the loop reads
`tokio::time::Instant` (movable under `start_paused`); pure parse/build tests are plain `#[test]`.

### 2. Consumer rewiring

- **service-sdk**: feature becomes `zeroconf = ["dep:uptrakit-zeroconf"]`; `dep:mdns-sd` is
  dropped from the manifest. `discovery.rs` keeps `DiscoveryCache`, `discovery.json`
  load/save/clear, and `discover()`; `browse_mdns()` becomes a thin wrapper over
  `uptrakit_zeroconf::browse_first()` mapping `DiscoveredController` → `DiscoveryCache` and
  `Report<ZeroconfError>` → the module's existing `String` error at the boundary (no churn in
  `lifecycle.rs` callers). `SERVICE_TYPE` is re-exported from `uptrakit_zeroconf` so existing
  references and the published API stay stable.
- **controller-runtime**: `zeroconf.rs` drops its local `SERVICE_TYPE` and imports the shared
  const; `build_txt_properties(&CaPublicSnapshot, &ZeroconfSnapshot)` stays as a thin adapter
  calling `uptrakit_zeroconf::build_txt_properties(&ca_snapshot.active_fingerprint,
zeroconf.url.as_deref(), zeroconf.pki_addr.as_deref())`. Its cargo `zeroconf` feature now
  enables `dep:uptrakit-zeroconf` (still additive; no `cfg(not)` anywhere). Snapshot-typed
  tests stay in controller-runtime against the adapter.
- **cli**: unconditional dependency on `uptrakit-zeroconf` (no cargo feature in the CLI;
  `mdns-sd` is always in the CLI graph — accepted). The CLI does **not** gain a dependency on
  `uptrakit-service-sdk`.

### 3. CLI login flow

Discovery replaces only the bare `prompt("Server URL: ")` fallback arm in `login()`; the
`--server` and `config.server` arms are untouched.

Resolution order when neither `--server`/`UPTRAKIT_SERVER` nor `config.server` is set:

1. **TTY gate.** If `!std::io::stdin().is_terminal()`: bail immediately (before any browse)
   with guidance to pass `--server` (and `--tofu=<fingerprint>` for non-interactive pinning).
   This applies **also under `--insecure`** — discovery input comes from an unauthenticated LAN
   protocol, so unattended connect-to-whoever-answers is fail-open and is rejected.
2. **Browse.** Print a progress line ("Searching for a controller on the local network (up to
   10 s)…"), run `browse_all(Duration::from_secs(10))`.
3. **Zero found** → fall back to today's `Server URL:` prompt (behavior-compatible tail).
4. **One found** → print URL + advertised fingerprint (`SHA256:<fp>`, or "none advertised")
   and confirm `Use this controller? [y/N]`. Decline → fall back to the manual prompt.
5. **Multiple found** → numbered list (URL + fingerprint per entry), operator selects by
   number; empty/invalid input → fall back to the manual prompt. Selection parsing lives in a
   pure helper (`fn parse_selection(input: &str, count: usize) -> Option<usize>`) for direct
   unit testing. The stale/rogue second-advertiser case is exactly why first-match is not used
   here.

Trust ceremony on a discovery-accepted server (ordering is load-bearing):

1. If `--insecure`: skip the ceremony entirely — discovery filled the URL only; existing
   insecure semantics (warning banner, `tls_danger_accept_invalid_certs`) apply unchanged.
2. Else fetch the CA from the accepted URL and compute the fetched fingerprint (extract the
   existing fetch block of `establish_ca_trust` into a private `fetch_ca(server) -> Result<(pem,
fingerprint)>` helper reused by both; `establish_ca_trust`'s public contract is unchanged and
   it still performs its own fetch — the extra network round-trip is accepted, and the trust
   decision is always made against `establish_ca_trust`'s own fetch).
3. **Cross-check**: if the advertisement carried `ca_fp`, compare it (case-insensitive hex)
   with the fetched fingerprint. Mismatch → hard fail naming both values. If no `ca_fp` was
   advertised, skip the cross-check and continue.
4. If the user passed `--tofu=<fp>`: the user's fingerprint is authoritative — call
   `establish_ca_trust(server, Some(fp), false, config)` as today; additionally warn (do not
   fail) if the advertised `ca_fp` disagrees with the user's value.
5. Otherwise (no `--tofu`, or bare `--tofu`): call `establish_ca_trust(server, None, false,
config)` so the interactive `Trust this CA? [y/N]` prompt **always runs** and shows the
   fetched fingerprint. The advertised `ca_fp` is **never** passed as `fingerprint_hint` — that
   branch pins without a prompt, which would let a LAN attacker advertising a consistent
   `ca_fp` + rogue controller be trusted silently.

The discovery path therefore implies TOFU even when the user did not pass `--tofu`; the
`--server` path keeps today's behavior (system roots unless `--tofu` given). This asymmetry is
intentional: a discovered controller is presumed to run the internal CA that advertised
`ca_fp`. It is documented, not inferred.

Flag × state × TTY matrix (pinned):

| Server source    | `--tofu`       | `--insecure` | TTY | Behavior                                                                                                   |
| ---------------- | -------------- | ------------ | --- | ---------------------------------------------------------------------------------------------------------- |
| `--server` / env | any            | any          | any | Unchanged (no discovery).                                                                                  |
| `config.server`  | any            | any          | any | Unchanged (prompt with stored default; no discovery).                                                      |
| none             | any            | any          | no  | Bail with guidance before browsing (new message; replaces today's EOF → "server URL is required").         |
| none             | absent or bare | no           | yes | Browse → confirm/select → cross-check → interactive TOFU prompt.                                           |
| none             | `=<fp>`        | no           | yes | Browse → confirm/select → cross-check → pin against user fp (no prompt); warn on advertised/user mismatch. |
| none             | conflict       | yes          | —   | Clap `conflicts_with` rejects `--tofu` + `--insecure` (unchanged).                                         |
| none             | absent         | yes          | yes | Browse → confirm/select → no pinning, insecure semantics unchanged.                                        |

Observable behavior changes (intentional, named): the interactive no-server login gains up to
10 s of browsing before any prompt; the non-TTY no-server login fails earlier with a clearer
message. `auth login` is inherently interactive (browser device flow), so no script contract is
considered broken; the changed message is called out in the changelog via the feature commit.

### 4. Security model

- **The interactive fingerprint confirmation (or the operator's explicit `--tofu=<fp>`) is the
  only trust gate.** The advertised `ca_fp` is consistency hardening: an attacker controlling
  both the mDNS answer and the TLS endpoint passes the cross-check trivially. It defends
  against split control (attacker owns mDNS but not the controller endpoint) and against
  misconfiguration (stale advertisement pointing at a different controller). The spec and the
  security doc state this honestly; the cross-check is not sold as MITM protection.
- `ca_fp` hashes the internal **CA certificate** (verified: `CaPublicSnapshot.active_fingerprint`
  ← `ca_fingerprint(active CA PEM)`), and the CLI compares it against the CA fetched from
  `/api/v1/pki/ca.crt` — the reverse-proxy front certificate is not involved, so a public-cert
  proxy does not break the cross-check.
- **Reverse-proxy pinning trap (pre-existing, now first-class)**: after pinning the internal
  CA, the subsequent OAuth calls fail TLS if the deployment terminates TLS at a proxy with a
  public certificate. The security and end-user docs must state: TOFU CA pinning presumes the
  controller serves TLS with a certificate chaining to the internal CA; proxy-terminated
  public-cert deployments should use `--server` + system roots (and `uptrakit auth ca forget`
  if previously pinned). No detection logic in this iteration (deferred).
- **SSRF-omission rationale**: `establish_ca_trust`'s comment currently justifies omitting
  `SsrfSafeResolver` with "the operator chose the server URL". On the discovery path the URL is
  LAN-responder-influenced. The comment is re-scoped: mDNS yields LAN addresses the operator
  can reach anyway, the URL is displayed and explicitly confirmed by the operator before any
  request, and the CLI is operator-context (no server-side ambient authority). The interactive
  confirmation is named as the compensating control.
- No secrets are logged; fingerprints and URLs are not secrets. No new wire messages, no new
  HTTP endpoints, no `WireValidate` surface, no OpenAPI change.

### 5. Testing

Per `docs/development/testing.md` (success + failure paths; no upstream-crate behavior tests;
no live-mDNS/network tests — matching the existing precedent that `browse_mdns` itself has no
integration test):

- `uptrakit-zeroconf`: moved parse tests (`url_from_txt_override`, `url_from_mdns_ip_port`,
  IPv6, loopback-skip, loopback-only-none, `pki_addr`, TXT property lookup) plus moved builder
  tests in primitive form (basic / url override / all overrides, asserting order and content);
  new dedup test for the `browse_all` dedup helper (pure fn over already-parsed
  `DiscoveredController` values — the network loop itself stays untested, as today).
- `uptrakit-service-sdk`: existing `discovery.rs` cache tests unchanged (they exercise the
  cache layer that stays put); compile-level proof of the re-export (`SERVICE_TYPE`) via the
  existing references.
- `controller-runtime`: existing `build_txt_properties` snapshot-typed tests stay green against
  the adapter (they are the adapter's tests now).
- `uptrakit-cli`: unit tests for the new pure helpers — `parse_selection` (valid, out-of-range,
  garbage, empty), the advertised-vs-fetched cross-check (match, case-insensitive match,
  mismatch → error, absent-advertised → skip), and the server-resolution decision fn extracted
  so the discovery-vs-prompt-vs-flag precedence is testable without a network (fn takes
  `server_override: Option<&str>`, `config_server: Option<&str>`, and returns which path to
  take). Clap-surface tests unchanged (no new flags). The interactive browse/confirm loop and
  `establish_ca_trust` internals keep their current test posture (prompt-driven paths are not
  unit-testable without a TTY harness; the extracted pure fns carry the decision logic).

### 6. Registration and release ceremony (new crate)

All facts verified against the current tree; the plan re-runs each check at plan time.

1. Root `Cargo.toml` `[workspace.dependencies]`:
   `uptrakit-zeroconf = { path = "crates/shared/zeroconf", version = "0.0.1" }` (pattern:
   the existing `uptrakit-directories` entry). Consumers reference `workspace = true`
   (service-sdk and controller-runtime as `optional = true` behind their `zeroconf` features;
   cli unconditional).
2. Workspace membership: automatic via the `crates/shared/*` glob — no members edit.
3. `release-plz.toml`: add a `[[package]] name = "uptrakit-zeroconf"` entry with
   `git_release_enable = false` and `publish = true`, placed in the "Public-API library crates"
   section beside `uptrakit-shared-types`; add `"uptrakit-zeroconf"` to
   `uptrakit-service-sdk`'s `changelog_include` array. `uptrakit-cli` has no
   `changelog_include` array — no edit there. The `release_config_invariants.rs` sanity test
   (`git_only` + `publish` contradiction) passes as long as the entry does not set
   `git_only = true`.
4. crates.io: name is unclaimed; the first automated release cycle publishes and claims it. No
   manual publish step.
5. `no_workspace_db_deps.rs` (service-sdk + openapi-client): banned-list based — no edit
   needed; the new crate must simply never grow deps on the banned set.
6. `AGENTS.md`: one tree line under `crates/shared/` for `zeroconf/`.
7. Mechanical sweep at plan time: `grep -rln uptrakit-directories` (a sibling small shared
   crate) across the repo, excluding its own `src/` — every hit file is a candidate
   registration site for the new crate; reconcile the plan's file list against that output
   (never enumerate registration sites from memory).
8. `cargo deny check`: no new external dependencies; `mdns-sd` keeps its single workspace pin.
9. `ci/check_plugin_semantic_boundary.py`: plugins-only — unaffected.

### 7. Quality gates and verification

Canonical workspace gates (from `docs/development/quality-gates.md` via AGENTS.md) apply:
`cargo fmt --all`, `cargo check --no-default-features --features db-sqlite`,
`cargo check --all-features` (frontend built first), both clippy variants,
`cargo test --all-features`, `cargo deny check`, `markdownlint --config .markdownlint.json`.
Not triggered: `./scripts/regen-api.sh` (no route/REST change), `./scripts/regen-asyncapi.sh`
(no wire-type change), audit-coverage (no new state-changing handler), integration-test suites
(no reverse-proxy/DB/enrollment change).

Scoped per-crate gates the plan must list — and must first run **at baseline** to confirm each
invocation compiles on unmodified `main` before being cited as expected-PASS:

- `cargo clippy --all-targets -p uptrakit-zeroconf` and `cargo test -p uptrakit-zeroconf`
  (crate has no features).
- `uptrakit-service-sdk`, both feature worlds, clippy **and** test per world:
  default (zeroconf on) and `--no-default-features` (zeroconf off — the world where
  `lifecycle.rs` takes its "--url is required" fallback).
- `cargo clippy --all-targets -p uptrakit-controller-runtime --features zeroconf` (the world
  that compiles the advertiser adapter) plus that crate's canonical default-world gate.
- `cargo clippy --all-targets -p uptrakit-cli` and `cargo test -p uptrakit-cli`.

### 8. Documentation deliverables

Every file below is a named deliverable of the implementation; none are optional.

1. `docs/end-user/zeroconf-discovery.md` — new "CLI login discovery" section: behavior,
   confirmation flow, multi-controller selection, fallback to manual entry.
2. `docs/end-user/cli-usage.md` — `auth login` section updated with the discovery fallback and
   the flag matrix summary.
3. `docs/security/zeroconf-discovery.md` — CLI trust flow: interactive confirmation as the
   gate, cross-check framing (what it does and does not defend against), the fail-closed
   non-TTY rule, and the reverse-proxy pinning incompatibility guidance.
4. `docs/development/zeroconf-discovery.md` — implementation map updated for the crate split
   (who owns what: contract crate / advertiser adapter / SDK cache layer / CLI flow).
5. `docs/adr/0033-shared-zeroconf-crate.md` — new ADR: extraction rationale, crate boundary
   (contract + browse primitives only; persistence and snapshot-typed builders stay with their
   owners), published squat-chain membership and its cost, alternatives rejected.
6. `AGENTS.md` — tree line for `crates/shared/zeroconf`.
7. `CONTEXT.md` — extend the existing `"discovery"` ambiguity note (the pitfall entry that
   currently distinguishes Software Discovery from Proxmox VE Discovery) with the third sense:
   Zeroconf Discovery = locating the Controller via mDNS. Match the file's actual entry format
   when editing (open the file at plan time; do not assume a format).
8. Repo-wide staleness sweep: `grep -rni "zeroconf\|mdns" --include='*.md' .` (repo-wide, not
   just `docs/` — top-level ARCHITECTURE.md/README.md included; exclude `docs/superpowers/`)
   and reconcile every hit that describes the pre-split module layout or claims only services
   browse.

Rustdoc: the new crate gets crate-level docs stating the contract-ownership rule (TXT keys and
service type live here and nowhere else); moved functions keep their doc comments, updated for
the neutral types.

## Alternatives considered

- **CLI depends on `uptrakit-service-sdk`** — rejected: drags the WS/enrollment/PKI stack
  (tokio-tungstenite, rcgen, x509-cert, …) into the CLI graph for one browse call, and muddies
  the "CLI over openapi-client" crate purpose.
- **CLI-local minimal browse (duplicate)** — rejected: puts the security-relevant TXT/
  fingerprint contract in a third place with no drift guard; `SERVICE_TYPE` is already
  duplicated twice today and this feature is the occasion to fix that, not extend it.
- **Contract-only crate (consts + parse, CLI-local browse loop)** — rejected as dominated: the
  publish ceremony is identical to full extraction, the only difference is duplicating the
  browse loop.
- **Auto-pin the advertised `ca_fp` (service-style unattended TOFU)** — rejected: the CLI has a
  human present; skipping the interactive gate downgrades the existing login ceremony.
- **Pass advertised `ca_fp` as `fingerprint_hint` to `establish_ca_trust`** — rejected as
  insecure: the hint branch pins without a prompt, so a LAN attacker advertising a consistent
  fingerprint + rogue controller would be trusted with zero human interaction.
- **Standalone `discover` subcommand** — rejected by the owner (scope).

## Out of scope / deferred

- Standalone `uptrakit discover` subcommand (owner-rejected; revisit if scripting demand
  appears).
- CLI-side discovery cache (`config.json` persistence at login is sufficient).
- Multi-profile / multi-controller CLI config.
- Configurable browse timeout flag.
- Detection of proxy-terminated public-cert deployments during TOFU (documented guidance only).
- Back-fill of `browse_all` semantics into service enrollment (services keep first-match).

## Open questions

None. All grilling decisions are recorded above; the contrarian pass's findings (either/or
`establish_ca_trust` branch, `login()` baseline, squat-chain cost, banned-list test shape) were
verified against source before being incorporated.
