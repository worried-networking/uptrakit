# Uptrakit

[![codecov](https://codecov.io/gh/worried-networking/uptrakit/branch/main/graph/badge.svg)](https://codecov.io/gh/worried-networking/uptrakit)

> **Note:** Uptrakit is in early development. APIs may change, documentation may be incomplete,
> and some features are still being built. Contributions and feedback are welcome!

Uptrakit is a self-hosted **update tracking toolkit** for Linux homelabs and small fleets.

It’s intentionally **not** an auto-updater: the controller can _check_ for updates on a schedule, but **all update actions require explicit user
confirmation**.

## What it does

- Tracks installed software versions across multiple hosts
- Checks upstream for newer releases using a plugin system (e.g. GitHub Releases, Proxmox VE Helper-Scripts)
- Runs **manual, user-triggered** updates and reports results
- Exposes a minimal Web UI + API
- Integrates with Home Assistant using MQTT `update` auto-discovery so each tracked item shows up as an Update entity
- Exposes an MCP server with OAuth 2.1 authorization so AI assistants such as Claude Desktop and
  Cursor can connect securely. See [MCP Clients](docs/end-user/mcp-clients.md) for user setup and
  [OAuth Clients](docs/admin/oauth-clients.md) for operator registration.

## Security stance

- Agents run unprivileged (e.g. `uptrakit`)
- Privileged operations are constrained via a sudo allowlist (`NOPASSWD` for specific commands only)
- Agents accept no inbound connections
- Updates are always manual

## Configuration

Uptrakit Controller reads its configuration from a single TOML file.

**Default paths** (first match wins):

1. `~/.config/uptrakit/controller/controller.toml` (Linux XDG config dir)
2. `/etc/uptrakit/controller.toml`

**Override:**

- `--config <path>` flag
- `UPTRAKIT_CONFIG` environment variable

**Surviving CLI flags** (all other flags moved to the TOML file):

| Flag                      | Description                                                   |
| ------------------------- | ------------------------------------------------------------- |
| `--config <path>`         | Path to the TOML configuration file                           |
| `--master-key-from <src>` | Master encryption key source (`file:`, `env:`, or inline hex) |
| `--migrate-and-exit`      | Run DB migrations and exit                                    |
| `--check-config`          | Validate the config file and exit                             |
| `--version`               | Print version information                                     |
| `--verbose` / `-v`        | Increase log verbosity                                        |

For the full config schema and reload mechanics, see the
[operator runbook](docs/end-user/operator-runbook-reload.md) and
[ADR 0008](docs/adr/0008-graceful-reload-architecture.md).

## Development

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Contribution guidelines and project conventions:

- [CONTRIBUTING.md](CONTRIBUTING.md)

## Documentation

### Core overviews

- [ARCHITECTURE.md](ARCHITECTURE.md) — condensed architecture overview
- [SECURITY.md](SECURITY.md) — high-level security stance and disclosure/reporting policy
- [CONTRIBUTING.md](CONTRIBUTING.md) — quick contributor overview
- [docs/README.md](docs/README.md) — catalogue listing all audience-focused documents
- [TODO.md](TODO.md) — roadmap and planned work

### Audience docs

- [docs/end-user/](docs/end-user/) — end-user documentation
- [docs/api/](docs/api/) — HTTP API and Wire protocol documentation
- [docs/security/](docs/security/) — detailed security description
- [docs/development/](docs/development/) — setup, testing, coding standards, PR process, dependency rules, plugin expectations, and AI guidance
- [website/](website/) — public marketing site at <https://uptrakit.org>

## License

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

## AI Disclosure

This project is being built with significant use of agentic AI tools (Claude Code, Codex, Gemini), under active human review and guidance.
The project is in active early development and a full code review has not yet been conducted. AI assistance does not reduce the
project's engineering standards: merged changes are still expected to meet the same correctness, security, documentation, and
maintainability requirements as any other contribution.

The project has not undergone a formal 3rd-party security audit. As with any software published online, use at your own risk.
