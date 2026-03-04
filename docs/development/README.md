# Development Documentation

This folder contains contributor-facing implementation standards and workflows for building, testing, and maintaining
Uptrakit.

## Contents

| Document | Description |
| --- | --- |
| [Setup](setup.md) | Local environment setup, prerequisites, and baseline build commands. |
| [Testing](testing.md) | Required test suites, coverage expectations, and execution guidance. |
| [Coding Standards](coding-standards.md) | Core coding rules and quality constraints. |
| [Error Handling](error-handling.md) | rootcause/thiserror patterns, decision guide, anti-patterns, and approved exceptions. |
| [PR Process](pr-process.md) | Pull request requirements, review expectations, and merge readiness checklist. |
| [Dependency Policy](dependency-policy.md) | Dependency introduction/update rules and `cargo deny` policy. |
| [Plugin Guidelines](plugin-guidelines.md) | Plugin architecture expectations, capabilities, host compatibility detection, and lifecycle hooks. |
| [Plugin System](plugin-system.md) | Plugin system architecture, discovery flow, and capability extension model. |
| [AI Guidelines](ai-guidelines.md) | Policy for using AI tools in project development. |
| [CLI Output](cli-output.md) | CLI output formatting conventions and standards. |
| [Commit Messages](commit-messages.md) | Conventional Commits format and examples. |
| [Database Migrations](database-migrations.md) | Migration authoring guide: naming, skeleton, registration, testing, and backend feature flags. |
| [Cross-Controller Communication](cross-controller-comm.md) | HA controller-to-controller event propagation via NATS JetStream. |
| [NATS Integration](nats-integration.md) | NATS JetStream development guide: feature flags, architecture, testing. |
| [Graceful Restart](graceful-restart.md) | Zero-downtime restart and shutdown behavior. |
| [Quality Gates](quality-gates.md) | CI quality gate requirements for all changes. |
| [Update Hooks](update-hooks.md) | Pre/post-update hook execution and configuration. |
| [Command Executor](command-executor.md) | `CommandExecutor` trait, `CommandSpec`, and `LocalCommandExecutor` for transport-agnostic command dispatch. |
| [Service Lifecycle](service-lifecycle.md) | `ServiceHandler` trait and `run_service_lifecycle()` for building new services. |
| [OpenAPI Client](openapi-client.md) | Typed HTTP client crate for the web API (`uptrakit-openapi-client`). |
| [Embedded Frontend](embedded-frontend.md) | Building the controller with the frontend embedded in the binary (`embed-frontend` feature). |
| [Logging](logging.md) | Logging infrastructure, verbosity flags, `RUST_LOG` interaction, and best practices. |
| [Releases](releases.md) | release-please workflow, binary artifact matrix, attestation verification, and `cargo install` instructions. |

## Related Documentation

- Top-level docs catalogue: [`docs/README.md`](../README.md)
- Security requirements: [`docs/security/README.md`](../security/README.md)
- API/protocol behavior: [`docs/api/README.md`](../api/README.md)
- End-user behavior and workflows: [`docs/end-user/README.md`](../end-user/README.md)
