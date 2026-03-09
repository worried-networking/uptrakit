# API and Protocol Documentation

This folder documents the controller HTTP API, service/controller wire protocol, authentication flows, and runtime settings behavior.

## Contents

| Document | Description |
| --- | --- |
| [Wire Protocol](wire-protocol.md) | WebSocket lifecycle, message types, and AsyncAPI mapping for controller/service communication. |
| [HTTP Web API](http-web-api.md) | REST endpoints, request/response behavior, rate limits, and multi-tenancy notes. |
| [Settings Runtime](settings-runtime.md) | Settings storage, reconciliation, snapshots, and runtime reload behavior. |
| [Auth Flows](auth-flows.md) | Login/device authorization, token lifecycle, and enrollment-related auth flows. |
| [Services and Operations](services-operations.md) | Agent and MQTT service lifecycle operations, update execution, and state transitions. |
| [Notifications](notifications.md) | Notification channels, rules, delivery log, and Telegram callback endpoints. |
| [Batch Actions](batch-actions.md) | Batch/group action endpoints for bulk management operations across all resources. |
| [Host Tags](host-tags.md) | Host tag CRUD, batch delete, and host tag assignment endpoints. |

## Related Documentation

- Top-level docs catalogue: [`docs/README.md`](../README.md)
- End-user workflows: [`docs/end-user/README.md`](../end-user/README.md)
- Security model: [`docs/security/README.md`](../security/README.md)
- Development workflow: [`docs/development/README.md`](../development/README.md)
- AsyncAPI source: [`crates/shared/wire/asyncapi.yaml`](../../crates/shared/wire/asyncapi.yaml)
