# Development Documentation

This folder contains contributor-facing implementation standards and workflows for building, testing, and maintaining
Uptrakit.

## Contents

| Document | Description |
| --- | --- |
| [Setup](setup.md) | Local environment setup, prerequisites, and baseline build commands. |
| [Testing](testing.md) | Required test suites, coverage expectations, and execution guidance. |
| [Coding Standards](coding-standards.md) | Core coding rules, error handling conventions, and quality constraints. |
| [PR Process](pr-process.md) | Pull request requirements, review expectations, and merge readiness checklist. |
| [Dependency Policy](dependency-policy.md) | Dependency introduction/update rules and `cargo deny` policy. |
| [Provider Guidelines](provider-guidelines.md) | Provider architecture expectations and implementation conventions. |
| [AI Guidelines](ai-guidelines.md) | Policy for using AI tools in project development. |
| [CLI Output](cli-output.md) | CLI output formatting conventions and standards. |
| [Commit Messages](commit-messages.md) | Conventional Commits format and examples. |
| [Cross-Controller Communication](cross-controller-comm.md) | HA controller-to-controller event propagation. |
| [Graceful Restart](graceful-restart.md) | Zero-downtime restart and shutdown behavior. |
| [Quality Gates](quality-gates.md) | CI quality gate requirements for all changes. |
| [Update Hooks](update-hooks.md) | Pre/post-update hook execution and configuration. |
| [Command Executor](command-executor.md) | `CommandExecutor` trait, `CommandSpec`, and `LocalCommandExecutor` for transport-agnostic command dispatch. |
| [Service Lifecycle](service-lifecycle.md) | `ServiceHandler` trait and `run_service_lifecycle()` for building new services. |

## Related Documentation

- Top-level docs catalogue: [`docs/README.md`](../README.md)
- Security requirements: [`docs/security/README.md`](../security/README.md)
- API/protocol behavior: [`docs/api/README.md`](../api/README.md)
- End-user behavior and workflows: [`docs/end-user/README.md`](../end-user/README.md)
