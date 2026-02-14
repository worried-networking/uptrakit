# AGENTS -- AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before
making any changes.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software
versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an
auto-updater.

Key components:

- **Controller** (server): API, Web UI, scheduler, upstream version checking.
- **MQTT Service** (standalone binary): MQTT/Home Assistant integration with lease-based multi-instance tenant
  distribution.
- **Agents**: lightweight daemons on each managed host; outbound-only secure WebSocket to the controller; local version
  detection and update execution via sudo allowlists.
- **Providers**: pluggable modules that define how to detect installed versions, resolve latest versions, and perform
  updates.

For full project context, see [README.md](README.md). For contribution rules, see [CONTRIBUTING.md](CONTRIBUTING.md).
For system design and technology choices, see [ARCHITECTURE.md](ARCHITECTURE.md). For security policy and cryptographic
details, see [SECURITY.md](SECURITY.md). For the documentation catalogue, see [docs/README.md](docs/README.md).

## Documentation split

- **End-user docs** ([`docs/end-user/`](docs/end-user/)): overview, manual update workflow, Home Assistant/MQTT
  integration, and deployment map (including
  [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)).
- **API & protocol docs** ([`docs/api/`](docs/api/)): AsyncAPI/wire protocol
  ([wire-protocol.md](docs/api/wire-protocol.md)), REST API endpoints ([http-web-api.md](docs/api/http-web-api.md)),
  settings reconciliation ([settings-runtime.md](docs/api/settings-runtime.md)), auth flows
  ([auth-flows.md](docs/api/auth-flows.md)), and service/tenant operations
  ([services-operations.md](docs/api/services-operations.md)).
- **Security docs** (`docs/security/`): architecture, cryptography, PKI/certificates, auth/permissions,
  secrets/encryption, reverse proxy security, TOFU/TLS, filesystem/dependency hardening, and secure development
  guidance.
- **Architecture docs** ([`docs/architecture/`](docs/architecture/)): entity-level design for multi-tenancy, hosts,
  software items, and update history.
- **Development docs** (`docs/development/`): setup, testing, coding standards, PR process, dependency policy, provider
  guidelines, and AI usage expectations.
- **Deployment guides**: reverse proxy deployment and per-proxy guides live under
  [`docs/end-user/deployment/`](docs/end-user/deployment/). Reverse proxy security model is at
  [`docs/security/reverse-proxy-security.md`](docs/security/reverse-proxy-security.md). Human documentation must link
  into those files rather than [AGENTS.md](AGENTS.md).

## Codebase layout

```text
uptrakit/
├── Cargo.toml                          # Workspace root (resolver = "3", members = "crates/*/*")
├── crates/
│   ├── core/
│   │   ├── agent/                      # uptrakit-agent                         (bin)  — agent daemon
│   │   ├── controller/                 # uptrakit-controller                    (bin)  — central server
│   │   └── mqtt/                       # uptrakit-mqtt                          (bin)  — standalone MQTT service
│   ├── providers/
│   │   ├── core/                       # uptrakit-provider-core                 (lib)  — provider trait/abstractions (re-exports shared types; delegates command execution to uptrakit-command)
│   │   ├── docker-registry/            # uptrakit-provider-docker-registry      (lib)  — Docker/OCI Registry provider
│   │   ├── github/                     # uptrakit-provider-github               (lib)  — GitHub Releases provider
│   │   ├── homebrew/                   # uptrakit-provider-homebrew              (lib)  — Homebrew formulae/cask provider
│   │   ├── proxmox-helper-scripts/     # uptrakit-provider-proxmox-helper-scripts (lib) — PVE helper-scripts provider
│   │   └── registry/                   # uptrakit-provider-registry             (lib)  — provider dispatch & validation
│   ├── shared/
│   │   ├── command/                    # uptrakit-command                       (lib)  — shell command execution (shell_escape, run_command_*)
│   │   ├── core/                       # uptrakit-core                          (lib)  — shared domain models
│   │   ├── db/                         # uptrakit-shared-db                     (lib)  — SeaORM entities, migrations & crypto
│   │   ├── directories/                # uptrakit-directories                   (lib)  — cross-platform directory management
│   │   ├── macros/                     # uptrakit-shared-macros                 (lib)  — shared declarative macros (impl_report_conversion!)
│   │   ├── types/                      # uptrakit-shared-types                  (lib)  — shared value types (ProviderType, ReleaseAsset, ReleaseInfo, SecretString, hex encode/decode)
│   │   ├── web-api-types/              # uptrakit-web-api-types                 (lib)  — shared HTTP request/response types
│   │   ├── service-sdk/                # uptrakit-service-sdk                   (lib)  — shared service SDK (lifecycle, enrollment, identity, TLS, CA bootstrap, CLI, ControllerConnection)
│   │   └── wire/                       # uptrakit-internal-wire                 (lib)  — service<->controller wire protocol
│   └── ui/
│       ├── cli/                        # uptrakit-cli                           (bin)  — CLI interface
│       └── web-api/                    # uptrakit-web-api                       (lib)  — HTTP API
├── frontend/                           # SvelteKit SPA (Skeleton UI v4 + Tailwind CSS v4)
│   ├── src/
│   │   ├── lib/                        # Shared modules: api client, auth store, types, utils, notifications
│   │   │   └── components/             # Shared UI: ConfirmDialog, ModalBackdrop (focus-trapped), ContextMenu (viewport-aware), Pagination
│   │   └── routes/                     # SvelteKit file-based routes
│   │       └── settings/               # Settings sub-components (Registration, Auth, MQTT, OIDC, Certs, Enrollment)
│   ├── package.json                    # npm scripts: build, check
│   ├── svelte.config.js                # SvelteKit config (static adapter)
│   ├── tsconfig.json
│   ├── vite.config.ts
│   └── vitest.config.ts
├── .github/
│   ├── workflows/ci.yml                # CI: fmt check, clippy, tests, reverse-proxy Docker tests, frontend check + build
│   └── dependabot.yml                  # Weekly Cargo + npm dependency updates
├── CONTRIBUTING.md
├── README.md
└── AGENTS.md                           # This file
```

All crates use **edition = "2024"**. Some specify `rust-version = "1.91"`.

## General MUST FOLLOW Rules for AI Coding Agents

### Quality Gates

All changes must pass defined quality gates. See [docs/development/quality-gates.md](docs/development/quality-gates.md) for details.

#### AI execution guidance

- Always run quality gates relevant to modified areas before finalizing.
- Scope-based execution is allowed for local iteration:
  - frontend-only changes: run frontend checks (`npm run check`, `npm run build`) and docs checks.
  - Rust/backend-only changes: run Rust checks/tests/linters and docs checks.
  - mixed changes: run both Rust and frontend gates.
- If anything related to reverse proxy behavior changes, run ignored reverse proxy integration tests:
  - `cargo test -p uptrakit-controller reverse_proxy -- --ignored`
- Treat the reverse proxy trigger list broadly, including (non-exhaustive):
  - mTLS and certificate forwarding/extraction
  - auth behavior behind proxies
  - IP detection / `ClientIp`, forwarded headers, trusted-proxy logic
  - reverse proxy middleware/settings and related TLS behavior

### Commit Messages

Conventional Commits are required. See [docs/development/commit-messages.md](docs/development/commit-messages.md) for details.

### Architecture rules and invariants

These are non-negotiable design constraints. Do not violate them.

1. **Updates are never automatic.** The scheduler triggers version *checks* only. Update execution requires explicit
   user action (via UI, CLI, or MQTT/Home Assistant).
1. **Agents initiate outbound-only connections.** Agents connect to the controller via secure WebSocket
   (`/api/v1/ws/service`). They never listen on any port or accept inbound connections.
1. **Agents run unprivileged.** They run as a dedicated user (e.g. `uptrakit`). Only specific update commands are
   granted `NOPASSWD` sudo access.
1. **Provider split.** Most providers resolve upstream versions on the controller and installed versions on the agent.
   Providers with a local package index (e.g. Homebrew) resolve both on the agent via `RefreshPackageIndex` +
   `fetch_releases()` and report `latest_version` in `VersionCheckResult`. Keep this boundary clear.
1. **No shell injection.** Any path that constructs or executes shell commands must validate inputs. Custom scripts are
   treated as untrusted input.
1. **No secrets in logs.** Never log tokens, passwords, API keys, or other credentials.
1. **Logging goes to journald or stdout.** No internal log storage. Full command output is not captured internally --
   only high-level summaries are retained for display.
1. **No overlapping update actions per host.** The scheduler must ensure that two update operations for the same host
   never run concurrently.
1. **No raw SQL.** Use the structures and methods provided by Sea ORM everywhere. **Approved exception:** The rate
   limiter (`crates/ui/web-api/src/auth/rate_limit.rs`) uses `sea_orm::Statement::from_sql_and_values()` for an atomic
   `INSERT ... ON CONFLICT DO UPDATE SET ... CASE WHEN` upsert, because SeaORM's `on_conflict` builder doesn't support
   conditional expressions. The statement is fully parameterized (no injection risk).
1. **Cover new logic with tests.** Cover success and failure paths.
1. **Document everything.** Any code change must be properly documented either in the code, or in the separate
   documentation. Any changes to the agent-controller wire protocol must be documented in
   `crates/shared/wire/asyncapi.yaml` and reflected in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).
1. **Version/build metadata contract is unified.** All workspace binaries (`uptrakit-controller`, `uptrakit-agent`,
   `uptrakit-mqtt`, `uptrakit-cli`) must expose consistent `--version` metadata output. Enabled features are derived at
   build time from `CARGO_CFG_FEATURE` via `uptrakit_build_info::emit_enabled_features_env()` and passed through
   `UPTRAKIT_BUILD_ENABLED_FEATURES`; do not hardcode feature lists per binary.
1. **Do not add any `#[allow()]`** without explicit confirmation. There are currently no approved exceptions in the
   codebase; all previously allowed lints have been resolved via parameter structs, `FromStr` implementations, or dead
   code removal.
1. **Do not use `unsafe`, `unwrap` or `panic!`.** Always prefer safe and graceful solutions. Follow the error handling
   requirements in [docs/development/coding-standards.md](docs/development/coding-standards.md): define typed errors
   with `thiserror` and attach/propagate context with `rootcause` (including match-with-fallback and serialization
   helper patterns where applicable).
   **Approved exceptions**: `Mutex::lock().unwrap()`, `RwLock::read().unwrap()`, and `RwLock::write().unwrap()` are safe
   because `panic = "abort"` in the release profile makes lock poisoning impossible.

### Error handling quick reference

Every boundary (crate or module) must define its own typed error enum. Here is the minimal setup and decision guide.

**Required imports:**

```rust
use rootcause::prelude::*;      // Report, report!, bail!, ResultExt, etc.
use thiserror::Error;            // #[derive(Debug, Error)]
use uptrakit_shared_macros::impl_report_conversion;  // cross-boundary conversions
```

**Boundary checklist:**

1. Define `#[derive(Debug, Error)] pub enum MyError { ... }`
1. Define `pub type Result<T> = std::result::Result<T, Report<MyError>>;`
1. Add `impl_report_conversion!` for every foreign error type your boundary encounters.

**`bail!()` vs `report!()`:**

- `bail!(MyError::Variant(...))` — use for guard-clause early returns (replaces `return Err(report!(...))`).
- `report!(MyError::Variant(...))` — use inside `.ok_or_else()`, `.map_err()`, or when building a `Report` without
  returning.

**Decision table — which context method to use:**

| Scenario | Method |
| --- | --- |
| Foreign error has `ReportConversion` impl | `.context_to()` |
| Wrap low-level error with high-level meaning | `.context(Higher::Variant)` |
| Change error type in-place (1:1 mapping) | `.context_transform(\|e\| ...)` |
| One-off conversion, no impl needed | `.map_err(\|e\| report!(...))` |
| Guard clause / early return | `bail!(...)` |

**Approved exceptions:**

- `Mutex::lock().unwrap()`, `RwLock::read().unwrap()`, `RwLock::write().unwrap()` — safe because `panic = "abort"` in
  release.
- String-based error variants for external types that don't impl `std::error::Error` (e.g. `aws_lc_rs::Unspecified`):
  `.map_err(|e| report!(Err::Variant(e.to_string())))`.

Full details: [docs/development/coding-standards.md](docs/development/coding-standards.md).

### Directory management

All binaries (controller, agent, MQTT service) use the `uptrakit-directories` crate for cross-platform directory
resolution. The crate uses the `directories` crate (`ProjectDirs`) to follow platform conventions:

| Platform | Config directory | State directory |
| --- | --- | --- |
| Linux | `~/.config/{app}/` (XDG) | `~/.local/state/{app}/` (XDG) |
| macOS | `~/Library/Application Support/io.uptrakit.{app}/` | `~/Library/Application Support/io.uptrakit.{app}/` |
| Windows | `{FOLDERID_RoamingAppData}\uptrakit\{app}\` | `{FOLDERID_LocalAppData}\uptrakit\{app}\` |

Where `{app}` is one of: `controller`, `agent`, `mqtt`.

#### Config vs state separation

| Directory | Contents | Characteristics |
| --- | --- | --- |
| **Config** | Rarely-changing, persistent configuration | External CA certificates, user-provided TLS certs |
| **State** | Runtime state that may change frequently | SQLite DB, JWT keys, service identity, private keys, issued certificates |

**Controller:**

- Config: External CA certificate/key (if configured), server TLS certificate/key
- State: SQLite database (includes managed CA history, JWT signing key)

**Agent/MQTT Service:**

- Config: Controller's CA certificate
- State: Service ID, private key, issued certificate

#### CLI directory flags

All binaries support `--config-dir` and `--state-dir` CLI flags (and corresponding `UPTRAKIT_CONFIG_DIR` /
`UPTRAKIT_STATE_DIR` environment variables) to override the platform defaults. Both support `~` expansion for home
directory paths.

#### Secure permissions

All created files and directories use secure permissions:

- **Directories**: 0o700 (owner read/write/execute only)
- **Files**: 0o600 (owner read/write only)

The `uptrakit-directories` crate provides helper functions:

- `create_secure_dir(path)` -- creates directory with 0o700 permissions
- `write_secure_file(path, data)` -- writes file with 0o600 permissions
- `AppDirs::resolve(app_kind, config_override, state_override)` -- resolves directories for an application
- `AppDirs::ensure_dirs()` -- creates both directories with secure permissions

#### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/directories/src/lib.rs` | Cross-platform directory resolution and secure file/directory operations |

## Detailed Documentation References

For more in-depth information on specific topics, refer to the following documents:

### Security

- [PKI and Certificate Lifecycle](docs/security/pki-certificates.md)
- [Secrets Handling and Encryption](docs/security/secrets-and-encryption.md)
- [TOFU and TLS Hardening](docs/security/tofu-tls.md)
- [Authentication and Authorization](docs/security/auth-and-authorization.md)
- [Cryptography](docs/security/cryptography.md)
- [Security Architecture](docs/security/security-architecture.md)
- [Filesystem and Dependency Security](docs/security/filesystem-dependency-security.md)
- [Reverse Proxy Security](docs/security/reverse-proxy-security.md)

### Development Guidelines

- [Quality Gates](docs/development/quality-gates.md)
- [Commit Messages](docs/development/commit-messages.md)
- [CLI Output Formatting](docs/development/cli-output.md)
- [Graceful Restart](docs/development/graceful-restart.md)
- [Cross-Controller Communication](docs/development/cross-controller-comm.md)
- [Coding Standards (Error Handling)](docs/development/coding-standards.md)
- [Testing Expectations](docs/development/testing.md)
- [Provider Guidelines](docs/development/provider-guidelines.md)
- [Update Hooks](docs/development/update-hooks.md)
- [Service Lifecycle](docs/development/service-lifecycle.md)

### Architecture

- [Multi-Tenancy](docs/architecture/multi-tenancy.md)
- [Host Entity](docs/architecture/host-entity.md)
- [Software Item Entity](docs/architecture/software-item-entity.md)
- [Update History Entity](docs/architecture/update-history-entity.md)

### API and Protocol

- [Wire Protocol](docs/api/wire-protocol.md)
- [Authentication Flows](docs/api/auth-flows.md)
- [Settings Runtime](docs/api/settings-runtime.md)
- [HTTP Web API](docs/api/http-web-api.md)
- [Services and Operations](docs/api/services-operations.md)
