# Home Assistant and MQTT

- The MQTT service publishes Home Assistant discovery topics for each tracked software item.
- Users can trigger updates by calling `uptrakit.update` services from Home Assistant.
- MQTT credentials (username/password) are encrypted in the database (`EncryptedString`). See [docs/security/secrets-and-encryption.md](../security/secrets-and-encryption.md) for details.
- Multiple MQTT service instances can run; assignments are delivered via `tenant_assignments` messages.
