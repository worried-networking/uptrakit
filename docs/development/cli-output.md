# CLI output formatting

The CLI supports three output formats via the global `--output` / `-o` flag:

| Format | Flag value | Behaviour |
| --- | --- | --- |
| Human (default) | `human` | Columnar / free-text output identical to pre-flag behaviour |
| JSON | `json` | Compact single-line JSON, suitable for `jq` piping |
| YAML | `yaml` | YAML output via `serde_yaml_ng` |

## Implementation

### `HumanOutput` trait

Every response type that can appear on stdout implements the `HumanOutput` trait
defined in `crates/ui/cli/src/output.rs`:

```rust
pub trait HumanOutput {
    fn to_human_string(&self) -> String;
}
```

This keeps terminal formatting logic co-located with the types that own it, rather
than mixing it with API-orchestration code in command handler functions.

**Placement rules:**

- **Local CLI types** (`AuthStatusOutput`, `MergeServiceOutput`, `DeletedOutput`, etc.) —
  `impl HumanOutput` lives immediately after the struct definition in the same file.
- **External API types** (`HostResponse`, `ServiceResponse`, `PaginatedResponse<T>`, etc.) —
  `impl HumanOutput` lives in the command file that owns those types, under a
  `// ── Human output ─────` section comment above the command functions.
- **`BuildInfo`** — implemented in `src/main.rs` just above `async fn run`, delegating
  to `BuildInfo::render_human()`.

### `print_output` function

```rust
pub fn print_output<T: Serialize + HumanOutput>(
    format: OutputFormat,
    value: &T,
) -> Result<()>
```

- `Human` branch — calls `value.to_human_string()` and prints to stdout.
- `Json` branch — serialises via `serde_json` and prints compact JSON.
- `Yaml` branch — serialises via `serde_yaml_ng` and prints YAML.

`print_output` is called in `src/main.rs` inside `async fn run`, after each typed
command returns its response. The command layer never receives or handles
`OutputFormat`; formatting is entirely the responsibility of `main.rs`.

### `print_value` function

`print_value(format, &serde_json::Value)` is used exclusively by the `api` command,
which works with raw JSON values rather than typed responses. The `api` command
retains its own `format` parameter for this reason.

### Command return types

Command functions return `Result<ConcreteResponseType>` instead of `Result<()>`.
`format: OutputFormat` is absent from every `Params` struct and every bare command
function signature. Example:

```rust
// Before
pub async fn list(params: ListParams<'_>) -> Result<()>

// After
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<HostResponse>>
```

`auth::login` is the only exception — it remains `Result<()>` because it is fully
interactive and has no structured output.

### Special-case wrapper types

| Type | Location | Purpose |
| --- | --- | --- |
| `MergeServiceOutput` | `commands/services.rs` | Bundles `source_id` with the merged `ServiceResponse` |
| `DeletedOutput` | `commands/settings.rs` | Wraps a plain `{ message }` for MQTT/OIDC delete endpoints |

Both implement `Serialize` (for JSON/YAML output) and `HumanOutput`.

## Version metadata output

`uptrakit --version` prints crate/build metadata and supports all output formats:

- `--output human` (default): deterministic line-based `key: value` output.
- `--output json`: compact JSON.
- `--output yaml`: YAML.

Examples:

```sh
uptrakit --version
uptrakit --version --output json
uptrakit --version --output yaml
```

Service/controller binaries (`uptrakit-agent`, `uptrakit-mqtt`, `uptrakit-controller`) also expose
`--version` with the same deterministic human-format keys for automation.

## Example usage

```sh
uptrakit auth status                     # human-readable (default)
uptrakit auth status -o json             # compact JSON
uptrakit auth token list --output yaml   # YAML
uptrakit api GET /api/v1/auth/me -o json # compact JSON for raw API calls
uptrakit hosts list -o json              # hosts as JSON
uptrakit history list --status failed    # filtered human-readable list
uptrakit scheduler show <ID> -o yaml     # scheduler task as YAML
```

For the full command reference see [CLI Usage Guide](../end-user/cli-usage.md).
