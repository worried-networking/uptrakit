# Sudoers Management

This document describes how Uptrakit generates and manages the sudoers drop-in
file on remote SSH hosts, why per-command entries are preferred over
`NOPASSWD: ALL`, and how operators can control the behavior.

**Related docs:**

- [SSH Agent Bootstrap](../end-user/ssh-agent-bootstrap.md) — bootstrap workflow
- [SSH Agent Host Management](../end-user/ssh-agent-host-management.md) — `update-sudoers` command
- [SSH Agent Secrets](ssh-agent-secrets.md) — broader threat model
- [Plugin Guidelines](../development/plugin-guidelines.md) — `required_sudo_commands()` contract
- [Command Executor](../development/command-executor.md) — `SudoAwareCommandExecutor` and `SudoContext`

## Why per-command sudoers entries

The traditional approach of writing `NOPASSWD: ALL` grants the managed account
unrestricted root access. The Uptrakit SSH agent uses a minimal, per-command
approach instead:

```text
# Managed by Uptrakit - DO NOT EDIT MANUALLY
# Regenerate: uptrakit-agent-ssh host update-sudoers <host>
# /usr/bin/apt-get: Package installation and index refresh require root privileges
uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get
```

The `SETENV:` tag is included only for commands whose plugin sets
`SudoCommandEntry::needs_setenv = true` — those that invoke the command with
[`CommandSpec::with_env`] combined with `.privileged()`. It allows the agent to
pass inline `NAME=VALUE` env var assignments before the program name
(e.g. `sudo DEBIAN_FRONTEND=noninteractive apt-get update`). Without it, sudo
rejects those assignments. Importantly, `SETENV:` does **not** bypass
`env_reset` — sudo's built-in `env_delete` list still strips dangerous variables
such as `LD_PRELOAD` before they reach the privileged process.

**Advantages over `NOPASSWD: ALL`:**

- **Least privilege** — the managed account can only elevate for commands
  declared by registered plugins.
- **Auditability** — each entry carries a human-readable explanation comment.
- **Path pinning** — absolute paths prevent `PATH` manipulation attacks where an
  attacker replaces a command with a malicious one in an earlier `PATH` entry.
- **Regenerability** — the file is machine-managed; running `update-sudoers`
  keeps it current as plugins are added or removed.

## How entries are generated

Sudoers entries come from the `required_sudo_commands()` method on each
registered `Plugin`. Each entry carries a command name, a human-readable
explanation, and an optional `SudoHelperScript`.

During bootstrap or `update-sudoers`:

1. `PluginRegistry::compatible_sudo_commands_for_host(ssh_executor)` collects
   declarations from registered plugins that are **compatible with the target host**.
   - Plugins that declare the `DetectHostCompatibility` capability first run a
     compatibility check over SSH (e.g. the Proxmox Helper Scripts plugin checks
     for `/usr/bin/update`). Incompatible plugins are silently skipped.
   - Plugins without `DetectHostCompatibility` are always included (assumed
     compatible with all hosts).
2. For each entry from a compatible plugin:
   - **With `helper_script`**: the script is written to `install_path` on the remote host
     with mode `0755`, and its path is used directly as the sudoers command.
   - **Without `helper_script`**: `command -v <name>` is run on the remote host to resolve
     the absolute path. Commands not found are skipped with a warning (non-fatal).
3. Resolved entries become `<username> ALL=(root) NOPASSWD: <absolute-path>` lines, with `SETENV:` inserted
   only when `SudoCommandEntry::needs_setenv` is `true` for that entry.
4. The file is written with permissions `440` and validated with `visudo -cf`.

If no commands resolve (all plugin tools are missing or incompatible), the command fails unless `--allow-all` is passed.

### Host compatibility checks

Each plugin may declare the `DetectHostCompatibility` capability and implement
`detect_host_compatibility()` to test whether it makes sense on the target host.
The check runs over the same SSH session used for bootstrapping, so it reflects
the actual remote environment.

| Plugin | Compatibility check |
| --- | --- |
| Proxmox Helper Scripts | Tests for `/usr/bin/update` (PHS update script, Proxmox VE only) |
| APT | Tests for `apt-get` via `which` |
| Homebrew | Tests for `brew` via `which` |
| npm | Tests for `npm` via `which` |

Plugins that fail their compatibility check are **not** included in the sudoers
file and their helper scripts are **not** installed. This is important for hosts
with read-only filesystems (e.g. Flatcar Linux's `/usr/local/bin`) where helper
script installation would otherwise fail the entire bootstrap.

## Helper scripts

Some operations cannot be safely restricted using sudoers wildcards. In sudoers, `*`
matches `/`, so a pattern like `NOPASSWD: /usr/bin/cat /root/.*` would still allow
reading `/root/.ssh/id_rsa`. The `SudoHelperScript` mechanism solves this.

When a `SudoCommandEntry` includes a `helper_script`, bootstrap installs a small
shell script on the managed host. The sudoers entry covers only that specific script
path — no wildcards — and the script validates its own arguments before acting.

**Example: PHS version detection.** The Proxmox Helper Scripts plugin reads version
files from `/root/.<slug>`. Rather than granting `NOPASSWD: /usr/bin/cat` (which
allows reading any file), bootstrap installs `/usr/local/bin/uptrakit-phs-version`.
The script accepts a single slug argument, rejects anything outside `[a-z0-9][a-z0-9-]*`
(no `/`, no `.`, no special characters), then reads `/root/.<slug>`. The resulting
sudoers line is:

```text
uptrakit ALL=(root) NOPASSWD: /usr/local/bin/uptrakit-phs-version
```

This restricts root access to exactly one path pattern — PHS dot-files in `/root/` —
and argument injection is prevented by the script's `case` guard.

**Example: PHS updates.** Executing a PHS update requires root to run
`/usr/bin/update` with `PHS_SILENT=1` (suppresses interactive whiptail dialogs) and
`TERM=xterm` (ensures terminal commands succeed over a non-interactive SSH channel).
The agent embeds these as inline `NAME=VALUE` assignments in the `sudo` call:

```text
sudo PHS_SILENT=1 TERM=xterm /usr/bin/update
```

The `SETENV:` tag in the sudoers entry allows sudo to accept those assignments:

```text
uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/update
```

The Shell plugin embeds `sudo` directly in the `update_command` string because
`CommandSpec::shell()` does not support the `.privileged()` flag.

### Implementing a helper script in a plugin

Override `required_sudo_commands()` and supply a `SudoHelperScript`:

```rust
fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
    vec![SudoCommandEntry {
        command: "my-helper".into(),
        explanation: "Short description of why root access is needed".into(),
        helper_script: Some(SudoHelperScript {
            install_path: "/usr/local/bin/my-helper",
            content: include_str!("my_helper.sh"),
        }),
    }]
}
```

The helper script **must**:

- Validate every argument that influences a file path or command before use.
- Exit non-zero with a clear error message on invalid input.
- Be self-contained (no external dependencies beyond POSIX shell and `cat`/`printf`).

## The `--allow-all` fallback

Both `host bootstrap` and `host update-sudoers` accept `--allow-all`:

```bash
uptrakit-agent-ssh host update-sudoers my-server --allow-all
```

This writes `NOPASSWD: ALL` instead of per-command entries. Use only when:

- Required tools are not yet installed on the remote host.
- You are developing a new plugin and the command is not deployed yet.
- You have a specific operational reason to grant unrestricted access.

`--allow-all` is never set by default and must be explicitly passed each time.

## Sudo policy per host

Each enrolled host stores a `sudo_policy` that controls how the SSH agent
prepends `sudo` at runtime (when executing plugin commands over SSH):

| Policy | CLI value | Behavior |
| --- | --- | --- |
| `Auto` (default) | `auto` | Prepend `sudo` when agent user is not root **and** `sudo_available` is true in the database |
| `ForceWith` | `force-with` | Always prepend `sudo` (unless agent user is UID 0) |
| `ForceWithout` | `force-without` | Never prepend `sudo` |

Change the policy with:

```bash
uptrakit-agent-ssh host update my-server --sudo-policy force-with
```

The policy is stored in the `ssh_hosts.sudo_policy` column (`"auto"` by
default). At runtime, `Model::resolved_sudo_context()` builds a `SudoContext`
from the stored values. When `sudo_available` or `is_root` are `NULL` (unknown),
the defaults are:

- `sudo_available = true` — backward compatibility for hosts enrolled before
  sudo tracking was added.
- `is_root = false` — conservative; the agent user is assumed non-root until
  confirmed otherwise.

## Detecting and persisting sudo state

The `host update-sudoers` command always re-detects the agent user's privilege
context by running `id -u` (root check) and `sudo -n true` (passwordless sudo
check) on the remote host, and persists the results to the database.

The `host bootstrap` command also sets `is_root` and `sudo_available` in the
database, but uses the detected auth-user context (not the target user's).

The `SudoAwareCommandExecutor` (used in `handle_check_versions_ssh`,
`handle_execute_update_ssh`, `handle_discover_software_ssh`) reads these stored
values at message-dispatch time — it **never** re-detects at runtime, which
avoids extra SSH round-trips during normal operations.

## Regenerating after plugin changes

When new plugins are added or existing plugins add new commands, the sudoers
file becomes stale. Refresh it by running:

```bash
uptrakit-agent-ssh host update-sudoers my-server
```

This resolves current plugin commands, writes the updated file, and persists
the detected sudo state. Use `--dry-run` to preview the file without writing:

```bash
uptrakit-agent-ssh host update-sudoers my-server --dry-run
```

## Security recommendations

- **Use per-command entries** (the default). Avoid `--allow-all` in production.
- **Use `--strict-host-key-checking`** during bootstrap to prevent MITM when writing the sudoers file.
- **Restrict the master encryption key** (`400` permissions, service account ownership) to protect SSH credentials at rest.
- **Review the sudoers file** after bootstrap: `ssh user@host sudo cat /etc/sudoers.d/uptrakit-user`.
- **Run `update-sudoers`** after adding or removing plugins to keep the file minimal and current.
- **Avoid `--allow-all` for `update-sudoers`** unless the remote host is missing required tools during initial provisioning.

## File format reference

```text
# Managed by Uptrakit - DO NOT EDIT MANUALLY
# Regenerate: uptrakit-agent-ssh host update-sudoers <host>
# <absolute-path>: <explanation>
<username> ALL=(root) NOPASSWD: [SETENV: ]<absolute-path>
```

`SETENV:` is included only when the plugin sets `needs_setenv = true` (currently only `apt-get`).

Or with `--allow-all`:

```text
# Managed by Uptrakit - DO NOT EDIT MANUALLY
# Regenerate: uptrakit-agent-ssh host update-sudoers <host>
<username> ALL=(root) NOPASSWD: ALL
```

> **Note for existing hosts:** If your sudoers file was generated by a version of
> Uptrakit before `SETENV:` was added, privileged commands that set environment
> variables (such as `apt-get update` and `apt-get install`, which set
> `DEBIAN_FRONTEND=noninteractive`) will fail with "a password is required".
> Regenerate the sudoers file to fix this:
>
> ```bash
> uptrakit-agent-ssh host update-sudoers <host>
> ```

The file is written to `/etc/sudoers.d/uptrakit-<username>` with `440`
permissions. The path is deterministic and idempotent — re-running
`update-sudoers` overwrites the same file.
