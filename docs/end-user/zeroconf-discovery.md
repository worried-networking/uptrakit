---
title: Zero-Configuration Service Discovery
weight: 250
description: Uptrakit supports automatic controller discovery on the local network using mDNS/DNS-SD, allowing agents and services to find the controller without an explicit URL flag.
---

# Zero-Configuration Service Discovery

Uptrakit supports automatic controller discovery on the local network using mDNS/DNS-SD. When enabled, the
controller advertises itself as `_uptrakit._tcp.local.`, and services (agent, SSH agent, MQTT) find it
automatically without requiring an explicit `--url` flag. This simplifies initial setup on trusted LANs
where the controller and services run on the same network segment.

## Quick Start

1. **Enable zeroconf on the controller.** Either pass the `--zeroconf` flag at startup:

   ```sh
   uptrakit-controller --zeroconf
   ```

   Or enable it via the web UI: **Global Settings > Zero-Configuration Discovery > Enable**.

   The controller logs its CA fingerprint at startup:

   ```text
   INFO mDNS advertising enabled. CA fingerprint: SHA256:a1b2c3d4e5f6...
   INFO Verify this fingerprint on connecting services to prevent MITM.
   ```

2. **Start services without `--url`.** Services automatically browse for the controller:

   ```sh
   uptrakit-agent
   ```

   The service logs the discovered controller and its CA fingerprint:

   ```text
   INFO browsing for controller via mDNS (_uptrakit._tcp.local.)...
   WARN Discovered controller via mDNS at https://192.168.1.100:8443
   WARN CA fingerprint: SHA256:a1b2c3d4e5f6...
   WARN Verify this matches the controller's fingerprint to prevent MITM.
   WARN Use --tofu-fingerprint to enforce verification automatically.
   ```

3. **Verify the CA fingerprint.** Compare the fingerprint shown in the controller logs with the fingerprint
   shown in the service logs. They must match. If they do not match, an attacker may be impersonating the
   controller on the network. Stop the service and investigate.

## Strict Fingerprint Verification

For automated deployments or stricter security, pass the expected CA fingerprint to the service:

```sh
uptrakit-agent --tofu-fingerprint a1b2c3d4e5f6...
```

The service will abort immediately if the discovered CA fingerprint does not match the provided value. This
prevents any possibility of connecting to a spoofed controller.

You can also combine `--tofu-fingerprint` with `--url` to skip mDNS entirely while still using fingerprint
pinning:

```sh
uptrakit-agent --url https://controller.example.com:8443 --tofu-fingerprint a1b2c3d4e5f6...
```

## Reverse Proxy Setup

When the controller is behind a reverse proxy, the mDNS-resolved IP address and port may not be the correct
endpoint for services to connect to. Use the URL and PKI address overrides to advertise the correct values:

```sh
uptrakit-controller --zeroconf \
  --zeroconf-url https://proxy.example.com:443 \
  --zeroconf-pki-addr http://pki.internal:8080
```

These overrides can also be configured via the web UI (**Global Settings > Zero-Configuration Discovery**) or
the REST API:

```sh
curl -X PUT https://controller:8443/api/v1/global-settings/zeroconf \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://proxy.example.com:443", "pki_addr": "http://pki.internal:8080"}'
```

When a URL override is configured, services use that URL instead of constructing one from the mDNS-resolved
address. The CA fingerprint is still advertised and verified as normal.

Changes to zeroconf settings require a controller restart to take effect.

## Cache Behaviour

After the first successful discovery, the resolved URL and CA fingerprint are saved to `discovery.json` in the
service's state directory. On subsequent restarts, the service reuses the cached result without performing a new
mDNS browse. This provides two benefits:

- **Faster startup** -- no 10-second mDNS browse timeout on every restart.
- **Resilience** -- once the correct controller is cached, subsequent restarts are immune to mDNS spoofing.

To force a fresh discovery (for example, after moving the controller to a new address), clear the cache:

```sh
uptrakit-agent --clear-discovery-cache
```

This removes `discovery.json` and triggers a new mDNS browse on startup.

## When NOT to Use Zero-Configuration Discovery

Zero-configuration discovery relies on mDNS, which is LAN-scoped and unauthenticated. It is designed for
trusted network environments such as home labs and private infrastructure. Do not use it when:

- **The network is untrusted.** On shared or public networks, an attacker on the same LAN can spoof mDNS
  responses and impersonate the controller.
- **Services connect over WAN.** mDNS does not traverse routers. For WAN deployments, use `--url` with the
  controller's public address.
- **High-security environments.** Use explicit `--url` combined with `--ca-cert` (providing the CA certificate
  directly) to eliminate any trust-on-first-use window.

For these scenarios, always specify the controller URL explicitly:

```sh
uptrakit-agent --url https://controller.example.com:8443 --ca-cert /path/to/ca.pem
```

## Troubleshooting

**No controller found via mDNS discovery:**

- Verify that zeroconf is enabled on the controller (`--zeroconf` flag or web UI toggle).
- Ensure UDP port 5353 is open on both the controller and service hosts. mDNS uses multicast UDP on port 5353.
- Confirm both hosts are on the same network segment. mDNS does not traverse routers unless multicast routing
  is explicitly configured.
- On Linux, verify that the firewall allows multicast traffic:

  ```sh
  sudo iptables -L -n | grep 5353
  ```

**Service connects to the wrong controller:**

- Clear the discovery cache and re-discover:

  ```sh
  uptrakit-agent --clear-discovery-cache
  ```

- Use `--tofu-fingerprint` to pin the expected CA fingerprint and reject any mismatch.

**Zeroconf was working but stopped after a controller move:**

- The service is using the cached `discovery.json` with the old address. Clear the cache:

  ```sh
  uptrakit-agent --clear-discovery-cache
  ```

**CA fingerprint mismatch between controller and service logs:**

- This indicates a potential man-in-the-middle attack. Stop the service immediately and investigate. Do not
  proceed until the fingerprints match. See
  [Zero-Configuration Discovery Security](../security/zeroconf-discovery.md) for details.

## Related Documentation

- [Zero-Configuration Discovery (Development)](../development/zeroconf-discovery.md) -- architecture and internals
- [Zero-Configuration Discovery Security](../security/zeroconf-discovery.md) -- threat model and mitigations
- [TOFU and TLS Hardening](../security/tofu-tls.md) -- fingerprint pinning and TLS verification
