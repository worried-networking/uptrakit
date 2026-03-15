# Command Executor

The `CommandExecutor` trait decouples plugin logic from the command execution transport. Plugins build a
`CommandSpec` describing *what* to run, and the injected executor decides *how* to run it (locally, over SSH, etc.).

**Related docs:**

- [Plugin Guidelines](plugin-guidelines.md) -- plugin architecture and construction
- [Coding Standards](coding-standards.md) -- error handling conventions used throughout
- [SSH Agent Architecture](../architecture/ssh-agent.md) -- future SSH executor use case
- [Secure Development](../security/secure-development.md) -- shell injection prevention

## Key types

Core data types (`CommandSpec`, `CommandMode`, `CommandOutput`, `InteractiveHandle`) live in
`crates/shared/command/src/types.rs`. The `CommandExecutor` trait and its implementations live in
`crates/shared/command/src/executor.rs`. All are re-exported from `uptrakit_command` and
`uptrakit_plugin_infrastructure_core::command`.

### `CommandSpec`

Describes a command to execute without specifying the transport.

```rust
pub struct CommandSpec {
    pub mode: CommandMode,
    pub working_dir: Option<String>,
    pub timeout: Option<std::time::Duration>,
    pub privileged: bool,
}
```

Convenience constructors:

| Constructor | Description |
| --- | --- |
| `CommandSpec::exec(program, args)` | Direct program execution (no shell). |
| `CommandSpec::shell(command)` | Shell command via Bash with fail-early settings (`set -euo pipefail`). |
| `CommandSpec::shell_with(command, shell)` | Shell command with a specific shell (`Bash`, `Sh`). |
| `.with_working_dir(dir)` | Builder method to set the working directory. |
| `.privileged()` | Mark the command as requiring elevated privileges (used with `SudoAwareCommandExecutor`). |

#### The `privileged` flag

Set `privileged: true` on a `CommandSpec` when the command requires root or elevated privileges to
execute (for example, running `apt-get install`). Do **not** hardcode `sudo` in the command program
or arguments — use `.privileged()` instead:

```rust
// ✓ Correct: declare privilege intent, let the executor handle sudo
CommandSpec::exec("apt-get", ["install", "-y", "package"]).privileged()

// ✗ Wrong: hardcoding sudo prevents executor-level control
CommandSpec::exec("sudo", ["apt-get", "install", "-y", "package"])
```

The `.privileged()` flag has no effect on `Shell` mode specs — shell commands control their own
privilege escalation. If `.privileged()` is set on a `Shell` spec, a warning is logged and the spec
is passed through unchanged.

### `CommandMode`

```rust
pub enum CommandMode {
    Exec { program: String, args: Vec<String> },
    Shell { command: String, shell: HookShell },
}
```

- **Exec**: runs the program directly without a shell (arguments are not interpreted).
- **Shell**: wraps the command in fail-early settings and invokes through the selected shell.

### `CommandOutput`

```rust
pub struct CommandOutput {
    pub output: String,
    pub exit_code: i32,
}
```

### `CommandExecutor` trait

```rust
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<CommandOutput>;

    async fn execute_quiet(&self, spec: &CommandSpec) -> Result<CommandOutput>;
}
```

- `execute()` streams output lines through `output_tx` for real-time progress reporting.
- `execute_quiet()` accumulates output silently (no streaming channel needed).

Both return `Result<CommandOutput>` -- errors are raised when the process fails to spawn or exits with a non-zero code.

When compiled with the `interactive` feature, the trait gains two additional methods:

- `execute_interactive()` -- allocates a PTY and returns an `InteractiveHandle` for bidirectional I/O.
- `supports_interactive()` -- returns `true` if the executor supports interactive execution.

See [Interactive Updates Development](interactive-updates.md) for details on the `InteractiveHandle`
type and the PTY implementation.

### `LocalCommandExecutor`

The default implementation. Delegates to `tokio::process::Command` on the local machine.

```rust
pub struct LocalCommandExecutor;
```

No configuration needed — instantiate with `LocalCommandExecutor` and wrap in `Arc`:

```rust
let executor: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
```

### `SudoAwareCommandExecutor`

A decorator executor that conditionally prepends `sudo` for `privileged` specs based on the runtime host context.

```rust
pub struct SudoAwareCommandExecutor {
    inner: Arc<dyn CommandExecutor>,
    context: SudoContext,
}
```

#### `SudoContext`

Controls whether and how `sudo` is prepended:

```rust
pub struct SudoContext {
    pub is_root: bool,          // agent user is UID 0
    pub sudo_available: bool,   // passwordless sudo is available
    pub policy: SudoPolicy,
}
```

`SudoPolicy` (default: `Auto`) determines the behavior:

| Variant | String | Behavior |
| --- | --- | --- |
| `Auto` | `"auto"` | Prepend `sudo` when not root AND `sudo_available` is `true` |
| `ForceWith` | `"force_with"` | Always prepend `sudo` (unless root) |
| `ForceWithout` | `"force_without"` | Never prepend `sudo` |

`SudoContext::default()` encodes the backward-compatible assumption used by the local agent:
non-root user, sudo available, auto policy. This matches the old hardcoded-sudo behavior.

#### How it works

`SudoAwareCommandExecutor::apply_sudo()` inspects each spec before delegating to the inner executor:

1. If `!spec.privileged` or `!context.should_use_sudo()` — passes the spec through unchanged.
2. If `spec.privileged && context.should_use_sudo()` and mode is `Exec { program, args }`:
   - Rewrites to `Exec { program: "sudo", args: [VAR=val…, old_program] + old_args }`.
   - Any `envs` on the spec are forwarded as inline `NAME=VALUE` assignments **before** the program name
     (e.g. `sudo DEBIAN_FRONTEND=noninteractive apt-get update -q`). Sudo parses these natively when the
     sudoers entry carries `SETENV:`, which all Uptrakit-generated entries include. The `envs` field is
     cleared on the resulting spec to avoid double-setting.
   - Using `env VAR=val PROG` is intentionally avoided: it would cause sudo to authorise `/usr/bin/env`
     rather than the actual program, breaking the `NOPASSWD: SETENV: /path/to/PROG` sudoers match.
3. If mode is `Shell { .. }` — emits `tracing::warn!` and passes through unchanged (shell commands handle their own privilege).

#### Usage

```rust
use std::sync::Arc;
use uptrakit_command::{
    CommandExecutor, LocalCommandExecutor,
    SudoAwareCommandExecutor, SudoContext,
};

// Local agent: use default context (backward-compatible sudo behavior)
let raw: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
let executor: Arc<dyn CommandExecutor> =
    Arc::new(SudoAwareCommandExecutor::new(raw, SudoContext::default()));

// SSH agent: load context from database
let executor: Arc<dyn CommandExecutor> =
    Arc::new(SudoAwareCommandExecutor::new(raw, host.resolved_sudo_context()));
```

The SSH agent stores `is_root`, `sudo_available`, and `sudo_policy` in the `ssh_hosts` table and
reads them back via `Model::resolved_sudo_context()`. When the values are unknown (`NULL`) in the
database, `resolved_sudo_context()` defaults to `sudo_available = true` for backward compatibility
with hosts bootstrapped before the sudo tracking migration.

See [SSH Agent Architecture — Sudo Context](../architecture/ssh-agent.md#sudo-context-and-dynamic-execution) for the full SSH agent integration.
See [Sudoers Management](../security/sudoers-management.md) for the security model and operator guidance.

## Plugin construction

Every plugin struct stores `Arc<dyn CommandExecutor>` and receives it via the constructor:

```rust
pub struct MyPlugin {
    config: MyConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl MyPlugin {
    pub fn new(config: MyConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        Self { config, executor }
    }
}
```

The `PluginRegistry::create_plugin()` method accepts the executor and forwards it to each plugin:

```rust
let plugin = PluginRegistry::create_plugin(
    PluginType::ReleasesGithub,
    &config,
    executor,
)?;
```

## Using the executor in plugins

### Streaming execution (e.g., `execute_update`)

```rust
let cmd_output = self
    .executor
    .execute(&CommandSpec::shell(&install_cmd), output_tx)
    .await
    .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
```

### Quiet execution (e.g., `detect_installed_version`)

```rust
let cmd_output = self
    .executor
    .execute_quiet(&CommandSpec::exec("brew", ["info".to_string(), "--json=v2".to_string(), name.to_string()]))
    .await?;
```

### Direct exec vs shell

Use `CommandSpec::exec()` when running a known program with fixed arguments (avoids shell
interpretation). Use `CommandSpec::shell()` when the command requires shell features (pipes, variable
expansion, compound statements).

## `SshCommandExecutor`

The SSH-backed implementation lives in `crates/core/agent-ssh/src/ssh_executor.rs`. It runs commands
on remote hosts via an `SshSession` from the SSH transport layer.

In practice, `SshCommandExecutor` is always wrapped with `SudoAwareCommandExecutor` in the SSH agent's handler functions — never used bare:

```rust
use std::sync::Arc;
use uptrakit_command::{CommandExecutor, SudoAwareCommandExecutor};

// Wrap an authenticated SSH session in Arc and pass it to the executor.
let session = Arc::new(session);
let raw: Arc<dyn CommandExecutor> = Arc::new(SshCommandExecutor::new(Arc::clone(&session)));
// Wrap with sudo-aware behavior from the database-backed context.
let executor: Arc<dyn CommandExecutor> =
    Arc::new(SudoAwareCommandExecutor::new(raw, host.resolved_sudo_context()));

// Use exactly like LocalCommandExecutor — plugins are transport-agnostic.
let output = executor
    .execute_quiet(&CommandSpec::exec("uname", ["-r".to_string()]))
    .await?;
```

### How it works

1. `CommandSpec::resolve()` converts the spec to `(program, args)` regardless of mode. Shell mode
   specs are resolved to `(shell_executable, ["-c", wrapped_command])`.
2. `build_remote_command_string()` shell-escapes every component with `shell_escape()` and joins
   them into a single command string. When `working_dir` is set, it prepends `cd '<dir>' &&`.
3. The command string is passed to `SshSession::exec_command_streaming()`, which runs it on the
   remote host and optionally streams output lines through an `mpsc::Sender<UpdateOutputLine>`.
4. Transport errors map to `CommandError::CommandSpawn` and non-zero exit codes map to
   `CommandError::CommandFailed`, matching `LocalCommandExecutor` semantics.

### Output limits

Both `SshCommandExecutor` and `LocalCommandExecutor` enforce a 10 MB output limit to prevent OOM
from runaway commands.

## StdioTunnel

The `StdioTunnel` trait (`crates/shared/command/src/executor.rs`) provides bidirectional byte-stream
tunnelling through a `CommandExecutor`. It enables plugins to communicate with remote processes over
raw stdin/stdout channels rather than line-oriented command output.

```rust
pub trait StdioTunnel: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin {}
```

Two opt-in methods on `CommandExecutor` expose the capability:

| Method | Default | Description |
| --- | --- | --- |
| `supports_stdio_tunnel()` | `false` | Whether this executor can open tunnels |
| `open_stdio_tunnel(command)` | Returns `UnsupportedOperation` error | Open a tunnel to the given command |

`SudoAwareCommandExecutor` delegates both methods directly to its inner executor — tunnels are raw
byte streams that bypass sudo wrapping.

### `SshStdioTunnel`

`SshCommandExecutor` (`crates/core/agent-ssh/src/ssh_executor.rs`) implements `supports_stdio_tunnel()`
returning `true` and opens tunnels via `SshSession::open_channel_for_command()`. The returned
`SshStdioTunnel` (`crates/core/agent-ssh/src/ssh_stdio_tunnel.rs`) wraps a `russh::ChannelStream` and
implements `AsyncRead + AsyncWrite` by delegating through pin projection.

### Docker socket proxy

The Docker plugin uses `StdioTunnel` to proxy Docker API traffic over an SSH connection without
spawning a second SSH session. When `executor.supports_stdio_tunnel()` returns `true` and no explicit
`docker_host` is configured, `DockerSocketProxy` (`crates/plugins/releases/docker/src/docker_proxy.rs`)
starts a local Unix socket that bridges each accepted connection to `docker system dial-stdio` via the
tunnel. Bollard then connects to this local socket using its `unix://` codepath.

See [Docker Plugin — Remote Docker via SSH](../end-user/plugins/docker.md#remote-docker-via-ssh) for
end-user details.

## Implementing additional executors

To run commands over a different transport, implement the `CommandExecutor` trait:

```rust
#[async_trait]
impl CommandExecutor for MyExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<CommandOutput> {
        let (program, args) = spec.resolve();
        // Run the command via your transport, stream output through output_tx
        todo!()
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> Result<CommandOutput> {
        let (program, args) = spec.resolve();
        // Run the command, accumulate output
        todo!()
    }
}
```

## Testing

In tests, use `LocalCommandExecutor` wrapped in `Arc`:

```rust
fn test_executor() -> Arc<dyn CommandExecutor> {
    Arc::new(LocalCommandExecutor)
}

#[test]
fn create_plugin() {
    let plugin = MyPlugin::new(config, test_executor());
    // ...
}
```

For unit tests that should not execute real commands, implement a mock executor:

```rust
struct MockExecutor {
    output: String,
    exit_code: i32,
}

#[async_trait]
impl CommandExecutor for MockExecutor {
    async fn execute(
        &self,
        _spec: &CommandSpec,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput {
            output: self.output.clone(),
            exit_code: self.exit_code,
        })
    }

    async fn execute_quiet(&self, _spec: &CommandSpec) -> Result<CommandOutput> {
        Ok(CommandOutput {
            output: self.output.clone(),
            exit_code: self.exit_code,
        })
    }
}
```
