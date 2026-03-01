# Architecture Documentation

This folder contains entity-level architecture documentation for Uptrakit's core domain model.

## Contents

| Document | Description |
| --- | --- |
| [Multi-tenancy](multi-tenancy.md) | Database and API multi-tenancy model (currently single-tenant). |
| [Host Entity](host-entity.md) | Host representation, `machine_id` tracking, and agent linking. |
| [Software Item Entity](software-item-entity.md) | Software item definition, plugin configuration, and host assignment. |
| [Update History Entity](update-history-entity.md) | Immutable update history records, batch updates, and status tracking. |
| [Scheduler](scheduler.md) | Centralised DB-backed task scheduler with HA-safe optimistic locking. |
| [SSH Agent](ssh-agent.md) | SSH-backed agent architecture and local DB schema. |

## Related Documentation

- High-level architecture overview: [`ARCHITECTURE.md`](../../ARCHITECTURE.md)
- End-user system overview: [`docs/end-user/system-overview.md`](../end-user/system-overview.md)
- Wire protocol: [`docs/api/wire-protocol.md`](../api/wire-protocol.md)
- Top-level docs catalogue: [`docs/README.md`](../README.md)
