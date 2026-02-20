# Home Assistant and MQTT

- The MQTT service publishes Home Assistant discovery topics for each tracked software item.
- Users can trigger updates by calling `uptrakit.update` services from Home Assistant.
- MQTT credentials (username/password) are encrypted in the database (`EncryptedString`). See
  [docs/security/secrets-and-encryption.md](../security/secrets-and-encryption.md) for details.
- Multiple MQTT service instances can run; assignments are delivered via `tenant_assignments` messages.

## Custom CA Certificates for Private Brokers

When connecting to MQTT brokers that use private or internal certificate authorities (e.g.,
self-signed certificates or enterprise CAs), you can provide a custom CA certificate in PEM format.
This is configured per MQTT client in **Settings > MQTT Clients**:

1. Create or edit an MQTT client configuration.
1. Set the transport to **TLS**.
1. Paste the CA certificate PEM into the **CA Certificate (PEM)** field.
1. Save the configuration.

The CA certificate is encrypted at rest using AES-256-GCM (`EncryptedString`) and transmitted to
the MQTT service via the wire protocol (`MqttTenantConfig.ca_pem`). The MQTT service uses the
provided PEM bytes as the trusted CA for TLS connections instead of the system trust store.

If no custom CA is provided, the MQTT service falls back to the system trust store (default
behavior for public brokers).

The CLI supports this via `--ca-pem <PEM_STRING>` (inline) or `--ca-pem-file <PATH>` (from file)
on the `settings mqtt create` and `settings mqtt update` subcommands.
