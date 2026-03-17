# Code Review: `uptrakit-nats`

- Review date: 2026-03-17
- Scope: current-state review (full 14-dimension)

## Summary

The NATS wrapper crate is in good shape. Startup retry behavior, plaintext warning behavior,
credential-bearing message handling, and config transit encryption are all solid.

## Strengths

- Connection startup has bounded retry/backoff (10 attempts, 1s-30s exponential backoff)
  instead of immediate failure.
- Plaintext NATS URL (`nats://`) triggers a `tracing::warn!` with a link to the security docs.
- Credential-bearing controller messages are explicitly blocked from publication via
  `is_nats_publishable()`.
- Config transit encryption (`encrypt_message_configs`/`decrypt_message_configs`) provides
  AES-256-GCM protection for plugin credentials in transit over NATS, with dedicated AAD
  (`uptrakit:nats:config_transit`).
- Backward compatibility: `decrypt_message_configs` handles non-encrypted configs gracefully
  during rolling upgrades.
- Encryption/decryption failures use graceful degradation (leave config unchanged) rather than
  crashing.
- The crate stays small and focused around JetStream wiring rather than accumulating business
  logic.
- Thorough roundtrip tests for all credential-bearing message variants.

## Active Findings

No active findings were confirmed in this review pass.
