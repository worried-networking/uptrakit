# AGENTS.md — AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before making any changes.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an auto-updater.

Key components:

- **Controller** (server): API, Web UI, scheduler, MQTT/Home Assistant integration, remote provider logic.
- **Agents**: lightweight daemons on each managed host; outbound-only secure WebSocket to the controller; local version detection and update execution via sudo allowlists.
- **Providers**: pluggable modules that define how to detect installed versions, resolve latest versions, and perform updates.

For full project context, see [README.md](README.md). For contribution rules, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Codebase layout

```text
uptrakit/
├── Cargo.toml                          # Workspace root (resolver = "3", members = "crates/*/*")
├── crates/
│   ├── core/
│   │   ├── agent/                      # uptrakit-agent                         (bin)  — agent daemon
│   │   └── controller/                 # uptrakit-controller                    (bin)  — central server
│   ├── providers/
│   │   ├── core/                       # uptrakit-provider-core                 (lib)  — provider trait/abstractions
│   │   ├── github/                     # uptrakit-provider-github               (lib)  — GitHub Releases provider
│   │   └── proxmox-helper-scripts/     # uptrakit-provider-proxmox-helper-scripts (lib) — PVE helper-scripts provider
│   ├── shared/
│   │   ├── core/                       # uptrakit-core                          (lib)  — shared domain models
│   │   └── wire/                       # uptrakit-internal-wire                 (lib)  — agent<->controller wire protocol
│   └── ui/
│       ├── cli/                        # uptrakit-cli                           (bin)  — CLI interface
│       ├── mqtt/                       # uptrakit-mqtt                          (lib)  — MQTT / Home Assistant integration
│       └── web-api/                    # uptrakit-web-api                       (lib)  — HTTP API
├── .github/
│   ├── workflows/ci.yml                # CI: fmt check, clippy, tests (runs on macOS)
│   └── dependabot.yml                  # Weekly Cargo dependency updates
├── CONTRIBUTING.md
├── README.md
└── AGENTS.md                           # This file
```

All crates use **edition = "2024"**. Some specify `rust-version = "1.91"`.

## Quality gates (must pass before committing)

```sh
cargo fmt --all                                              # Format
cargo clippy --all-targets --all-features -- -D warnings     # Lint (zero warnings)
cargo test --all-features                                    # Tests
```

CI runs these same checks. A PR that fails any of them will not merge.

## Commit messages

**Conventional Commits are required.** Format:

```gitmessage
<type>(optional-scope): <description>
```

Types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`.

Scopes typically match crate or component names: `agent`, `controller`, `provider-core`, `provider-github`, `provider-phs`, `mqtt`, `web`, `cli`, `wire`, `core`.

Breaking changes: add `!` after type/scope, e.g. `feat(api)!: change ws handshake payload`.

Examples:

- `feat(agent): add helper-scripts autodiscovery`
- `fix(controller): handle websocket reconnect backoff`
- `refactor(provider-github): simplify release tag normalisation`

## Architecture rules and invariants

These are non-negotiable design constraints. Do not violate them.

1. **Updates are never automatic.** The scheduler triggers version *checks* only. Update execution requires explicit user action (via UI, CLI, or MQTT/Home Assistant).
2. **Agents initiate outbound-only connections.** Agents connect to the controller via secure WebSocket (`/api/v1/ws/agent`). They never listen on any port or accept inbound connections.
3. **Agents run unprivileged.** They run as a dedicated user (e.g. `uptrakit`). Only specific update commands are granted `NOPASSWD` sudo access.
4. **Provider split.** Remote (upstream version resolution) logic runs on the controller. Local (installed version detection + update execution) logic runs on the agent. Keep this boundary clear.
5. **No shell injection.** Any path that constructs or executes shell commands must validate inputs. Custom scripts are treated as untrusted input.
6. **No secrets in logs.** Never log tokens, passwords, API keys, or other credentials.
7. **Logging goes to journald or stdout.** No internal log storage. Full command output is not captured internally — only high-level summaries are retained for display.
8. **No overlapping update actions per host.** The scheduler must ensure that two update operations for the same host never run concurrently.

## Error handling

Use the [`rootcause`](https://github.com/rootcause-rs/rootcause) crate for error propagation and handling. Use [`thiserror`](https://github.com/dtolnay/thiserror) for constructing and designing errors.

- Add context at boundaries (host, provider, software item, operation).
- Prefer structured context over generic string errors.
- Never log or expose secrets in error messages.

## Testing expectations

Every behaviour change must include tests. Types of tests used:

- **Unit tests**: pure logic, version comparison, parsing.
- **Provider tests**: parsing upstream metadata, mapping to internal models.
- **API boundary tests**: request/response (de)serialisation, backwards compatibility.
- **Error path tests**: expected failures produce correct error types and messages.

Run tests with:

```sh
cargo test --all-features
# or with nextest:
cargo nextest run --all-features
```

## Provider architecture

Each software item is associated with a provider. A provider defines:

| Concern | Runs on | Responsibility |
| --- | --- | --- |
| Remote/upstream version | Controller | Fetch latest version metadata (version string, release URL, changelog URL, publish timestamp, channel, notes) |
| Local/installed version | Agent | Detect currently installed version |
| Update execution | Agent | Run the update (via sudo-allowlisted commands or custom script) |

Provider crates:

| Crate | Path | Purpose |
| --- | --- | --- |
| `uptrakit-provider-core` | `crates/providers/core/` | Shared provider traits and abstractions |
| `uptrakit-provider-github` | `crates/providers/github/` | GitHub Releases: controller fetches release metadata; agent installs from artifacts |
| `uptrakit-provider-proxmox-helper-scripts` | `crates/providers/proxmox-helper-scripts/` | Proxmox VE Helper-Scripts: agent auto-discovers and manages helper-script-installed apps |

The update step can always be overridden by a custom shell script, regardless of provider.

When adding or changing a provider, document in the same PR:

- How installed version is detected (agent side)
- How upstream/latest version is determined (controller side)
- Version comparison rules (semver, tag prefixes, build metadata handling)
- Update mechanism, required privileges, and failure modes
- Required config fields with examples

## Home Assistant / MQTT integration

Each tracked software item becomes a Home Assistant `update` entity via MQTT auto-discovery. Entity attributes include: installed version, latest version, changelog URL, release link, and more. Updates can be triggered from Home Assistant, the Web UI, or the CLI.

## Dependencies policy

- Avoid heavy dependencies without strong justification.
- Prefer well-maintained crates with clear track records.
- Crates affecting command execution, untrusted input parsing, crypto, or networking receive extra scrutiny.

## Release profile

The workspace uses an optimised release profile:

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```
