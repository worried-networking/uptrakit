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

## Removed: bare `--tofu`

The historical bare `--tofu` flag is removed in this release. Operators using it must
choose explicitly: pin via fingerprint or SPKI, or accept any chain via `--tofu-insecure`.
Following the graceful-reload precedent, no compatibility shim is shipped.

| Old                               | New                                                         |
| --------------------------------- | ----------------------------------------------------------- |
| `--tofu` (alone)                  | `--tofu-insecure` (preferred) or `--tofu-fingerprint=<hex>` |
| `--tofu --tofu-fingerprint=<hex>` | `--tofu-fingerprint=<hex>`                                  |
