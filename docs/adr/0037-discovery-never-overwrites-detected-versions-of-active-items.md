# 0037 — Discovery never overwrites detected versions of active items

Date: 2026-08-04

## Status

Accepted

## Context

Autodiscovery previously wrote `host_software_items.installed_version` unconditionally on every
run for every matched item. When an operator customizes the `detect_version` role assignment
(the grocy case: the Proxmox-Helper-Scripts version file goes stale after out-of-band updates),
the discovery-reported value and the `DetectVersion` scheduled check disagree, and the stored
version flip-flops between wrong (each 6-hour discovery pass) and right (each daily check). The
prior documented invariant ("re-discovery only updates versions for autodiscovery-created
items") was not enforced by any provenance check.

## Decision

Discovery never overwrites a non-NULL `installed_version` on a `host_software_items` row that
was already active. The skip predicate is evaluated per matched row at both existing-row write
sites in `find_or_create_software_item`. Version fields (`installed_version`,
`installed_display_version`, `installed_version_detected_at`) are still written on fresh
inserts, on link-level reactivation (the matched row had `deactivated_at` set — revival is
re-registration), and when the stored version is NULL (filling overrides nothing).
Presence/provenance stamps (`last_discovered_at`, `discovery_source`, clearing `missing_since`
and `deactivated_at`) remain unconditional. "Reactivation" is defined at the link level, not the
software-item level: a cascade repoint onto an always-active link preserves that link's version.
For active items the `DetectVersion` scheduled task is the sole version writer.

## Consequences

The freshness regression is universal: every active discovered item, including the majority
whose discovery source is live and correct, moves from 6-hour to daily (worst-case 24-hour)
version refresh. In-app updates are unaffected because `UpdateResult` finalization writes the
version directly. An item that flaps (deactivated then re-discovered) re-admits the
discovery-reported value once per revival until the next daily check.

Active invariant gap, accepted knowingly: nothing structurally guarantees every registered item
a `detect_version` assignment — the target-based registration path creates only the roles listed
on each `DiscoveryTarget`. A discovery-only item registered in the future would keep its last
written version indefinitely, previously masked by the 6-hour discovery overwrite. The live
deployment was audited point-in-time (all active rows carry a `detect_version` assignment).
Detection-staleness observability is deferred follow-up work.
