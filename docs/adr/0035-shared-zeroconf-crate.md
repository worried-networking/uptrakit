# 0035 — Shared Zeroconf Crate

**Date:** 2026-07-29 **Status:** Accepted

## Context

`SERVICE_TYPE` and the TXT contract (`ca_fp`, `url`, `pki_addr`) were duplicated between the
controller-runtime advertiser and the service-sdk browser — a security-relevant cross-process
contract with no drift guard. The advertiser builds the TXT record a discovering client parses;
if the two copies diverge (a renamed key, a reordered field, a changed service type string),
services silently stop discovering the controller, or worse, discover a URL/fingerprint pairing
whose fields were parsed against the wrong keys. Nothing in CI caught that divergence because the
two implementations lived in different crates with no shared source of truth. The CLI becoming a
third consumer — needing the same `SERVICE_TYPE` and TXT parsing to implement `uptrakit auth login`
zeroconf discovery — forced the decision: a third hand-copy of a security contract is the point
past which duplication has to become extraction.

## Decision

Extract `uptrakit-zeroconf` (`crates/shared/zeroconf`) owning the service type (`SERVICE_TYPE`),
the TXT keys (`TXT_KEY_CA_FP`, `TXT_KEY_URL`, `TXT_KEY_PKI_ADDR`), primitive-typed build/parse
(`build_txt_properties`, `parse_txt`), and the browse primitives (`browse_all`, `browse_first`).
The controller-runtime advertiser builds its TXT records through `build_txt_properties`; every
browser (service-sdk, the CLI) parses through `parse_txt`. This is the single home for the
contract: no other crate may redefine the service type string or the TXT key names.

Persistence (`DiscoveryCache`) stays in service-sdk — it is a service-lifecycle concern, not part
of the wire contract, and pulling it into the shared crate would drag service-sdk's persistence
model into a crate the CLI also depends on. The snapshot-typed builder stays in controller-runtime
as an adapter: its input types (the controller's live TLS/PKI snapshot) cannot enter a shared
crate without pulling controller-runtime's snapshot types down into a leaf dependency of the CLI
and every service — `uptrakit-zeroconf` only knows about primitive `&str`/`IpAddr`/`u16` inputs
and outputs, and controller-runtime is responsible for adapting its own snapshot into those
primitives before calling `build_txt_properties`.

The crate is `publish = true`: `uptrakit-service-sdk` is in the published squat-chain (per the
squat-chain-break design referenced from `release-plz.toml`), so every workspace dependency of it
must be published each release cycle — this cost was accepted explicitly, the same way it was for
`uptrakit-shared-types`, `uptrakit-surfaces`, and `uptrakit-wire` before it. Its sole workspace
dependency is `uptrakit-shared-macros` (published, and not on the `no_workspace_db_deps.rs` banned
list enforced against service-sdk and openapi-client), so it does not reintroduce a banned
transitive dependency into the squat chain.

## Consequences

- One more crate in the release cycle and in `release-plz.toml`'s `changelog_include` sets for the
  binaries that depend on it transitively (controller, agent, agent-ssh, mqtt, scheduler, cli).
- The `SERVICE_TYPE` + TXT contract has a single home; the controller-runtime advertiser and every
  browser (service-sdk, CLI) compile against the same constants and the same `parse_txt`/
  `build_txt_properties` functions, so a future key rename or reordering is a one-crate change that
  every consumer picks up at the next `cargo update`, not three independent hand-edits.
- `#[non_exhaustive]` on `ZeroconfError` and `DiscoveredController`, plus the crate's `0.0.x`
  version, absorb API churn without forcing a major-version bump while the contract stabilizes.

## Alternatives rejected

- **CLI depends on service-sdk directly.** Rejected: service-sdk's dependency surface includes the
  WebSocket event loop, enrollment, and PKI/identity stack — none of which the CLI's login flow
  needs; depending on it purely to reach the zeroconf browse primitives would drag that whole stack
  into the CLI binary.
- **CLI-local duplicate of `SERVICE_TYPE`/TXT parsing.** Rejected: this is exactly the failure mode
  in Context — a third hand-copy of a security-relevant contract with no drift guard, one incident
  away from silently diverging from the advertiser.
- **A contract-only crate (constants + parse/build, no browse primitives).** Rejected: the browse
  loop itself (the `mdns-sd` daemon lifecycle, adaptive-settle collection) is dependency and
  drift-risk of the same shape as the TXT contract — splitting it into a fourth crate would incur
  the same publish cost as `uptrakit-zeroconf` while still leaving the browse loop duplicated
  between service-sdk and the CLI.

## Cross-references

- Crate: `crates/shared/zeroconf` (`uptrakit-zeroconf`)
- Advertiser (controller-runtime): `crates/core/controller-runtime`
- Browser (service-sdk): `crates/shared/service-sdk`
- Browser (CLI): `crates/ui/cli/src/discovery.rs`
- Publish policy: `release-plz.toml`
- Squat-chain design: `docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md`
- Banned-dependency guard: `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`
