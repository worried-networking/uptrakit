---
title: TOFU and TLS Hardening
weight: 60
description: Four explicit TOFU modes, trust composition options, ServerName binding, and operator override semantics for Uptrakit Agents and Services.
---

# TOFU and TLS Hardening

## Overview

Uptrakit Agents and Services verify the Controller's TLS certificate via one of four explicit modes selected at boot.
The historical bare `--tofu` flag is removed; mode is determined by which CLI flag is present.

## Modes

| Flag                          | Mode              | Server-cert check                                                     | `ServerName`                                 |
| ----------------------------- | ----------------- | --------------------------------------------------------------------- | -------------------------------------------- |
| (none)                        | `system`          | Trust composition store; chain + expiry + key-usage required          | Enforced                                     |
| `--tofu-fingerprint=<sha256>` | `pin-fingerprint` | Any chain accepted iff CA bundle SHA-256 matches                      | Enforced; opt-out via `--tofu-skip-hostname` |
| `--tofu-spki=<sha256>`        | `pin-spki`        | Any chain accepted iff any cert's `SubjectPublicKeyInfo` hash matches | Enforced; opt-out via `--tofu-skip-hostname` |
| `--tofu-insecure`             | `insecure-tofu`   | Accept any chain; `WARN` every connection                             | Off (forced)                                 |

Each `pin-*` / insecure flag conflicts with `--ca-cert` and `--pki-addr`.
`--tofu-skip-hostname` requires a `pin-*` or `--tofu-insecure` flag.

## Trust composition

| Flag                   | Effect                                                          |
| ---------------------- | --------------------------------------------------------------- |
| (none, default)        | Controller-CA bundle only (today's behavior).                   |
| `--trust-public-roots` | Add compiled-in `webpki-roots`.                                 |
| `--trust-native-roots` | Add OS root store via `rustls-native-certs` at process startup. |

Native roots are loaded once at startup. To pick up OS-level changes (admin pushes new corporate root), restart the Agent.

## Persistence semantics

- **`pin-fingerprint`**: on first successful connection where the fetched CA bundle's SHA-256 matches
  `--tofu-fingerprint`, the bundle is persisted to `service.json` as if `--ca-cert` had been used.
  Subsequent reconnects use the `system` verifier with the bundle in the root store. The flag is no longer
  required after persistence; if supplied on a later run, the on-disk bundle is validated against it and a
  mismatch fails startup.
- **`pin-spki`**: same persistence flow. The matched SPKI hash is stored alongside the bundle so future
  renewals validating via the same flag confirm key continuity.
- **`insecure-tofu`**: stateless TOFU by default — every reconnect re-fetches the bundle, no persistence,
  `WARN` log every connection. To persist, supply `--tofu-fingerprint-acknowledge=<sha256>` matching the
  fingerprint observed on the previous run. Mismatch → exit non-zero with both fingerprints logged at `ERROR`.

## ServerName binding

Server-cert SAN must include the dialed hostname. Disable with `--tofu-skip-hostname` (only valid alongside
a `pin-*` mode; implied by `--tofu-insecure`). Use case: development with IP addresses or hostnames not in
the cert SAN.

## Examples

LE-fronted Controller:

```sh
uptrakit-agent --trust-public-roots
```

Self-signed Controller, fingerprint-pin first contact:

```sh
uptrakit-agent \
  --tofu-fingerprint=aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99
```

SPKI-pin (survives cert renewals):

```sh
uptrakit-agent \
  --tofu-spki=11:22:33:44:55:66:77:88:99:00:aa:bb:cc:dd:ee:ff:11:22:33:44:55:66:77:88:99:00:aa:bb:cc:dd:ee:ff
```

Development against a Controller serving IP-only:

```sh
uptrakit-agent --tofu-insecure # implies --tofu-skip-hostname; WARN logged
```

Corporate internal CA Agent:

```sh
uptrakit-agent --trust-native-roots
```

## CLI CA Trust

The `uptrakit` CLI uses a separate Trust-On-First-Use mechanism for connecting to Controllers
with internally-generated (self-managed) CAs. This is distinct from TOFU mode, which is
scoped to Services (Agents, MQTT, etc.).

| Command                              | Purpose                                                 |
| ------------------------------------ | ------------------------------------------------------- |
| `uptrakit auth login --tofu`         | Bootstrap CA trust during first login                   |
| `uptrakit auth login --tofu=<fp>`    | Non-interactive bootstrap with fingerprint verification |
| `uptrakit auth ca trust`             | Establish or rotate CA trust independently of auth      |
| `uptrakit auth ca trust --tofu=<fp>` | Non-interactive CA trust update                         |
| `uptrakit auth ca status`            | Show stored CA fingerprint                              |
| `uptrakit auth ca forget`            | Remove stored trust, revert to system roots             |

### Fingerprint format

SHA-256 of the CA certificate's DER bytes, encoded as 64-character lowercase hex.
This matches the fingerprint shown in Dashboard → Global Settings. Accepts a `sha256:` prefix.
Example: `e3676c6137dada24f41974e2fb62546dadc2c6d6b831e4bb2635393218c64ce4`

### CA rotation recovery

```sh
# Interactive (fingerprint visible in Dashboard > Global Settings)
uptrakit auth ca trust

# Non-interactive (CI)
uptrakit auth ca trust --tofu=<fingerprint>

# Migration to public CA (Let's Encrypt)
uptrakit auth ca forget
```

`auth login --tofu` deliberately fails when a stored CA differs from the fetched one,
even when an explicit fingerprint is supplied. Use `auth ca trust` for rotation.

### `--insecure` interaction

`--insecure` overrides all stored CA trust. When `--insecure` is active, `ca_fingerprint`
is `None` in `auth status` output.

## Removed: bare `--tofu`

The historical bare `--tofu` flag is removed in this release. Operators using it must
choose explicitly: pin via fingerprint or SPKI, or accept any chain via `--tofu-insecure`.
Following the graceful-reload precedent, no compatibility shim is shipped.

| Old                               | New                                                         |
| --------------------------------- | ----------------------------------------------------------- |
| `--tofu` (alone)                  | `--tofu-insecure` (preferred) or `--tofu-fingerprint=<hex>` |
| `--tofu --tofu-fingerprint=<hex>` | `--tofu-fingerprint=<hex>`                                  |
