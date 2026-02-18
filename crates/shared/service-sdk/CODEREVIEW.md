# Code Review: uptrakit-service-sdk

Extensibility-focused review of the service SDK crate.

## Role in the Architecture

This crate provides the lifecycle management framework for services (agents, MQTT service) that
connect to the controller. External developers creating new service types would use this crate as
their primary dependency.

## Findings

### Significant: ServiceType enum constrains new service types

The service enrollment flow requires a `ServiceType` value (from `shared-types`), which is a
closed enum with 3 variants (`Agent`, `Mqtt`, `SshAgent`). An external developer creating a new
service type (e.g., a Kubernetes operator) must modify the `ServiceType` enum in `shared-types`
before they can enroll their service.

**Impact:** The SDK itself is well-designed for extensibility, but the type system blocks external
service creation at the enrollment boundary.

**Recommendation:** Consider accepting a string-based service type identifier at enrollment time,
with the `ServiceType` enum used only for well-known built-in types. The controller could accept
unknown service types and store them as strings.

## Positive Observations

- **`ServiceHandler` trait is well-designed** for creating new services. It provides clear
  lifecycle hooks (`on_approved`, `on_message`, `on_disconnect`) with sensible defaults.
- **`run_service_lifecycle` orchestrates the full lifecycle** -- enrollment, certificate
  management, reconnection with backoff, and graceful shutdown -- so service developers only
  implement business logic.
- **`CertificateRenewalHandler`** eliminates boilerplate certificate management across services.
- **`ServiceEnrollmentInfo`** provides a clean data structure for enrollment metadata.
- **`AuthenticatedContext`** encapsulates the authenticated connection state without exposing
  raw WebSocket details.
- **`Backoff`** provides configurable exponential backoff with jitter for reconnection.
- **Good documentation** with a minimal example in `docs/development/service-lifecycle.md`.
- Clean dependency chain: depends on `wire`, `directories`, `shared-types` -- no database or
  provider dependencies.
