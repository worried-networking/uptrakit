# Uptrakit

Uptrakit is a self-hosted **update tracking toolkit** for Linux homelabs and small fleets.

It’s intentionally **not** an auto-updater: the controller can *check* for updates on a schedule, but **all update actions require explicit user confirmation**.

## What it does

- Tracks installed software versions across multiple hosts
- Checks upstream for newer releases using a provider system (e.g. GitHub Releases, Proxmox VE Helper-Scripts)
- Runs **manual, user-triggered** updates and reports results
- Exposes a minimal Web UI + API
- Integrates with Home Assistant using MQTT `update` auto-discovery so each tracked item shows up as an Update entity

## Architecture

- **Controller**: API + minimal Web UI, scheduler (checks only), provider “remote” logic, and Home Assistant integration.
- **Agents**: lightweight daemons on each host that connect **outbound-only** to the controller via secure WebSocket. They detect installed versions and execute updates via **sudo allowlists**.

## Security stance

- Agents run unprivileged (e.g. `uptrakit`)
- Privileged operations are constrained via a sudo allowlist (`NOPASSWD` for specific commands only)
- Agents accept no inbound connections
- Updates are always manual

## Development

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Contribution guidelines and project conventions:

- [CONTRIBUTING.md](CONTRIBUTING.md)

## Documentation

| Document | Description |
| --- | --- |
| [ARCHITECTURE.md](ARCHITECTURE.md) | System design, component diagram, technology stack, key decisions |
| [SECURITY.md](SECURITY.md) | Security policy, vulnerability reporting, cryptographic details |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Development setup, testing, commit conventions, PR guidelines |
| [AGENTS.md](AGENTS.md) | AI agent guide: codebase layout, quality gates, error patterns |
| [docs/](docs/README.md) | Documentation catalogue and planned guides |
| [TODO.md](TODO.md) | Project roadmap and progress tracker |

## License

Licensed under either of

- Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## AI Disclosure

The initial codebase for this project was significantly shaped by AI (Claude Code, Codex, Gemini) under heavy human supervision and constant code reviews. While efforts were made to ensure quality and correctness, the project has not undergone a formal 3rd-party security audit. As with any software published online, use at your own risk.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
