# Master Key Rotation

This guide describes how to rotate the master encryption key (KEK) used by the
controller and SSH agent. With envelope encryption, rotation re-wraps data
encryption keys (DEKs) without touching encrypted data — an O(1) operation
regardless of data volume.

## Prerequisites

- All controllers and SSH agents must be running code that supports envelope
  encryption (`ENC:v3:` format).
- Generate a new 32-byte (64 hex character) master key:

  ```bash
  openssl rand -hex 32 > /path/to/new-master.key
  chmod 0600 /path/to/new-master.key
  ```

## Controller Rotation

### Single-controller deployment

```bash
# 1. Stop the controller.
systemctl stop uptrakit-controller

# 2. Restart with both old and new keys.
uptrakit-controller \
  --master-key-file /path/to/old-master.key \
  --rotate-master-key-file /path/to/new-master.key \
  serve

# 3. Observe the log output:
#    INFO starting master key rotation
#    INFO master key rotation complete — restart with the new key file

# 4. Stop the controller again.
systemctl stop uptrakit-controller

# 5. Update configuration to use only the new key.
#    Set --master-key-file or UPTRAKIT_MASTER_KEY to the new key.

# 6. Restart.
systemctl start uptrakit-controller
```

### Multi-controller HA deployment

In an HA deployment, only one controller performs the rotation. The others
continue operating with DEKs already in memory.

```text
┌──────────────────────────────────────────────────────────────────┐
│ 1. Pick one controller (the "rotator").                         │
│ 2. Restart the rotator with:                                    │
│    --master-key-file old.key --rotate-master-key-file new.key   │
│ 3. The rotator:                                                 │
│    a. Unwraps all DEKs with the old KEK                         │
│    b. Re-wraps all DEKs with the new KEK (in a transaction)     │
│    c. Updates the master-key verification token                 │
│    d. Logs "master key rotation complete"                       │
│ 4. Other controllers continue running (DEKs in memory are       │
│    unchanged, data operations are unaffected).                  │
│ 5. Rolling restart: update each controller's config to use      │
│    --master-key-file new.key and restart one at a time.         │
│ 6. Each restarting controller unwraps DEKs with the new KEK     │
│    and operates normally.                                       │
└──────────────────────────────────────────────────────────────────┘
```

**Important**: Do not restart all controllers simultaneously. Rolling restarts
ensure continuous availability.

## SSH Agent Rotation

The SSH agent has its own independent master key. Rotation follows the same
pattern:

```bash
uptrakit-agent-ssh \
  --master-key-file /path/to/old-ssh.key \
  --rotate-master-key-file /path/to/new-ssh.key \
  --url wss://controller.example.com:8443
```

After the rotation log message, restart the agent with only the new key.

## What happens during rotation

The `--rotate-master-key-file` flag triggers the following sequence:

1. The current master key (`--master-key-file` / `UPTRAKIT_MASTER_KEY`) is used
   to unwrap all DEKs from the `data_encryption_keys` table.
2. A database transaction begins.
3. Each DEK is re-wrapped using the new KEK (from the rotation file).
4. The `wrapped_key` and `kek_fingerprint` columns are updated for each DEK.
5. A new master-key verification token is created with the new KEK.
6. The transaction is committed.

**No data re-encryption occurs.** Encrypted data values remain unchanged because
they are encrypted with DEKs, not the KEK. Only the DEK wrappers change.

## Verification

After rotation and restart with the new key, verify:

```bash
# Check the controller starts without errors
journalctl -u uptrakit-controller --since "5 minutes ago" | grep -i "key"

# Expected log lines:
# INFO master encryption key initialized
# INFO data key ring initialized active_key_id=... count=...
```

If the controller fails to start with a `MasterKeyMismatch` error, the new key
does not match the wrapped DEKs. Restart with the old key to recover.

## Rollback

If rotation needs to be reversed before restarting other controllers:

1. Stop the rotator controller.
2. Repeat the rotation in reverse (new key as current, old key as rotation target).
3. Restart all controllers with the original key.

Alternatively, restore the `data_encryption_keys` table from a database backup
taken before the rotation.

## Security considerations

- **Old key file**: After all controllers and agents have been restarted with the
  new key, securely delete the old key file (`shred -u old-master.key`).
- **Environment variable**: If using `UPTRAKIT_MASTER_KEY`, update the value in
  all deployment configurations before restarting services.
- **External scheduler**: The scheduler receives the master key from the
  controller via `ServiceCredentials`. After the controller restarts with the new
  key, the scheduler automatically receives the new key on its next connection.
  No manual scheduler restart is required (unless the scheduler is already
  running — in that case, restart it so it reconnects).

## Related documentation

- [Secrets and Encryption](secrets-and-encryption.md) — encryption model and
  master key management
- [SSH Agent Secrets](ssh-agent-secrets.md) — SSH agent encryption model
- [Cryptography](cryptography.md) — cryptographic primitives
- [External Scheduler Deployment](../end-user/deployment/external-scheduler.md) —
  scheduler credential flow
