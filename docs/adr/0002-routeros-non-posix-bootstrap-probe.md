# RouterOS Bootstrap: Non-POSIX Detection and Executor Architecture

## Probe-then-route detection during bootstrap

When Agent-SSH bootstraps a new host it connects over SSH, but the operator does
not tell uptrakit which OS family the host is running. We need to detect RouterOS
hosts transparently, without adding friction to the host-registration workflow.

After establishing the SSH connection, the bootstrap sequence issues `/system
resource print` with a 5-second timeout. A successful execution whose output
contains `"platform:"` or `"MikroTik"` routes the session to the RouterOS path.
Anything else — a command error, timeout, or output matching neither string —
continues on the standard POSIX Linux path.

Two alternatives were considered. Having the operator declare an OS field at
host-add time shifts a decision the system can make itself onto a human, adds a
UI field that most operators will never need, and introduces a mismatch risk when
the field is wrong. Passive detection from the SSH banner is unreliable: many
MikroTik devices emit no banner, and generic banners (e.g. `"SSH-2.0-OpenSSH"`)
give no signal. The active probe is idempotent, completes in under 5 seconds, and
is authoritative — the RouterOS command either exists and produces recognisable
output or it does not. There is a theoretical TOCTOU window (the OS could be
reinstalled between bootstrap and a later operation), but this is accepted as
negligible for managed infrastructure where OS changes require explicit operator
action.

## `RouterOsExecutor` trait location

The RouterOS plugin crate (`uptrakit-package-manager-routeros`) must call methods
on the SSH executor — check for available packages, install updates — but it
cannot depend on `agent-ssh` directly. That dependency direction would create a
cycle: `plugin` → `agent-ssh` → `plugin-infrastructure-core` → ... → `plugin`.
The trait must be visible to both `agent-ssh` (which provides the concrete
implementation) and the plugin (which consumes it).

`RouterOsExecutor` (together with `RouterOsHostRuntime`) is defined in
`plugin-infrastructure-core`, the shared crate already on the dependency path of
every plugin. `agent-ssh` provides `RouterOsSshExecutor` as the concrete
implementation, depending on `plugin-infrastructure-core` in the normal direction.

The placement follows the same principle as every other cross-boundary trait in
the project: `HostRuntime`, `CommandExecutor`, and `ControllerRuntime` all live in
`plugin-infrastructure-core` precisely because they must be visible on both sides
of the plugin/service boundary. Creating a new `shared-ssh-types` crate would
achieve the same result at the cost of an extra crate with no other occupants.
Duplicating the trait in each consumer would solve the dep cycle but violate DRY
and diverge silently over time. Placing it in `plugin-infrastructure-core` is the
minimal change that follows the established pattern.

## Plugin access to RouterOS-specific capabilities via downcast

The plugin system's standard constructor signature is `new(config, runtime:
Arc<dyn HostRuntime>)`. RouterOS plugins need access to `Arc<dyn RouterOsExecutor>`
and the `allow_reboot` flag, neither of which belongs on the base `HostRuntime`
trait — adding them would pollute the interface for all POSIX plugins that never
interact with RouterOS.

A concrete `RouterOsHostRuntime` struct implements `HostRuntime` and carries the
RouterOS-specific fields as public members. Plugins that require them downcast via
`runtime.as_any().downcast_ref::<RouterOsHostRuntime>()`. If the downcast fails
the plugin returns `RouterOsError::SshExec` with a clear diagnostic message,
producing a fast, visible failure at instantiation time rather than a later panic
or silent no-op.

This is the same pattern already used for `ControllerRuntime` on the
controller-side plugin path. It extends runtime capabilities without growing the
base trait, keeps the extension invisible to plugins that do not need it, and
requires no changes to the plugin system's dispatch or registry logic.
