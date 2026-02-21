# CLI output formatting

The CLI supports three output formats via the global `--output` / `-o` flag:

| Format | Flag value | Behaviour |
| --- | --- | --- |
| Human (default) | `human` | Columnar / free-text output identical to pre-flag behaviour |
| JSON | `json` | Compact single-line JSON, suitable for `jq` piping |
| YAML | `yaml` | YAML output via `serde_yaml_ng` |

## Implementation

- `OutputFormat` enum in `crates/ui/cli/src/output.rs` — derives `clap::ValueEnum` and `Default` (`Human`).
- `print_output<T: Serialize>(format, human_text, value)` — for typed commands (`auth status`, `auth token *`,
  `hosts`, `services`, `software-items`, `check`, `update`, `history`, `scheduler`).
- `print_value(format, &serde_json::Value)` — for the `api` command which works with raw JSON values.
  In Human format, the HTTP status line is printed to stdout before the body.
  In JSON/YAML format, the status is wrapped in the response envelope.
- Each structured command uses server response types from `uptrakit-web-api-types` (e.g. `HostResponse`,
  `SoftwareItemResponse`, `UpdateHistoryResponse`, `ScheduledTaskResponse`) for JSON/YAML output while building
  human-readable table/detail text for the human format.
- `auth login` is interactive and does not support `--output`.

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
