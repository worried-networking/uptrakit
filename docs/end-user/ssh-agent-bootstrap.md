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

## Target format

The bootstrap command accepts a single positional `TARGET` argument in standard
SSH address format:

- `[user@]host[:port]` — plain format (e.g. `root@192.168.1.100`,
  `myserver:2222`)
- `ssh://[user@]host[:port]` — SSH URL format (e.g.
  `ssh://root@192.168.1.100:22`)
- IPv6 bracket notation: `[::1]`, `user@[::1]:22`,
  `ssh://root@[::1]:2222`

Values extracted from the target string (username, port) take precedence. When
omitted from the target, the bootstrap command applies defaults in this order:

1. **Username**: `~/.ssh/config` `User` directive, then `$USER` environment
   variable
2. **Port**: `~/.ssh/config` `Port` directive, then `22`
3. **Hostname**: `~/.ssh/config` `HostName` directive (allows SSH aliases), then
   the hostname from the target string

The `--name` flag defaults to the target hostname (before `HostName`
resolution), so SSH aliases map naturally to host names.

## Usage

### Bootstrap with password prompt

```bash
uptrakit-agent-ssh host bootstrap root@192.168.1.100 \
  --auth-password \
  --master-key-file /etc/uptrakit/master.key
```

When `--auth-password` is passed without a value, you are prompted to enter the
password securely (no echo) at runtime. The host name defaults to
`192.168.1.100` (overridable with `--name`).

### Bootstrap with inline password

```bash
uptrakit-agent-ssh host bootstrap root@192.168.1.100 \
  --name my-server \
  --auth-password "my-secret-pass" \
  --master-key-file /etc/uptrakit/master.key
```

When `--auth-password` is followed by a value, that value is used directly. Note
that the password will be visible in your shell history and process listing. See
[SSH Agent Secrets](../security/ssh-agent-secrets.md) for security implications.

### Bootstrap with key authentication

```bash
uptrakit-agent-ssh host bootstrap admin@192.168.1.100:2222 \
  --auth-private-key-file ~/.ssh/id_ed25519 \
  --target-username uptrakit \
  --host-key-fingerprint "SHA256:abc123..." \
  --master-key-file /etc/uptrakit/master.key
```

### Bootstrap with SSH agent

```bash
uptrakit-agent-ssh host bootstrap admin@192.168.1.100 \
  --master-key-file /etc/uptrakit/master.key
```

When no `--auth-password` or `--auth-private-key-file` flag is given, the
bootstrap command automatically detects the local SSH agent via `SSH_AUTH_SOCK`
and tries each loaded key. No additional flags are needed.

### Bootstrap with SSH config alias

If `~/.ssh/config` contains:

```text
Host myserver
  User deploy
  HostName 10.0.0.5
  Port 2222
```

Then:

```bash
uptrakit-agent-ssh host bootstrap myserver \
  --auth-password \
  --master-key-file /etc/uptrakit/master.key
```

This resolves to `deploy@10.0.0.5:2222` with host name `myserver`.

### All arguments and flags

| Argument/Flag | Required | Default | Description |
| --- | --- | --- | --- |
| `TARGET` (positional) | Yes | — | SSH target: `[user@]host[:port]` or `ssh://[user@]host[:port]` |
| `--name` | No | target hostname | Friendly name for the host (must be unique) |
| `--auth-password` | No | — | Password auth: no value = prompt, with value = use directly |
| `--auth-private-key-file` | No | — | Path to PEM private key for authentication |
| `--target-username` | No | `uptrakit` (when auth is `root`), else auth username | Username for the managed account |
| `--target-private-key-file` | No | (generated) | Path to PEM private key for the target user |
| `--host-key-fingerprint` | No | (TOFU) | Expected host key fingerprint (SHA-256) |
| `--allow-all` | No | `false` | Write `NOPASSWD: ALL` instead of specific command entries (less secure) |
| `--remove-stale-keys` | No | `false` | Extend stale-key removal to all Uptrakit-managed keys (same-service keys are always removed automatically; see [Stale key detection](#stale-key-detection)) |

## SSH config resolution

The bootstrap command reads `~/.ssh/config` to fill in defaults for the target
host. The following directives are supported:

| Directive | Applies to |
| --- | --- |
| `User` | Auth username (when not specified in the target) |
| `Port` | SSH port (when not specified in the target) |
| `HostName` | Resolved hostname (allows SSH aliases like `myserver` → `10.0.0.5`) |

Resolution never fails due to a missing or malformed SSH config — defaults are
silently skipped.

## Root-aware target username default

When the auth username is `root` and `--target-username` is omitted, the
bootstrap command defaults `--target-username` to `uptrakit` instead of reusing
`root`. This ensures the managed account is a dedicated, unprivileged service
user rather than the root account.

A notice is printed when this default is applied:

```text
NOTE: auth username is 'root'; defaulting target username to 'uptrakit'.
```

When the auth username is any non-root user and `--target-username` is omitted,
the auth username is used as the target username (unchanged behavior).

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
   --shell /bin/sh`. The `/bin/sh` shell is used because the managed account
   only needs non-interactive command execution.

4. **Deploy SSH key** — If `--target-private-key-file` is omitted, generates a
   new Ed25519 keypair in memory. Reads the existing `authorized_keys` (if any),
   then applies two-tier stale-key handling (see
   [Stale key detection](#stale-key-detection) below):

   - **Automatic removal** — any existing entry written by this service on a
     previous bootstrap run (comment matching
     `uptrakit-svc:<service-uuid>-host:*`) is removed without any flag.
   - **Explicit removal** — pass `--remove-stale-keys` to additionally remove
     all other Uptrakit-managed entries.

   The new public key is then written to `~target/.ssh/authorized_keys` with SSH
   restrictions (`no-pty,no-agent-forwarding,no-X11-forwarding`) and proper
   permissions (`700` for `.ssh`, `600` for `authorized_keys`). The restrictions
   prevent interactive terminal allocation, SSH agent forwarding, and X11
   forwarding through the managed account while allowing the non-interactive
   command execution that the agent requires.

   The key entry is written with a comment that identifies its origin:

   - `uptrakit-svc:<service-uuid>-host:<host-uuid>` — when the service has
     already been enrolled with the controller.
   - `uptrakit-host:<host-uuid>` — when bootstrap runs before first enrollment
     (fallback; still marks the key as Uptrakit-managed).

5. **Configure sudoers** — Queries all registered plugins for their required
   sudo commands, resolves each to its absolute path on the remote host via
   `command -v`, and writes a minimal
   `/etc/sudoers.d/uptrakit-<target_username>` with one entry per resolved
   command. Validates with `visudo -cf`. Commands not found on the remote host
   are skipped with a warning. Pass `--allow-all` to fall back to
   `NOPASSWD: ALL` when no commands resolve.

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

## Stale key detection

Each key entry written to `authorized_keys` by bootstrap includes a comment
that identifies it as Uptrakit-managed:

- `uptrakit-svc:<service-uuid>-host:<host-uuid>` — when the service has been
  enrolled with the controller.
- `uptrakit-host:<host-uuid>` — when bootstrap runs before first enrollment
  (the service UUID is not yet known).

Before writing the new key, bootstrap reads `authorized_keys` and applies
**two-tier stale-key handling**:

### Tier 1 — automatic same-service removal (always)

Any existing entry whose comment matches
`uptrakit-svc:<this-service-uuid>-host:*` is removed **without any flag**.
This keeps `authorized_keys` clean when the same service re-bootstraps a
machine — for example, after key rotation or when a host was previously
bootstrapped under a different name. Keys placed by *other* Uptrakit services
and non-Uptrakit keys are left untouched.

Example output when one same-service key is found and removed:

```text
Removing 1 key(s) written by this service on a previous bootstrap...
  no-pty,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA... uptrakit-svc:550e8400-...-host:018d7f12-...
```

This automatic removal only occurs when the service has been enrolled (so its
UUID is known). If bootstrap runs before first enrollment, no automatic removal
takes place.

### Tier 2 — explicit broad removal (`--remove-stale-keys`)

Pass `--remove-stale-keys` to also remove all other Uptrakit-managed entries:

- **For the `uptrakit` target user**: all existing entries (the `uptrakit`
  account is exclusively managed by the agent; no external keys should be
  present).
- **For all other target users**: only entries whose last
  whitespace-separated token starts with `uptrakit`.

Entries already covered by the automatic same-service removal are not counted
twice.

If remaining stale keys are found but the flag is not set, bootstrap prints a
notice:

```text
NOTE: Found 1 existing key(s) in authorized_keys:
  no-pty,no-agent-forwarding,no-X11-forwarding ssh-ed25519 BBBB... uptrakit-svc:aaaaaaaa-...-host:bbbbbbbb-...
      Pass --remove-stale-keys to remove them before writing the new key.
```

By default those entries are left in place (non-destructive append). Pass
`--remove-stale-keys` when you want to remove all Uptrakit-managed keys on the
machine before deploying the new one — for example, when transferring a host to
a different service instance:

```bash
uptrakit-agent-ssh host bootstrap root@192.168.1.100 \
  --auth-password \
  --master-key-file /etc/uptrakit/master.key \
  --remove-stale-keys
```

## Sudoers configuration

The bootstrap command generates a minimal sudoers file with one entry per
registered plugin command. For example, if only the APT plugin is active:

```text
# Managed by Uptrakit - DO NOT EDIT MANUALLY
# Regenerate: uptrakit-agent-ssh host sync <host>
# /usr/bin/apt-get: Package installation and index refresh require root privileges
uptrakit ALL=(root) NOPASSWD: /usr/bin/apt-get
```

This is written to `/etc/sudoers.d/uptrakit-<target_username>` with `440`
permissions and validated with `visudo -cf`.

The `--allow-all` flag writes `NOPASSWD: ALL` instead (less secure; matches the
pre-1.x behavior). Use it only when the required tools are not yet installed or
during development.

To refresh the sudoers file (and PVE configuration) after adding new plugins, run:

```bash
uptrakit-agent-ssh host sync <host-name> \
  --master-key-file /etc/uptrakit/master.key
```

For the full security rationale, see [Sudoers Management](../security/sudoers-management.md).

## POSIX username requirements

Both the auth username (from the target string or SSH config) and
`--target-username` must be valid POSIX usernames:

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

### "could not determine SSH username"

No username was found in the target string, `~/.ssh/config`, or the `$USER`
environment variable. Specify the username in the target (e.g. `user@host`) or
set the `$USER` environment variable.

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

### "No plugin commands could be resolved on the remote host"

The required plugin tools (e.g. `apt-get`) are not installed on the remote
host, so no sudoers entries could be generated. Either install the required tools
first, or re-run bootstrap with `--allow-all` to fall back to `NOPASSWD: ALL`.

### "sudoers validation failed"

The generated sudoers file did not pass `visudo -cf` validation. This is
unexpected and may indicate a non-standard sudoers configuration on the remote
host. Check `/etc/sudoers` and `/etc/sudoers.d/` for syntax errors.

### Accumulating keys after repeated bootstraps

If bootstrap is run multiple times against the same host without
`--remove-stale-keys`, each run appends a new key entry. The old entries
remain valid but are unused. Re-run with `--remove-stale-keys` to clean up:

```bash
uptrakit-agent-ssh host bootstrap root@192.168.1.100 \
  --auth-password \
  --master-key-file /etc/uptrakit/master.key \
  --remove-stale-keys
```

The flag removes only the entries identified as Uptrakit-managed (comment
starts with `uptrakit`), leaving any manually added keys untouched — except
for the `uptrakit` user account, where **all** existing entries are removed.

### Partial failure

If the bootstrap fails after step 2 (remote setup has started), the error
message describes what was completed. The remote host may be partially
configured (user created, key deployed, sudoers written). You can either:

- Fix the issue and re-run bootstrap (the existing user will be detected and
  reused)
- Manually clean up the remote host and retry

The host entry is **not** saved to the database unless all steps succeed.

## PVE node detection

When bootstrapping a host via SSH, the agent automatically checks whether the
target is a Proxmox VE node (by looking for `pveversion`). If detected, the
agent checks whether Uptrakit has already been set up on the same PVE cluster:

- **No existing token** — creates a tenant-scoped PVE API user
  (`uptrakit-{tenant_id}@pve`) with `PVEAuditor` role (read-only access),
  marks the host as a PVE node, and reports the plugin configuration to the
  controller.
- **Token owned by the same tenant** — reuses the existing plugin
  configuration from a previously bootstrapped node in the same cluster. No
  duplicate credentials are created.
- **Token owned by a different tenant** — bootstrap fails with an error
  explaining that the cluster is already claimed by another tenant.
- **Tenant ID not yet available** — skips PVE credential creation with a
  warning. This can happen if the service has not received its settings from
  the controller yet.

This enables the **Bootstrap via Proxmox** extension action, which allows
bootstrapping LXC containers and QEMU VMs through the PVE node without direct
SSH access. See [Proxmox VE Integration](proxmox.md#bootstrapping-guests-via-ssh-agent)
for details.

If PVE API credential creation fails (e.g. insufficient permissions), a warning
is printed and the bootstrap continues normally. You can configure the Proxmox
plugin manually afterwards.

## Related documentation

- [SSH Agent Host Management](ssh-agent-host-management.md) — managing existing
  host entries, including `sync`
- [SSH Agent Architecture](../architecture/ssh-agent.md) — architecture and
  database schema
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — encryption model and
  threat model
- [Sudoers Management](../security/sudoers-management.md) — sudoers generation,
  security model, and operator guidance
