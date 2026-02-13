# CLI output formatting

The CLI supports three output formats via the global `--output` / `-o` flag:

| Format | Flag value | Behaviour |
| --- | --- | --- |
| Human (default) | `human` | Columnar / free-text output identical to pre-flag behaviour |
| JSON | `json` | Compact single-line JSON, suitable for `jq` piping |
| YAML | `yaml` | YAML output via `serde_yaml_ng` |

## Implementation

- `OutputFormat` enum in `crates/ui/cli/src/output.rs` — derives `clap::ValueEnum` and `Default` (`Human`).
- `print_output<T: Serialize>(format, human_text, value)` — for typed commands (`auth status`, `auth token *`).
- `print_value(format, &serde_json::Value)` — for the `api` command which works with raw JSON values.
- Each structured command defines a serializable response struct (e.g. `AuthStatusOutput`, `TokenCreateOutput`,
  `TokenListOutput`, `TokenRevokeOutput`) in `commands/auth.rs`.
- `auth login` is interactive and does not support `--output`.

## Example usage

```sh
uptrakit-cli auth status                     # human-readable (default)
uptrakit-cli auth status -o json             # compact JSON
uptrakit-cli auth token list --output yaml   # YAML
uptrakit-cli api GET /api/v1/auth/me -o json # compact JSON for raw API calls
```
