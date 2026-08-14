# 0042 — Untenanted System-Service Surfaces and a Single Runtime-Owned MQTT Definition

Date: 2026-08-14

## Status

Accepted

## Context

Before this change the MQTT settings surface registered with `Scope::Tenant` / `Targeting::Targeted`,
bound to the deployment's default tenant, and registration was skipped entirely — logged as "skipping
MQTT settings surface registration: tenant binding unavailable" — whenever no default tenant could be
resolved to bind to. This was a structural mismatch: a single MQTT service instance manages broker
connections for multiple tenants simultaneously (tenancy is a property of each MQTT client config row,
not of the provider registration as a whole), so binding the whole registration to one tenant was wrong
in general, not just in the no-default-tenant edge case.

Symmetrically, `MqttRuntime::validate_surface_request_context` compared the incoming request's tenant
against a `service_tenant_id` the runtime carried on `MqttRuntimeSettings` (`tenant_id`,
`with_tenant_id()`) — conflating "the tenant this service belongs to" with "the tenant the caller is
acting on behalf of," a category error for a system service that has no tenant of its own.

Separately, MQTT's deployment facts — app name, capability set, `ServiceScope`, `YieldPolicy`, embedded
shutdown timeout — were declared twice: once for the standalone binary
(`crates/core/mqtt/src/main.rs`) and once in `crates/core/controller-runtime/src/mqtt/`
(`mqtt_capabilities()`, `send_initial_service_config()`,
`controller-runtime::durations::EMBEDDED_MQTT_SHUTDOWN_TIMEOUT`). The two copies could drift silently: a
capability or yield-policy change made in one path left the other stale with no compiler signal tying
them together.

`send_initial_service_config()`, the embedded-only predecessor of today's config-delivery path, also
returned early with no audit trail whenever a service's entry set was empty — an idle embedded service's
config delivery was invisible in the audit log, unlike the external (WebSocket) path, which always
audited delivery, success or failure.

Finally, a yielded embedded service (mid-handoff to an external counterpart advertising the same app
name) kept occupying its `Universal`-targeted surface registration until it next processed a message, so
an external service claiming the same `surface_id` could be rejected with a `ProviderConflict` even
though the embedded instance had already stopped serving traffic.

## Decision

Two changes, shipped together: system-service surface registrations move from tenant-bound to
Global/Universal with per-request tenant resolution, and the MQTT service's deployment facts collapse
to a single runtime-owned definition.

### System-service surfaces are Global/Universal; tenancy is resolved per request

- `crates/core/mqtt-runtime/src/surface_runtime.rs::build_surface_registration_with_ids` registers with
  `Scope::Global`, `Targeting::Universal`, `effective_tenant_binding.tenant_id: None`, and the
  `Capability::UniversalTargeting` capability. The empty registration sent on yield
  (`build_empty_surface_registration`) carries the identical Global/`None` binding.
- `MqttRuntime::validate_surface_request_context` no longer compares against any service-held tenant id.
  It requires the server-stamped `SurfaceActionRequest.tenant_id` — stamped by
  `crates/ui/surface-proxy/src/proxy/dispatch.rs`, never client-supplied — and accepts either
  `target_provider_id: None` (Universal auto-resolution) or an exact provider-id match.
  `MqttRuntimeSettings::tenant_id`, `with_tenant_id()`, and `MqttRuntime::service_tenant_id` are removed;
  the runtime tracks only `ui_surfaces_enabled`. `handle_clients_action` / `handle_get_item` /
  `handle_list_items` all take a `tenant_id: Uuid` parameter sourced from that per-request value and
  filter MQTT client configs by `cfg.tenant_id == tenant_id` — tenancy lives on each client config row,
  never on the provider registration.
- `AdminEvent::SurfacesChanged` is broadcast via `EventBroadcaster::send_global` for untenanted
  (Global-scope) providers and per-tenant otherwise, so connected admin UIs across every tenant refresh
  when a Universal provider's surface set changes.
- Yield handoff is deterministic, not eventual:
  `crates/ui/web-api/src/routes/service_ws/handler/surface_eviction.rs::evict_yielded_service_surfaces`
  unregisters the surface provider of every currently-yielded service (via the new
  `SurfaceRegistry::service_ids()` accessor) immediately after
  `EmbeddedServiceNotifier::on_external_connected` flips the yielded flag — before the newly-connected
  external service's own registration is admitted — and fails the evicted provider's in-flight requests
  via `SurfaceProxy::fail_in_flight_for_provider`. Service-side, `MqttRuntime::handle_yield_change` sends
  an empty registration on yield and the full one again on resume.

### One runtime-owned MQTT service definition

- `crates/core/mqtt-runtime/src/bootstrap.rs` is now the sole source of `MQTT_SERVICE_APP_NAME`,
  `capabilities()` (`SystemService`, `UpdateTracking`, `GracefulShutdown`, `UiSurfaces`,
  `WorkloadClaims`), `SCOPE` (`ServiceScope::System`), `YIELD_POLICY`
  (`YieldPolicy::SameServiceAnywhere`), `EMBEDDED_SHUTDOWN_TIMEOUT`, `new_handler()`, and `run_embedded()`.
  Both the standalone binary and the controller's embedded-service host
  (`crates/core/controller-runtime/src/service_host/builtins.rs`) read these facts instead of each
  declaring their own.
- The whole `crates/core/controller-runtime/src/mqtt/` module — including its separate
  `mqtt_capabilities()` and `send_initial_service_config()` — and
  `controller-runtime::durations::EMBEDDED_MQTT_SHUTDOWN_TIMEOUT` are deleted; there is no second place
  left for these facts to drift from `bootstrap.rs`.
- System bridges (scheduler, MQTT) are untenanted by construction: `run_embedded_system_message_handler`
  lost its `service_tenant_id` parameter, `EmbeddedBridgeMode::System` lost its payload, and the former
  per-service `spawn_scheduler_bridge` became `spawn_system_bridge(label, …)`, shared verbatim by both
  the scheduler and MQTT embedded bridges.
- Config delivery is one path for every embedded service. `service_config.rs`'s
  `load_delivery_entries(&ServiceScopeCtx<'_>)` is the transport-independent core (DB load plus
  audit-emission-on-load-failure) shared by `deliver_service_config_with_sink` (external, over the
  mTLS WebSocket) and the new `deliver_service_config_embedded(&ServiceScopeCtx<'_>)` (in-process, pushed
  through `state.service_connections`). `run_embedded_message_handler_inner` calls the embedded path for
  every embedded service — agent, agent-ssh, scheduler, MQTT — so each gets the same
  `SERVICE_CONFIG_DELIVER` audit trail, including on an empty entry set: the deleted
  `send_initial_service_config` previously skipped both delivery and audit, silently, whenever a service
  had no stored config.
- Default-tenant special-casing for system services is deleted outright:
  `system_service_tenant_binding` (`service_ws/handler/shared_types.rs`), the web-api-local copy of
  `MQTT_SERVICE_APP_NAME`, and `resolve_settings_tenant_id` (`service_ws/connection.rs`) are gone. A
  system service's `ServiceSettingsPayload.tenant_id` is now always `None`; a tenant service's carries
  its own tenant id. Settings construction moved into one pure `build_service_settings(…)` helper used
  by every call site.

## Consequences

- The MQTT settings surface can no longer silently fail to register for lack of a default tenant to bind
  to — the "skipping MQTT settings surface registration: tenant binding unavailable" failure mode is
  structurally impossible now, because Global/Universal registration has no tenant dependency at
  admission time.
- Any future system service that grows a shared surface gets both invariants for free by following the
  same pattern: register Global/Universal, and resolve tenant scope from
  `SurfaceActionRequest.tenant_id` per request rather than from a service-held tenant id.
- The single `mqtt-runtime::bootstrap` module makes future MQTT deployment-fact changes (capability set,
  yield policy, shutdown timeout) single-edit by construction — there is exactly one place left to make
  them, so the two deployment modes can no longer independently drift.
- Every embedded service, not just MQTT, now emits a config-delivery audit entry unconditionally,
  including on first boot with zero stored config. This adds one `SERVICE_CONFIG_DELIVER` event per
  embedded service per (re)connect, accepted as the cost of parity with the always-audited external path.
- Yield handoff no longer risks a spurious `ProviderConflict`: eviction of the yielded embedded service's
  Universal registration is synchronous with the yield-flag flip rather than eventual.
- Two existing documentation claims needed correction as a direct result of this decision:
  `docs/architecture/surfaces.md`'s prior claim that "service registrations are tied to authenticated
  service identity **and tenant context**" no longer holds unconditionally (system services register
  with no tenant context at all), and `docs/development/service-config-store.md`'s prior claim that
  config delivery happens "after mTLS authentication" no longer describes the embedded delivery path,
  which never performs an mTLS handshake. Both are corrected alongside this ADR; see
  [System Services](../architecture/system-services.md#surfaces-and-config-delivery-are-untenanted) for
  the consolidated end-state description.
