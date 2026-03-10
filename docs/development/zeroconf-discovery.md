# Zero-Configuration Service Discovery (Development Guide)

This document covers the mDNS/DNS-SD zero-configuration discovery feature from a development perspective. For
end-user setup instructions, see [Zero-Configuration Discovery (End-User)](../end-user/zeroconf-discovery.md).
For the security model and threat analysis, see [Zero-Configuration Discovery Security](../security/zeroconf-discovery.md).

## Architecture

Zero-configuration discovery allows Uptrakit services (agent, SSH agent, MQTT) to find the controller on the
local network without an explicit `--url` flag. The controller advertises its presence via mDNS/DNS-SD (RFC 6762 /
RFC 6763), and services browse for the advertisement automatically when `--url` is omitted.

The feature is opt-in on the controller side: it must be enabled via the `--zeroconf` CLI flag or the
`zeroconf.enabled` database setting (toggled via web UI or API). On the service side, discovery runs
automatically when no `--url` is provided and the `zeroconf` feature is compiled in.

### mDNS service type

The controller registers (and services browse for) the service type:

```text
_uptrakit._tcp.local.
```

The instance name is derived from the controller's hostname.

### TXT record format

The mDNS advertisement includes TXT record properties:

| Key | Required | Description |
| --- | :---: | --- |
| `ca_fp` | Yes | SHA-256 fingerprint of the active CA certificate, for TOFU verification |
| `url` | No | Override URL for reverse proxy deployments (services connect to this instead of the mDNS-resolved address) |
| `pki_addr` | No | PKI endpoint address override |

When `url` is absent, services construct the controller URL from the mDNS-resolved IP address and port.

### Discovery flow

1. Service starts without `--url`.
2. Service checks for a cached result in `<state_dir>/discovery.json`.
3. If no cache exists, service browses for `_uptrakit._tcp.local.` via mDNS (10-second timeout).
4. On resolution, the service extracts the URL (from `url` TXT property or IP:port), `pki_addr`, and `ca_fp`.
5. The result is saved to `discovery.json` with `0o600` permissions.
6. If `ca_fp` is present, TOFU mode is implicitly enabled with the discovered fingerprint as the pinned value.
7. The service proceeds with CA bootstrap and enrollment using the resolved URL.

If no controller is found within the timeout, the service exits with an error instructing the user to provide
`--url` explicitly.

## Crate and dependency

The feature uses the `mdns-sd` crate (pure Rust, async-compatible). Both the controller advertiser and the
service browser depend on it.

| Crate | Dependency |
| --- | --- |
| `uptrakit-controller` | `mdns-sd` (gated on `zeroconf` feature) |
| `uptrakit-service-sdk` | `mdns-sd` (gated on `zeroconf` feature) |

## Key files

```text
crates/core/controller/src/zeroconf.rs           # mDNS advertiser (register, TXT records, shutdown)
crates/shared/service-sdk/src/discovery.rs        # mDNS browser, cache load/save/clear, DiscoveryResult
crates/shared/service-sdk/src/lifecycle.rs         # resolve_connection() integrates discovery into lifecycle
crates/ui/web-api/src/routes/settings_zeroconf.rs # GET/PUT /api/v1/global-settings/zeroconf handlers
crates/shared/web-api-types/src/settings_zeroconf.rs # ZeroconfSettingsResponse, UpdateZeroconfSettingsRequest
```

## Feature flag

The `zeroconf` Cargo feature is **default-enabled** on both `uptrakit-service-sdk` and `uptrakit-controller`.

- **`uptrakit-service-sdk`**: `zeroconf = ["dep:mdns-sd"]` -- enables the `discovery` module and mDNS browse
  code paths in `resolve_connection()`.
- **`uptrakit-controller`**: `zeroconf = ["dep:mdns-sd", "dep:hostname"]` -- enables the `zeroconf` module
  and mDNS advertiser startup.

When the `zeroconf` feature is disabled at compile time, `--url` becomes mandatory for services. The controller
compiles without mDNS support and the `--zeroconf` CLI flag is unavailable. All zeroconf-related code is compiled
out via `#[cfg(feature = "zeroconf")]`.

## Configuration reference

### Controller

The controller must explicitly opt in to mDNS advertising. There are two ways to enable it:

**CLI flags:**

| Flag | Description |
| --- | --- |
| `--zeroconf` | Enable mDNS advertising at startup |
| `--zeroconf-url <URL>` | Override URL advertised in the `url` TXT property (requires `--zeroconf`) |
| `--zeroconf-pki-addr <URL>` | Override PKI address in the `pki_addr` TXT property (requires `--zeroconf`) |

**Web UI / API:**

Toggle via **Global Settings > Zero-Configuration Discovery** in the web UI, or via the REST API:

- `GET /api/v1/global-settings/zeroconf` -- returns `ZeroconfSettingsResponse`
- `PUT /api/v1/global-settings/zeroconf` -- accepts `UpdateZeroconfSettingsRequest`

Both endpoints require the `manage_global_settings` permission.

Changes via the API update the database and in-memory snapshot but do **not** hot-reload the mDNS advertiser.
The controller must be restarted for changes to take effect.

### Services

| Flag | Description |
| --- | --- |
| (no `--url`) | Triggers automatic mDNS discovery |
| `--url <URL>` | Disables discovery; connects directly to the specified URL |
| `--clear-discovery-cache` | Removes `discovery.json` before discovery, forcing a fresh mDNS browse |
| `--tofu-fingerprint <HEX>` | Overrides the discovered CA fingerprint with a strict pinned value |

## Database settings

Zeroconf settings are stored in the `global_settings` table under the following keys:

| Key | Type | Description |
| --- | --- | --- |
| `zeroconf.enabled` | boolean | Whether mDNS advertising is enabled |
| `zeroconf.url` | string / null | Override URL for reverse proxy deployments |
| `zeroconf.pki_addr` | string / null | Override PKI endpoint address |

These are reconciled at startup using the standard `reconcile_setting()` pattern (same as NATS and SMTP
settings). CLI flags take precedence on the first run; subsequent runs use the database value. See
[Settings Runtime](../api/settings-runtime.md) for the full reconciliation logic.

The in-memory representation is `ZeroconfSnapshot` (in `crates/ui/web-api/src/settings.rs`):

```rust
pub struct ZeroconfSnapshot {
    pub enabled: bool,
    pub url: Option<String>,
    pub pki_addr: Option<String>,
}
```

## Discovery cache

The cache is stored at `<state_dir>/discovery.json` and is written with `0o600` permissions via
`uptrakit_directories::write_secure_file_str()`. The format is:

```json
{
  "url": "https://192.168.1.100:8443",
  "pki_addr": "http://192.168.1.100:8080",
  "ca_fingerprint": "abcd1234..."
}
```

The `pki_addr` and `ca_fingerprint` fields are optional (added after the initial implementation) and use
`#[serde(default, skip_serializing_if = "Option::is_none")]` for backward compatibility with older cache files.

Once cached, subsequent service restarts reuse the cached URL without mDNS browsing. This provides resilience
against mDNS spoofing after the initial discovery. Use `--clear-discovery-cache` to force a fresh discovery.

## Security model

Discovery uses a trust-on-first-use (TOFU) model with CA fingerprint verification. The controller logs its CA
fingerprint at startup, and the service logs the discovered fingerprint. The user compares these out-of-band to
verify authenticity.

When the discovered TXT record includes `ca_fp`, the service implicitly enables TOFU mode with the discovered
fingerprint as the pinned value. This means `bootstrap_ca()` will verify the downloaded CA certificate against
the discovered fingerprint before accepting it.

For strict verification, use `--tofu-fingerprint <hex>` on the service side. This overrides the discovered
fingerprint and causes a fail-fast abort on mismatch.

See [Zero-Configuration Discovery Security](../security/zeroconf-discovery.md) for the full threat model and
mitigation details.

## Testing

### Unit tests

The `zeroconf.rs` and `discovery.rs` modules include unit tests for TXT record building, cache serialization,
URL construction from mDNS data, and edge cases (IPv6, loopback filtering, backward-compatible deserialization):

```bash
cargo test -p uptrakit-controller zeroconf
cargo test -p uptrakit-service-sdk discovery
```

The `settings_zeroconf.rs` types have validation tests:

```bash
cargo test -p uptrakit-web-api-types settings_zeroconf
```

### Integration tests

Full mDNS browse/advertise requires multicast networking (port 5353/UDP), which is unavailable in CI containers.
Integration tests that exercise the complete discovery flow are marked `#[ignore]` and must be run on a host with
multicast support:

```bash
cargo test -p uptrakit-service-sdk discovery -- --ignored
```

## Related documentation

- [Service Lifecycle](service-lifecycle.md) -- lifecycle integration and `resolve_connection()`
- [TOFU and TLS Hardening](../security/tofu-tls.md) -- `TofuVerifier` and fingerprint pinning
- [Coding Standards](coding-standards.md) -- feature flag conventions and `#[cfg]` patterns
- [Zero-Configuration Discovery (End-User)](../end-user/zeroconf-discovery.md) -- setup instructions
- [Zero-Configuration Discovery Security](../security/zeroconf-discovery.md) -- threat model and mitigations
