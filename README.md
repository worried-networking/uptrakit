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

### Core overviews
- `ARCHITECTURE.md` — condensed architecture overview linking into `docs/`
- `SECURITY.md` — high-level security stance and disclosure/reporting policy
- `CONTRIBUTING.md` — quick contributor overview linking into `docs/development/`
- `docs/README.md` — catalogue listing all audience-focused documents
- `TODO.md` — roadmap and planned work

### Audience docs
- `docs/end-user/system-overview.md`, `docs/end-user/update-workflow.md`, `docs/end-user/home-assistant-mqtt.md`, `docs/end-user/deployment-map.md`
- `docs/end-user/deployment/reverse-proxy.md` — reverse proxy deployment details
- `docs/api/wire-protocol.md`, `docs/api/http-web-api.md`, `docs/api/settings-runtime.md`, `docs/api/auth-flows.md`, `docs/api/services-operations.md`
- `docs/security/*.md` — detailed security guides (`security-architecture`, `pki-certificates`, `auth-and-authorization`, `reverse-proxy`, etc.)
- `docs/development/*.md` — setup, testing, coding standards, PR process, dependency rules, provider expectations, and AI guidance


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
