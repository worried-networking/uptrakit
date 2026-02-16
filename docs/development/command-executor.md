# Command Executor

The `CommandExecutor` trait decouples provider logic from the command execution transport. Providers build a
`CommandSpec` describing *what* to run, and the injected executor decides *how* to run it (locally, over SSH, etc.).

**Related docs:**

- [Provider Guidelines](provider-guidelines.md) -- provider architecture and construction
- [Coding Standards](coding-standards.md) -- error handling conventions used throughout
- [SSH Agent Architecture](../architecture/ssh-agent.md) -- future SSH executor use case
- [Secure Development](../security/secure-development.md) -- shell injection prevention

## Key types

All types live in `crates/shared/command/src/executor.rs` and are re-exported from `uptrakit_command` and
`uptrakit_provider_core::command`.

### `CommandSpec`

Describes a command to execute without specifying the transport.

```rust
pub struct CommandSpec {
    pub mode: CommandMode,
    pub working_dir: Option<String>,
}
```

Convenience constructors:

| Constructor | Description |
| --- | --- |
| `CommandSpec::exec(program, args)` | Direct program execution (no shell). |
| `CommandSpec::shell(command)` | Shell command via Bash with fail-early settings (`set -euo pipefail`). |
| `CommandSpec::shell_with(command, shell)` | Shell command with a specific shell (`Bash`, `Sh`). |
| `.with_working_dir(dir)` | Builder method to set the working directory. |

### `CommandMode`

```rust
pub enum CommandMode {
    Exec { program: String, args: Vec<String> },
    Shell { command: String, shell: ShellType },
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

### `LocalCommandExecutor`

The default implementation. Delegates to `tokio::process::Command` on the local machine.

```rust
pub struct LocalCommandExecutor;
```

No configuration needed -- instantiate with `LocalCommandExecutor` and wrap in `Arc`:

```rust
let executor: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
```

## Provider construction

Every provider struct stores `Arc<dyn CommandExecutor>` and receives it via the constructor:

```rust
pub struct MyProvider {
    config: MyConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl MyProvider {
    pub fn new(config: MyConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        Self { config, executor }
    }
}
```

The `ProviderRegistry::create_provider()` method accepts the executor and forwards it to each provider:

```rust
let provider = ProviderRegistry::create_provider(
    ProviderType::GithubReleases,
    &config,
    executor,
)?;
```

## Using the executor in providers

### Streaming execution (e.g., `execute_update`)

```rust
let cmd_output = self
    .executor
    .execute(&CommandSpec::shell(&install_cmd), output_tx)
    .await
    .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
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

```rust
use std::sync::Arc;
use uptrakit_command::{CommandExecutor, CommandSpec};

// Create from an authenticated SSH session.
let executor: Arc<dyn CommandExecutor> = Arc::new(SshCommandExecutor::new(session));

// Use exactly like LocalCommandExecutor — providers are transport-agnostic.
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
fn create_provider() {
    let provider = MyProvider::new(config, test_executor());
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
