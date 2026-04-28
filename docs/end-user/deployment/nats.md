---
title: NATS Deployment Guide
weight: 100
description: Deploy Uptrakit with NATS JetStream to enable real-time cross-controller push notifications in multi-controller high-availability setups.
---

# NATS Deployment Guide

This guide covers deploying Uptrakit with NATS JetStream for multi-controller (high-availability) setups.
For development and testing details, see [NATS Integration](../../development/nats-integration.md).

## When to use NATS

NATS is required when running **multiple controller instances** behind a load balancer sharing a single
database. It enables real-time cross-controller push notification delivery so that a version check completed on
controller A can immediately push results to a service connected to controller B.

**Single-controller deployments do not need NATS.** The controller operates in single-instance mode by default.

## Requirements

- NATS server **with JetStream enabled** (NATS 2.6+)
- Uptrakit controller built with the `nats` Cargo feature
- Network connectivity between all controller instances and the NATS server

## Configuration

Pass the NATS server URL to each controller instance:

```bash
uptrakit-controller --nats-url nats://nats-server:4222
```

Or via environment variable:

```bash
export UPTRAKIT_NATS_URL=nats://nats-server:4222
uptrakit-controller
```

All controller instances must point to the same NATS server (or cluster).

## Docker Compose example

```yaml
services:
  nats:
    image: nats:latest
    command: ["-js"]
    ports:
      - "4222:4222"

  controller-1:
    image: uptrakit-controller:latest
    environment:
      UPTRAKIT_NATS_URL: nats://nats:4222
    depends_on:
      - nats

  controller-2:
    image: uptrakit-controller:latest
    environment:
      UPTRAKIT_NATS_URL: nats://nats:4222
    depends_on:
      - nats
```

## What NATS provides

- **Real-time cross-controller messaging**: Push notifications (version check results, certificate renewals,
  software state updates) are delivered across controllers with sub-second latency.
- **At-least-once delivery**: JetStream provides durable message delivery with explicit acknowledgement and
  automatic retry (up to 3 attempts).
- **Self-filtering**: Each controller ignores messages it originated, preventing echo loops.
- **Automatic stream management**: The controller creates and configures the JetStream stream on startup
  (idempotent — safe for concurrent startup).

## What NATS does not replace

- **Database**: The shared database remains the source of truth for all persistent state. NATS is used only
  for real-time push notification delivery.
- **Scheduler**: The DB-backed scheduler continues to use optimistic locking for HA-safe task execution. NATS
  does not affect scheduler coordination.
- **MQTT credential delivery**: Credential-bearing messages are always delivered locally only and never
  published to NATS.

## Security considerations

- MQTT credentials (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are never published to NATS.
  The MQTT service reconciles its state from the database on reconnect.
- Consider using NATS authentication and TLS in production. Refer to the
  [NATS documentation](https://docs.nats.io/running-a-nats-service/configuration/securing_nats) for details.
- Messages in the JetStream stream are retained for up to 24 hours. Ensure appropriate access controls on the
  NATS server.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| Controller fails to start with NATS error | NATS server unreachable or JetStream not enabled |
| Cross-controller messages not delivered | Controllers pointing to different NATS servers |
| Duplicate message processing | Normal — at-least-once delivery; handlers are idempotent |
| Messages delayed | NATS consumer pull interval (5s expiry); check NATS server load |

## Related documentation

- [Cross-Controller Communication](../../development/cross-controller-comm.md) — design overview
- [NATS Integration](../../development/nats-integration.md) — development and testing guide
- [System Overview](../system-overview.md) — high-level architecture
