# ADR 0015 — ReleaseFetchContext: Extending ReleaseFetcher Factories

Date: 2026-05-13
Status: Accepted

## Context

The `package-manager.skills` plugin requires controller-side access to the global GitHub Provider
(for fetching git tree SHAs). Prior to this change, `ReleaseFetcher` factories received only
`(config_json, runtime: Arc<dyn HostRuntime>)`. The GitHub Provider is an instance-level singleton
that lives in `GlobalProviders`, not in `HostRuntime`.

## Decision

Introduce `ReleaseFetchContext` — a `#[non_exhaustive]` struct passed as a third argument to
`ReleaseFetcher` factory functions. The struct carries
`global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>` (gated on the `catalog` feature).
All existing factory functions add `_ctx: &ReleaseFetchContext` and ignore it.

## Alternatives Rejected

- **Adding `global_provider_lookup()` to `HostRuntime`:** `HostRuntime` is a host-execution
  abstraction. Leaking provider-registry concerns into it would require all implementations
  (`StandardHostRuntime`, `MetadataAwareHostRuntime`, `RouterOsHostRuntime`, `ControllerRuntime`)
  to carry the lookup even when they have no access to it, and would create delegation footguns
  in wrapper runtimes.
- **Scheduler-side token injection:** Would require a new per-plugin field in `PluginDescriptor`
  and an additional lookup phase inside the scheduler, duplicating what `GlobalProviders` already
  handles.
- **Per-plugin `auth_token` field in config:** Exposes a parallel credential surface. Operators
  must configure two places instead of one. Rejected for ergonomic and security reasons.

## Consequences

- `CreateReleaseFetcherFn` is now a 3-arg type alias (config, runtime, &context).
- All existing plugin factories get a mechanical `_ctx` parameter addition — no behaviour change.
- Future providers (GitLab, Forgejo) can be exposed through the same `ReleaseFetchContext`
  without changing the factory signature again.
- Standalone scheduler deployments pass `None`; the Skills plugin returns a clear error when the
  provider is absent.
