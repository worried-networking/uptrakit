# SSH Agent Bootstrap

The `uptrakit-agent-ssh host bootstrap` command automates the full setup of a
remote host: it connects over SSH, creates a target user, deploys an SSH key,
configures passwordless sudo, verifies connectivity, and saves the host entry to
the local database — all in one step.

## Prerequisites

- SSH access to the remote host as a user with **passwordless sudo** (or as
  `root`).
- Master encryption key configured (see
  [SSH Agent Secrets](../security/ssh-agent-secrets.md)).
- The remote host must have `useradd`, `getent`, `visudo`, and standard POSIX
  utilities.

## Authentication methods

The bootstrap command supports three authentication methods. They are resolved in
priority order:

1. **`--auth-password <VALUE>`** — use the provided password directly.
2. **`--auth-password`** (no value) — prompt for password via stdin (no echo).
3. **`--auth-private-key-file <PATH>`** — read a PEM private key file.
4. **SSH agent** (automatic fallback) — if neither `--auth-password` nor
   `--auth-private-key-file` is provided and the `SSH_AUTH_SOCK` environment
   variable is set, the bootstrap command connects to the local SSH agent,
   enumerates its loaded keys, and tries each one.

`--auth-password` and `--auth-private-key-file` are mutually exclusive. If
neither flag is provided and `SSH_AUTH_SOCK` is not set, the command fails with
an error listing all three options.

## Usage

### Bootstrap with password prompt

```bash
uptrakit-agent-ssh host bootstrap \
  --name my-server \
  --hostname 192.168.1.100 \
  --auth-username root \
  --auth-password \
  --master-key-file /etc/uptrakit/master.key
```

When `--auth-password` is passed without a value, you are prompted to enter the
password securely (no echo) at runtime.

### Bootstrap with inline password

```bash
uptrakit-agent-ssh host bootstrap \
  --name my-server \
  --hostname 192.168.1.100 \
  --auth-username root \
  --auth-password "my-secret-pass" \
  --master-key-file /etc/uptrakit/master.key
```

When `--auth-password` is followed by a value, that value is used directly. Note
that the password will be visible in your shell history and process listing. See
[SSH Agent Secrets](../security/ssh-agent-secrets.md) for security implications.

### Bootstrap with key authentication

```bash
uptrakit-agent-ssh host bootstrap \
  --name my-server \
  --hostname 192.168.1.100 \
  --auth-username admin \
  --auth-private-key-file ~/.ssh/id_ed25519 \
  --target-username uptrakit \
  --port 2222 \
  --host-key-fingerprint "SHA256:abc123..." \
  --master-key-file /etc/uptrakit/master.key
```

### Bootstrap with SSH agent

```bash
uptrakit-agent-ssh host bootstrap \
  --name my-server \
  --hostname 192.168.1.100 \
  --auth-username admin \
  --master-key-file /etc/uptrakit/master.key
```

When no `--auth-password` or `--auth-private-key-file` flag is given, the
bootstrap command automatically detects the local SSH agent via `SSH_AUTH_SOCK`
and tries each loaded key. No additional flags are needed.

### All flags

| Flag | Required | Default | Description |
| --- | --- | --- | --- |
| `--name` | Yes | — | Friendly name for the host (must be unique) |
| `--hostname` | Yes | — | SSH hostname or IP address |
| `--auth-username` | Yes | — | Username for the initial SSH connection |
| `--auth-password` | No | — | Password auth: no value = prompt, with value = use directly |
| `--auth-private-key-file` | No | — | Path to PEM private key for authentication |
| `--target-username` | No | `--auth-username` | Username for the managed account |
| `--target-private-key-file` | No | (generated) | Path to PEM private key for the target user |
| `--port` | No | 22 | SSH port |
| `--host-key-fingerprint` | No | (TOFU) | Expected host key fingerprint (SHA-256) |

## What happens on the remote host

The bootstrap command performs these steps in order:

1. **Connect and authenticate** — Connects to the remote host using the
   provided auth credentials (password, private key file, or SSH agent). If
   `--host-key-fingerprint` is given, the host key is verified strictly.
   Otherwise, TOFU (trust-on-first-use) is used and the observed fingerprint is
   displayed and stored.

2. **Detect privileges** — Checks whether the auth user is root (`id -u`). If
   not root, verifies that the auth user has passwordless sudo access.

3. **Create target user** (if different from auth user) — Checks whether the
   target user exists (`id -u`). If not, creates it with `useradd --create-home
   --shell /bin/bash`.

4. **Deploy SSH key** — If `--target-private-key-file` is omitted, generates a
   new Ed25519 keypair in memory. Appends the public key to
   `~target/.ssh/authorized_keys` with proper permissions (`700` for `.ssh`,
   `600` for `authorized_keys`).

5. **Configure sudoers** — Writes
   `/etc/sudoers.d/uptrakit-<target_username>` with `ALL=(ALL) NOPASSWD: ALL`
   and validates it with `visudo -cf`.

6. **Disconnect** the auth session.

7. **Verify** — Reconnects as the target user using the target key with strict
   host key pinning. Runs `whoami` and `sudo -n true` to confirm everything
   works.

8. **Save to database** — Encrypts the target private key and stores the host
   entry in the local SQLite database.

## Key generation

When `--target-private-key-file` is omitted, the bootstrap command generates an
Ed25519 keypair in memory. The private key is:

- Never written to disk as a file
- Encrypted with the master key (AES-256-GCM)
- Stored only in the local SQLite database

This means the private key exists only in the encrypted database. If the
database is lost, the key is lost. Back up the database or provide your own key
via `--target-private-key-file`.

## Sudoers configuration

The bootstrap command grants **full NOPASSWD access** to the target user:

```text
<target_username> ALL=(ALL) NOPASSWD: ALL
```

This is written to `/etc/sudoers.d/uptrakit-<target_username>` with `440`
permissions and validated with `visudo -cf`.

**Security warning**: `NOPASSWD: ALL` grants unrestricted root access to the
target user. After bootstrapping, review and restrict the sudoers file to only
the commands needed for your update workflow. See
[SSH Agent Secrets](../security/ssh-agent-secrets.md) for more details.

## POSIX username requirements

Both `--auth-username` and `--target-username` must be valid POSIX usernames:

- Start with a lowercase letter or underscore
- Contain only `a-z`, `0-9`, `_`, `-`
- Maximum 32 characters

## Host key verification

| Mode | Behavior |
| --- | --- |
| `--host-key-fingerprint` provided | Strict pinning — rejects mismatched keys |
| `--host-key-fingerprint` omitted | TOFU — accepts any key, displays and stores the fingerprint |

The verification step (step 7) always uses strict pinning with the fingerprint
observed during the initial connection. See
[SSH Agent Secrets](../security/ssh-agent-secrets.md) for the security
implications of TOFU vs pinned fingerprints.

## Troubleshooting

### "no authentication method available"

No `--auth-password` or `--auth-private-key-file` flag was provided, and
`SSH_AUTH_SOCK` is not set. Either pass an explicit auth flag or start your local
SSH agent (`eval $(ssh-agent)` / `ssh-add`).

### "SSH agent has no keys loaded"

The SSH agent is running but has no identities. Add a key with `ssh-add` or pass
`--auth-password` or `--auth-private-key-file` instead.

### "none of the N SSH agent key(s) were accepted"

The SSH agent has loaded keys but the remote host rejected all of them. Verify
that one of the agent's keys is authorized on the remote host for the given
username, or use a different auth method.

### "auth user does not have passwordless sudo access"

The auth user must be able to run `sudo -n true` without a password prompt.
Configure passwordless sudo for the auth user before bootstrapping, or use
`root` as the auth user.

### "could not determine home directory"

The target user's home directory could not be resolved via `getent passwd`. This
can happen if the user's account is misconfigured. Verify that `getent passwd
<username>` returns a valid entry on the remote host.

### "sudoers validation failed"

The generated sudoers file did not pass `visudo -cf` validation. This is
unexpected and may indicate a non-standard sudoers configuration on the remote
host. Check `/etc/sudoers` and `/etc/sudoers.d/` for syntax errors.

### Partial failure

If the bootstrap fails after step 2 (remote setup has started), the error
message describes what was completed. The remote host may be partially
configured (user created, key deployed, sudoers written). You can either:

- Fix the issue and re-run bootstrap (the existing user will be detected and
  reused)
- Manually clean up the remote host and retry

The host entry is **not** saved to the database unless all steps succeed.

## Related documentation

- [SSH Agent Host Management](ssh-agent-host-management.md) — managing existing
  host entries
- [SSH Agent Architecture](../architecture/ssh-agent.md) — architecture and
  database schema
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — encryption model and
  threat model
