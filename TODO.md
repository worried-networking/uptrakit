# Uptrakit Project Roadmap

## About This Document

This roadmap tracks the development of Uptrakit, a self-hosted software update monitoring and management system. It organizes planned work into
logical phases with clear priorities and dependencies.

**Current Status**: Early MVP stage with foundational infrastructure in place.

**Key Documentation**:

- [README.md](README.md) - Project overview and architecture
- [ARCHITECTURE.md](ARCHITECTURE.md) - System design and technology decisions
- [SECURITY.md](SECURITY.md) - Security policy and cryptographic details
- [CONTRIBUTING.md](CONTRIBUTING.md) - Development setup and contribution guidelines
- [docs/README.md](docs/README.md) - Documentation catalogue

**How to Use This Roadmap**:

- Items are organized by priority and dependencies
- Check off items as they're completed
- Review regularly to adjust priorities based on project needs
- Foundation items should generally be completed before moving to higher-level features

______________________________________________________________________

## Phase 1: Foundation Layer (Priority 1)

Essential infrastructure needed before feature development.

### Database & Persistence

- [x] Select and integrate database solution (SQLite for simplicity, PostgreSQL for production)
- [ ] Design core database schema
  - [x] Hosts table (machine_id, hostname, OS info, architecture, last seen, linked agents)
  - [x] Software items table (provider config, package identifier, config override, host assignments)
  - [x] Available versions table (software item ID, version, release date, extra metadata)
  - [x] Update history table (host ID, software item ID, from/to version, status, output, initiated_by)
  - [x] Scheduled checks table (software item ID, schedule, last run, next run)
- [x] Implement database migrations system
- [x] Create database access layer with connection pooling
- [x] Add database initialization on controller startup

### Core Data Models

- [ ] Define Rust structs for core entities
  - [ ] Host model with serialization
  - [ ] SoftwareItem model with provider-specific fields
  - [x] Version model with comparison and ordering
  - [x] UpdateRecord model for tracking history
- [x] Implement version comparison logic (semver, custom formats)
- [ ] Create repositories/DAOs for each entity
- [ ] Add validation logic for data models
- [x] Store JSON encoded strings as settings values

### User Authentication & Authorization

- [x] Password-based authentication with Argon2id hashing
  - [x] User model (email, first/last name, password hash, active status)
  - [x] Argon2id with OWASP-recommended parameters (19 MiB, 2 iterations)
  - [x] Stateful session tokens (SHA-256 hashed, 7-day expiry, 30-min sliding window)
  - [x] Bearer token authentication via Authorization header
- [x] RBAC foundation
  - [x] Roles and permissions tables with seeded admin role
  - [x] User-role and role-permission junction tables
  - [x] First registered user automatically gets admin role
  - [x] Permission enum (9 variants: `ViewSettings`, `ManageSettings`, `ViewAgents`, `ManageAgents`,
    `ManageGlobalSettings`, `ViewSoftware`, `ManageSoftware`, `ViewHosts`, `ManageHosts`)
  - [x] `admin` role (all permissions) and `user` role (`view_agents` only)
  - [x] JWT and API responses expose resolved permissions instead of raw role names
  - [x] Permission-based authorization checks on all protected routes
  - [x] Non-first users automatically assigned `user` role
- [x] Auth API endpoints
  - [x] POST /api/v1/auth/register
  - [x] POST /api/v1/auth/login
  - [x] POST /api/v1/auth/logout
  - [x] GET /api/v1/auth/me
- [x] Auth middleware (require_auth with user injection)
- [x] OpenAPI documentation with Swagger UI (optional feature flag)
- [x] Full RBAC permission checking in middleware
- [x] OIDC integration
- [x] Rate limiting on login endpoint
- [ ] Audit logging for security events
- [x] Use JWT token

### Agent Authentication & Security

- [x] Implement mTLS for agent-controller communication
  - [x] Generate client certificates for agents during enrollment
  - [x] Add certificate-based authentication middleware
  - [x] Extract agent identity from client certificates
  - [x] Implement certificate validation on controller
- [x] CA certificate persistence on agents
  - [x] Secure storage location for CA certificate
  - [x] CA certificate pinning to prevent MITM attacks
  - [x] Verification of controller certificate against pinned CA
  - [ ] Fallback handling for CA certificate issues
- [x] CA rotation support
  - [x] Dual-CA validation (active + previous CA in trust bundle)
  - [x] CA bundle update endpoint (`GET /api/v1/pki/ca.crt` returns full bundle)
  - [x] Agent CA update workflow (via `CaBundleUpdated` wire message + hash-based staleness check)
  - [x] Rotation state tracking in database (`ca_fingerprint` column on `agent_certificates` and `service_certificates`)
  - [x] Automatic rotation when managed CA enters 6-month expiry window
  - [x] Background rotation task (24h check interval)
  - [x] Partitioned CRL generation (each CA signs its own CRL)
- [x] Certificate lifecycle management
  - [x] Certificate expiration monitoring (system alerts API + admin UI banners)
  - [x] Automated certificate renewal for agents
  - [x] Server certificate auto-renewal (90-day lifetime, 30-day renewal window)
  - [x] Manual server cert renewal endpoint (`POST /api/v1/settings/renew-server-certificate`)
  - [x] Certificate revocation mechanism (CRL)
  - [x] Revoked certificate checking on each connection

### Wire Protocol

- [ ] Design message types beyond ping/pong
  - [x] Agent registration message
  - [ ] Software inventory report message
  - [x] Version check request/response
  - [x] Update command message (ExecuteUpdate, UpdateStarted, UpdateOutput, UpdateResult)
  - [ ] Status update message
  - [x] Error reporting message
- [x] Implement message serialization/deserialization
- [x] Add message routing in controller
- [x] Implement message handling in agent
- [ ] Add protocol versioning for future compatibility

### Agent Registration & Discovery

- [x] Design agent enrollment flow
  - [x] Initial registration request from agent
  - [x] Controller approval mechanism (auto or manual)
  - [x] Client certificate issuance
  - [x] Agent ID assignment
- [x] Implement agent registration endpoint
- [ ] Create agent inventory tracking
- [x] Add agent heartbeat mechanism
- [x] Implement agent status monitoring (online/offline)
- [x] Add agent metadata collection (OS, architecture, machine_id via HostInfo)

### Provider Trait System

- [x] Refine Provider trait definition
  - [x] Methods for version detection
  - [x] Methods for version checking
  - [x] Methods for update execution
  - [x] Error handling patterns
- [x] Create provider registry system
- [x] Implement provider configuration mechanism
- [x] Add provider capability discovery
- [x] Design provider-specific configuration storage

______________________________________________________________________

## Phase 2: Core Features (Priority 2)

Main functionality that delivers the core value proposition.

### Version Detection (Agent-Side)

- [ ] Implement provider-specific version detection
  - [ ] GitHub releases provider
  - [ ] System package managers (apt, yum, pacman)
  - [x] Docker container provider
  - [x] Proxmox Helper Scripts provider
- [ ] Add caching for detected versions
- [ ] Implement periodic inventory scanning
- [ ] Send version inventory to controller
- [ ] Handle detection errors gracefully

### Version Checking (Controller-Side)

- [ ] Implement provider-specific version checking
  - [x] GitHub releases API integration
  - [ ] Package repository API integration
  - [x] Docker registry API integration (semver tag filtering + digest tracking)
  - [x] Proxmox Helper Scripts repository check
- [x] Add version comparison logic per provider
- [ ] Implement channel support (stable, beta, nightly)
- [ ] Cache available versions with TTL
- [x] Handle API rate limiting
- [ ] Add retry logic for failed checks

### Provider Implementations

- [x] Complete GitHub releases provider
  - [x] Asset detection and selection
  - [x] Release notes extraction
  - [x] Pre-release handling
- [ ] Implement Proxmox Helper Scripts provider
  - [x] Script version detection
  - [x] Script update mechanism
  - [x] Software discovery (`discover_software()`)
  - [ ] Script integrity verification
- [ ] Add system package manager provider
  - [ ] Support for multiple package managers
  - [ ] Package dependency handling
- [x] Create Docker container provider
  - [x] Image version checking (semver tag filtering + digest change detection)
  - [x] Registry authentication (anonymous, basic, bearer with OCI token flow)
  - [x] Multi-registry support (Docker Hub, GHCR, any OCI-compliant registry)

### Update Execution

- [x] Design update execution framework
  - [x] Pre-update hooks (from provider config + config_override)
  - [x] Update steps execution (provider-specific via ExecuteUpdate message)
  - [x] Post-update hooks (from provider config + config_override)
  - [ ] Rollback triggers
- [x] Implement update state machine
  - [x] Pending → In Progress → Completed/Failed
  - [x] State persistence (update_history table)
- [x] Add update progress reporting (UpdateOutput streaming)
- [x] Implement update logging (output accumulated in update_history)
- [ ] Handle update failures and retries
- [x] Add update timeout handling (configurable per-update)
- [ ] Support updating agent and controller

### Scheduling System

- [x] Design scheduling architecture
  - [x] Cron-like schedule expressions
  - [x] Next run time calculation
  - [x] Schedule persistence
- [x] Implement scheduler service
  - [x] Background task runner
  - [x] Scheduled check execution
  - [x] Schedule conflict resolution
- [ ] Add per-software-item schedule configuration
- [x] Implement global schedule defaults
- [x] Add schedule enable/disable functionality
- [x] Support manual trigger overrides

### Software Autodiscovery

- [x] Design autodiscovery architecture
  - [x] Event-driven trigger on new host registration
  - [x] `discovery_state` field on `software_items` (`pending` / `approved` / null)
  - [x] `autodiscovery_ignores` table for permanent suppression
  - [x] Partial unique index on `provider_configs(tenant_id, name)` for race-safe auto-creation
- [x] Implement `DiscoverSoftware` / `DiscoveryResults` wire messages
- [x] Agent-core `handle_discover_software()` shared implementation
- [x] Regular agent and SSH agent support for `DiscoverSoftware`
- [x] Controller `DiscoveryResults` handler with auto-creation of `ProviderConfig` records
- [x] `ProviderType::supports_discovery()` capability check
- [x] Homebrew: discover all (formulae + casks) when `package_type = None`; set `extra` metadata
- [x] Proxmox Helper Scripts: allow discovery with empty `script_url`
- [x] REST API for autodiscovery
  - [x] `POST /api/v1/software-items/{id}/approve`
  - [x] `POST /api/v1/hosts/{id}/discover`
  - [x] `DELETE /api/v1/hosts/{id}/discovered`
  - [x] `POST /api/v1/provider-configs/{id}/discover`
  - [x] `DELETE /api/v1/provider-configs/{id}/discovered`
  - [x] `GET /api/v1/autodiscovery/ignores`
  - [x] `POST /api/v1/autodiscovery/ignores`
  - [x] `DELETE /api/v1/autodiscovery/ignores/{id}`
  - [x] `DELETE /api/v1/software-items/{id}?ignore=true`
- [x] Version check scheduler excludes `pending` items

### Concurrency Control

- [ ] Implement update locking mechanism
  - [ ] Per-host update locks
  - [ ] Global concurrent update limits
  - [ ] Lock timeout handling
- [ ] Add update queue management
- [ ] Implement priority queue for updates
- [x] Handle concurrent version checks efficiently
- [ ] Add resource-based throttling

### User convenience

- [x] Reverse proxy support and documentation
  - [x] Agent identity forwarded via headers (info + PEM) with CA verification
  - [x] Header stripping for non-proxy clients
  - [x] External base URL resolution for OIDC/device auth redirects
  - [x] Agent-id-only lookup when cert serial unavailable
  - [x] Deployment guides for Traefik, Caddy, Nginx, NPM, Envoy, HAProxy
- [x] SIGHUP support for graceful restart (agent and MQTT service exit cleanly for external restart)
- [ ] SIGHUP support for graceful reloading (controller)
  - [ ] All possible settings are reloaded
  - [ ] Agents should not be disconnected during reload (but there should be a "please reconnect" message)
- [x] Add support for a graceful restart (via `SO_REUSEPORT`)
  - [x] `--reuseport` flag enables `SO_REUSEPORT` socket option
  - [x] `--takeover-from <PID>` signals old process to begin graceful shutdown
  - [x] `--shutdown-timeout-secs` configures drain timeout (default: 30)
  - [x] `ServerRestarting` wire message notifies agents (scattered to avoid thundering herd)
  - [x] Background tasks support `CancellationToken` for clean shutdown
- [x] Embedded frontend (`embed-frontend` feature): compile the SvelteKit SPA into the controller binary via `rust-embed` for single-binary deployment
- [ ] Certificate renewal on the controller through API must enable that certificate for new connections

______________________________________________________________________

## Phase 3: User Interfaces (Priority 3)

Ways users interact with the system.

### Web API

- [ ] Expand REST API beyond MQTT connection status
  - [x] List hosts endpoint
  - [x] List software items endpoint
  - [x] Get software item details endpoint
  - [x] Trigger version check endpoint (per-item and per-item-per-host)
  - [x] Trigger update endpoint (POST /api/v1/software-items/{id}/hosts/{host_id}/update)
  - [x] Get update history endpoint
  - [ ] Get system status endpoint
- [x] Add API authentication
- [x] Implement API rate limiting
- [x] Add API documentation (OpenAPI/Swagger)
- [x] Add WebSocket endpoint for real-time updates

### Web UI

- [x] Create basic web UI framework
  - [x] Choose framework (Svelte, React, Vue)
  - [x] Set up build system
  - [x] Implement API client
- [ ] Build dashboard view
  - [ ] System overview statistics
  - [ ] Recent update activity
  - [ ] Alert/notification display
- [x] Implement host list view
  - [ ] Sortable/filterable table
  - [ ] Host detail drill-down
  - [x] Host status indicators
- [ ] Create software list view
  - [ ] Grouped by host or provider
  - [ ] Current vs. available version display
  - [ ] Update action buttons
- [ ] Add update trigger UI
  - [ ] Manual update initiation
  - [ ] Update confirmation dialogs
  - [ ] Progress indicators
- [ ] Implement schedule configuration UI
  - [ ] Visual schedule builder
  - [ ] Enable/disable schedules
  - [ ] Test schedule expressions
- [ ] Add settings/configuration UI
  - [ ] Provider configurations
  - [ ] Global settings
  - [ ] User management

### CLI Tool

- [x] Device authorization login flow (RFC 8628-style)
  - [x] CLI requests device code from controller
  - [x] Opens browser for user approval
  - [x] Polls for authorization completion
  - [x] Stores API token locally on success
- [x] Auth status and API token management (`auth status`, `auth token create/list/revoke`)
- [x] Design CLI command structure
  - [x] `uptrakit hosts` - list and show hosts
  - [x] `uptrakit software-items` - list and show software items
  - [x] `uptrakit check` - trigger version checks (all, item)
  - [x] `uptrakit update` - trigger update
  - [x] `uptrakit history` - view update history (list with filters, show)
  - [x] `uptrakit services` - list, show, approve, reject, remove, merge, update services
  - [x] `uptrakit scheduler` - list, show, and trigger scheduled tasks
  - [x] `uptrakit settings` - settings management
  - [ ] `uptrakit status` - system status
  - [ ] `uptrakit agent` - various commands proxied to the agent. Plus
    `uptrakit agent install` to install the agent locally
  - [ ] `uptrakit controller` - various commands proxied to the controller. Plus
    `uptrakit agent install` to install the agent locally
- [x] Implement CLI commands (hosts, software-items, check, update, history, scheduler)
- [x] Add output formatting (table, JSON, YAML)
- [x] Implement filtering and query options (history filters by host, software item, status)
- [ ] Add interactive mode for confirmations
- [ ] Support configuration file for CLI

### MQTT/Home Assistant Integration

- [x] Separate MQTT binary with multi-instance support and lease-based tenant distribution
- [x] MQTT service communicates with controller via WebSocket/mTLS (no direct DB access)
  - [x] Wire protocol messages (`ServiceMessage`, `ControllerMessage` — unified for all service types)
  - [x] MQTT service enrollment flow (TOFU CA pinning, anonymous enrollment, certificate issuance)
  - [x] Controller WebSocket endpoint (`/api/v1/ws/service`) with 3-state handler
  - [x] Controller-side lease coordinator (centralized tenant assignment)
  - [x] Unified ServiceConnectionRegistry (track connected service instances)
  - [x] Push-based tenant config (controller pushes config changes to instances)
  - [x] Settings-based MQTT enrollment tokens (managed via unified services API)
  - [x] Unified REST API for service management (`/api/v1/services` — list, approve, reject, deactivate)
  - [x] Unified REST API for service enrollment tokens (`/api/v1/services/enrollment-tokens` — create, list, delete)
  - [x] Unified database entity (`services` table with `service_type` column, `service_certificates`)
- [ ] Implement MQTT auto-discovery for Home Assistant
  - [ ] Device discovery messages
  - [ ] Entity discovery (sensors, binary sensors, buttons)
- [ ] Publish software version sensors
  - [ ] Current version attribute
  - [ ] Available version attribute
  - [ ] Update available binary sensor
- [ ] Implement update command handling via MQTT
  - [ ] Listen to Home Assistant update commands
  - [ ] Publish update status
  - [ ] Publish update progress
- [ ] Add configurable MQTT topics
- [x] Implement MQTT connection resilience
- [ ] Add MQTT authentication support

______________________________________________________________________

## Phase 4: SSH Agent (Priority 3)

A new agent type that communicates with the controller over WebSocket (like the regular agent) but executes version detection and updates on remote
hosts over SSH instead of locally. This is a separate crate (`crates/core/agent-ssh/`) — not integrated into the existing agent or the controller.

Use case: managing hosts where installing a persistent daemon is impractical (appliances, locked-down systems, minimal containers, or environments
where outbound-only WebSocket is not feasible but inbound SSH is available).

### New Crate & Architecture

- [x] Create `crates/core/agent-ssh/` crate (`uptrakit-agent-ssh` binary)
- [x] Reuse the existing wire protocol and WebSocket transport to the controller
- [x] The SSH agent manages one or more remote hosts — each appears as a separate host in the controller
- [x] CLI subcommands for local SSH host management (`host add/list/show/update/remove/bootstrap`)
- [ ] Configuration file defines target hosts (hostname, port, username, key path, sudo setup)
- [ ] Clearly separate SSH transport logic from provider execution logic so providers remain transport-agnostic

### SSH Transport Layer

- [x] Password and key-based authentication (password auth supported for bootstrap; key-based for ongoing use)
- [x] SSH agent forwarding support (automatic `SSH_AUTH_SOCK` detection as fallback when no explicit auth is given)
- [x] Dual-mode `--auth-password` (accepts optional inline value or prompts interactively)
- [x] Support Ed25519 (preferred), RSA, and ECDSA keys (auto-detection from PEM content)
- [x] Strict host key verification (TOFU with persisted fingerprints, or pre-seeded fingerprints)
- [x] Reject connections on host key mismatch — never silently accept
- [ ] Connection pooling and multiplexing (reuse connections across checks and updates for the same host)
- [x] Configurable connection and command timeouts (30s default for bootstrap)
- [ ] Jump host / bastion support for reaching hosts behind NAT or firewalls
- [x] Support for custom SSH ports per host (configurable via `--port` in `host add/update`, or via the target string in `host bootstrap`)

### Remote Execution

- [ ] Run provider detection commands on the remote host over SSH and parse output locally
- [ ] Execute updates via sudo on the remote host (same sudo allowlist model as the regular agent)
- [x] Stream command output back for progress reporting
- [ ] Enforce per-host update locking (no concurrent updates to the same host)
- [ ] Timeout and kill long-running remote commands
- [ ] Handle connection drops mid-command gracefully (report failure, do not leave orphan processes)

### Provider Compatibility

- [ ] All agent-side providers must work over SSH (same commands, different transport)
- [x] Provider trait uses a transport abstraction (`CommandExecutor`) so the same provider logic works for both local and SSH execution
- [x] Implement `SshCommandExecutor` that executes commands over SSH connections
- [ ] Provider-level capability flag indicating SSH compatibility

### Security Considerations

- [x] Least-privilege SSH user on each managed host (e.g. `uptrakit`, created by bootstrap; mirrors the regular agent model)
- [ ] Sudo allowlist identical to regular agent: only specific update commands, NOPASSWD, no shell access
  (bootstrap creates `NOPASSWD: ALL` — manual restriction recommended)
- [x] No shell injection: remote commands constructed from validated inputs using `shell_escape()`, never string-interpolated
- [x] SSH private keys stored on the machine running the SSH agent — never sent to the controller or exposed in API responses (encrypted at rest via `EncryptedString`)
- [ ] Audit trail: log every SSH session (host, user, command, timestamp, exit code) without capturing secrets or key material
- [ ] Limit concurrent SSH sessions per host and globally to prevent resource exhaustion on both the SSH agent and the remote hosts
- [ ] Host key fingerprints should be verifiable through the controller UI (display, not edit)

### Configuration & Management

- [ ] Config file format for defining target hosts and their SSH credentials
- [ ] CLI flags for overrides (key path, known_hosts path, concurrency limits)
- [ ] Health checks: periodic SSH connectivity test to each managed host, reported to controller
- [ ] Controller UI and API: SSH-managed hosts appear alongside regular agents with a transport type indicator
- [ ] MQTT/Home Assistant entities work identically regardless of agent transport

______________________________________________________________________

## Phase 5: Provider Ecosystem (Priority 3-4)

Expanding the provider system with more integrations.

### Additional Providers

- [ ] Implement custom script provider
  - [ ] Script definition format
  - [ ] Script execution sandbox
  - [ ] Script output parsing
- [ ] Add pip/PyPI provider
- [ ] Add npm/Node.js provider
- [ ] Add Cargo/Rust provider
- [ ] Add Flatpak provider
- [ ] Add Snap provider
- [ ] Add AppImage provider
- [x] Add Homebrew provider (macOS)
- [ ] Add Chocolatey provider (Windows)

### Provider Framework

- [ ] Create provider testing framework
  - [ ] Mock version sources
  - [ ] Test harness for providers
- [ ] Add provider validation tools
- [ ] Implement provider hot-reloading
- [ ] Create provider marketplace/registry concept
- [ ] Add provider versioning

### Documentation

- [ ] Write provider development guide
  - [ ] Trait implementation tutorial
  - [ ] Best practices
  - [ ] Testing guidelines
- [ ] Create provider examples
  - [ ] Simple provider template
  - [ ] Complex provider example
- [ ] Document provider API reference
- [ ] Add troubleshooting guide for providers

______________________________________________________________________

## Phase 6: Advanced Features (Priority 4)

Polish and additional capabilities for production use.

### Multi-Channel Support

- [ ] Implement channel abstraction
  - [ ] Stable, beta, nightly, custom channels
  - [ ] Per-software-item channel selection
  - [ ] Channel switching rules
- [ ] Add channel-aware version checking
- [ ] Implement channel migration workflows
- [ ] Add channel configuration UI

### Rollback Capabilities

- [ ] Design rollback mechanism
  - [ ] Snapshot creation before updates
  - [ ] Rollback trigger conditions
  - [ ] Rollback execution
- [ ] Implement rollback for supported providers
- [ ] Add rollback history tracking
- [ ] Create rollback UI
- [ ] Add automatic rollback on failure

### Update Batching & Orchestration

- [ ] Design batch update system
  - [ ] Batch definition (groups of updates)
  - [ ] Batch execution strategies (sequential, parallel)
  - [ ] Batch failure handling
- [ ] Implement update dependencies
  - [ ] Update A must complete before update B
  - [ ] Cross-host dependencies
- [ ] Add batch progress tracking
- [ ] Create batch update UI
- [ ] Implement canary deployment patterns

### Real-Time Frontend Updates

- [ ] **Real-time frontend updates** — Add a WebSocket or SSE endpoint for the admin UI
  so service status changes, MQTT connection status, and version check completions are
  pushed to the browser without manual polling.

### Notification System

- [ ] Design notification architecture
  - [ ] Notification types (email, webhook, MQTT, push)
  - [ ] Notification triggers (updates available, completed, failed)
  - [ ] Notification templates
- [ ] Implement notification providers
  - [ ] Email notifications
  - [ ] Webhook notifications
  - [ ] Slack integration
  - [ ] Discord integration
- [ ] Add notification configuration UI
- [ ] Implement notification filtering/preferences
- [ ] Add notification history
- [ ] Support actionable notifications

### Update Windows

- [ ] Implement maintenance window concept
  - [ ] Time-based windows
  - [ ] Day-of-week restrictions
  - [ ] Blackout periods
- [ ] Add window validation for scheduled updates
- [ ] Implement update queuing outside windows
- [ ] Create window configuration UI
- [ ] Support timezone handling

______________________________________________________________________

## Phase 7: Security Enhancements (Priority 2-3)

Comprehensive security hardening.

### mTLS Implementation Details

- [x] Automated client certificate issuance
  - [x] Certificate signing request (CSR) handling
  - [x] Automated CA signing
  - [x] Certificate delivery to agents
- [x] Certificate revocation mechanism
  - [x] CRL generation and distribution (per-CA partitioned CRLs)
  - [x] OCSP responder implementation
  - [x] Revocation checking on agent connections
- [x] Certificate expiration handling
  - [x] Expiration monitoring (system alerts API for admin UI)
  - [x] Automated renewal workflow (agent certs + server cert auto-renewal)
  - [x] Pre-expiration notifications (admin alert banners)

### CA Management

- [ ] CA certificate backup and recovery
  - [ ] Automated CA backup
  - [ ] Secure backup storage
  - [ ] Recovery procedures documentation
- [x] CA rotation automation
  - [x] Rotation scheduling system (centralised scheduler, cron-based)
  - [x] Automated rotation execution (6-month window before expiry)
  - [ ] Rollback capability for failed rotations
- [x] Multi-CA validation support
  - [x] Trust store management (active + previous CA bundle)
  - [x] CA priority handling (active CA signs new certs; both CAs trusted)
  - [x] Gradual CA migration (agents auto-fetch updated bundle)

### Agent Authentication

- [x] Certificate-based agent identity
  - [x] Identity extraction from certificates
  - [x] Identity-to-agent mapping
  - [x] Identity persistence
- [ ] Agent authorization policies
  - [ ] Role-based access control
  - [ ] Per-agent permissions
  - [ ] Policy enforcement points
- [x] Secure agent enrollment flow
  - [x] Enrollment token generation
  - [x] Token expiration and validation
  - [x] Enrollment approval workflow

### Audit Logging

- [ ] Security event logging
  - [ ] Authentication attempts (success/failure)
  - [ ] Authorization decisions
  - [ ] Certificate operations (issuance, revocation, renewal)
  - [ ] CA operations (rotation, backup)
  - [ ] Configuration changes
- [ ] Tamper-evident log storage
  - [ ] Log signing
  - [ ] Log integrity verification
  - [ ] Immutable log storage
- [ ] Log management
  - [ ] Log rotation policies
  - [ ] Log retention policies
  - [ ] Log archival
  - [ ] Log search and analysis

### Additional Security

- [x] Implement rate limiting for all endpoints
- [x] Add brute force protection
- [ ] Implement security headers
- [x] Add input validation and sanitization
- [x] Implement secrets management
  - [x] Secure credential storage (AES-256-GCM encryption at rest via `EncryptedString`, mandatory in production; dev-only `--allow-plaintext-secrets`
    available)
  - [ ] Credential rotation
  - [ ] Vault integration
- [ ] Add security scanning to CI/CD
  - [ ] Dependency vulnerability scanning
  - [ ] Static code analysis
  - [ ] Container image scanning

______________________________________________________________________

## Phase 8: Quality & Reliability (Ongoing)

Ensuring robustness and maintainability.

### Testing

- [ ] Expand unit test coverage
  - [ ] Target 80%+ coverage for core logic
  - [ ] Test error handling paths
  - [ ] Test edge cases
- [ ] Add integration tests
  - [ ] Agent-controller communication
  - [ ] Database operations
  - [ ] Provider implementations
  - [ ] End-to-end update workflows
  - [x] OCSP revocation checking with reverse proxies (Nginx `ssl_ocsp leaf`). Uses a standalone test OCSP responder (`ocsp_responder.rs`) reachable
    from Docker via `host.docker.internal`. CRL tests exist for Nginx, HAProxy, and Envoy; OCSP test covers Nginx.
- [ ] Implement load testing
  - [ ] Many agents scenario
  - [ ] Concurrent update scenario
  - [ ] High-frequency check scenario
- [ ] Add chaos testing
  - [ ] Network failure scenarios
  - [ ] Database failure scenarios
  - [ ] Agent crash scenarios
- [ ] Create test fixtures and mocks
  - [ ] Mock providers
  - [ ] Mock version sources
  - [ ] Test data generators

### Error Recovery

- [x] Implement connection retry logic with exponential backoff
- [ ] Add graceful degradation for partial failures
- [ ] Implement circuit breaker pattern for external services
- [ ] Add automatic recovery from transient errors
- [ ] Implement idempotent operations
- [ ] Add operation replay capabilities

### Performance Optimization

- [ ] Profile and optimize hot paths
- [ ] Implement efficient caching strategies
- [ ] Optimize database queries
  - [ ] Add indexes
  - [ ] Query optimization
  - [ ] Connection pooling tuning
- [ ] Reduce memory footprint
- [ ] Optimize agent-controller communication
  - [ ] Message batching
  - [ ] Compression
- [ ] Add performance monitoring
  - [ ] Request timing
  - [ ] Database query timing
  - [ ] Resource usage metrics

### Reliability

- [ ] Implement health check endpoints
- [ ] Add readiness probes
- [x] Implement graceful shutdown (agent waits for in-flight updates before disconnecting)
- [ ] Add state recovery on restart
- [ ] Implement data integrity checks
- [ ] Add automatic backup and restore

______________________________________________________________________

## Phase 9: Documentation & Operations (Ongoing)

Making the system usable and maintainable.

### API Documentation

- [ ] Generate OpenAPI/Swagger specification
- [ ] Document all REST endpoints
- [ ] Add request/response examples
- [ ] Document WebSocket messages
- [x] Create API client libraries
  - [x] `uptrakit-openapi-client` typed HTTP client crate (used by CLI)

### User Documentation

- [ ] Write getting started guide
- [ ] Create installation guide
  - [ ] Controller installation
  - [ ] Agent installation
  - [ ] Configuration walkthrough
- [ ] Write user manual
  - [ ] Web UI guide
  - [ ] CLI guide
  - [ ] MQTT/Home Assistant integration guide
- [ ] Create FAQ
- [ ] Add troubleshooting guide
- [ ] Record video tutorials

### Security Documentation

- [ ] Write mTLS setup guide
  - [ ] CA certificate generation
  - [ ] Agent certificate provisioning
  - [ ] Certificate renewal procedures
- [ ] Document CA rotation procedures
  - [ ] Pre-rotation checklist
  - [ ] Rotation execution steps
  - [ ] Post-rotation verification
  - [ ] Rollback procedures
- [x] Create certificate management guide
  - [x] Certificate lifecycle overview
  - [x] Revocation procedures
  - [ ] Backup and recovery
- [ ] Document agent authentication
  - [ ] Enrollment workflow
  - [ ] Identity management
  - [ ] Authorization policies
- [x] Write security best practices guide
  - [x] Secure deployment recommendations
  - [x] Network security
  - [x] Secret management
  - [ ] Audit logging configuration

### Deployment Documentation

- [ ] Write deployment guide
  - [ ] System requirements
  - [ ] Network requirements
  - [ ] Security considerations
- [ ] Create Docker deployment guide
- [ ] Create Kubernetes deployment guide
- [ ] Document systemd service setup
- [ ] Add upgrade guide
- [ ] Create backup and restore guide

### Contributor Documentation

- [x] Write [CONTRIBUTING.md](CONTRIBUTING.md)
- [x] Document development setup
- [x] Create architecture documentation
- [ ] Document testing strategy
- [ ] Create PR template and guidelines

______________________________________________________________________

## Phase 10: Project Infrastructure (Ongoing)

Development and release automation.

### CI/CD

- [ ] Expand GitHub Actions workflows
  - [ ] Multi-platform builds
  - [ ] Cross-compilation
  - [ ] Test execution
  - [ ] Coverage reporting
- [ ] Add automated security scanning
  - [ ] cargo-audit integration
  - [ ] cargo-deny integration
  - [ ] SAST tools
- [ ] Implement automated dependency updates
- [ ] Add automated changelog generation
- [ ] Implement semantic versioning automation

### Release Automation

- [ ] Automate binary releases
  - [ ] Multi-platform binaries
  - [ ] Checksums and signatures
- [ ] Automate container image builds
  - [ ] Multi-arch images
  - [ ] Image scanning
  - [ ] Registry publishing
- [ ] Create release checklist
- [ ] Automate release notes generation
- [ ] Implement version bumping automation

### Monitoring & Observability

- [ ] Implement structured logging
  - [ ] JSON log output
  - [ ] Log levels
  - [ ] Correlation IDs
- [ ] Add metrics collection
  - [ ] Prometheus metrics
  - [ ] Custom metrics
  - [ ] Metric dashboards
- [ ] Implement tracing
  - [ ] Distributed tracing
  - [ ] OpenTelemetry integration
- [ ] Create monitoring dashboards
  - [ ] System health dashboard
  - [ ] Performance dashboard
  - [ ] Security dashboard
- [ ] Add alerting
  - [ ] Certificate expiration alerts
  - [ ] CA rotation status alerts
  - [ ] Agent authentication failure alerts
  - [ ] Update failure alerts
  - [ ] System health alerts

### Developer Experience

- [ ] Improve local development setup
  - [ ] Development containers
  - [ ] Mock services
  - [ ] Hot reloading
- [ ] Create debugging tools
- [ ] Add development documentation
- [ ] Implement consistent error messages
- [ ] Add development helpers and scripts

______________________________________________________________________

## Future Considerations

Items to consider for future versions but not currently prioritized:

- [x] Multi-tenant support (database infrastructure, single-tenant mode)
  - [x] Tenants table with default tenant seeding
  - [x] `tenant_id` FK on all scoped tables (agents, services, hosts, provider_configs, software_items, oidc_providers, user_roles, settings)
  - [x] TenantContext extractor (X-Tenant-Id header with default tenant fallback)
  - [x] Global vs tenant-scoped settings (SettingKey::is_global())
  - [x] All route handlers updated for tenant awareness
  - [ ] Tenant management API (CRUD)
  - [ ] Multi-tenant JWT (per-tenant permissions)
  - [x] Tenant-aware MQTT (separate `uptrakit-mqtt` binary with per-tenant lease-based distribution via unified `/api/v1/ws/service` WebSocket
    endpoint)
  - [ ] Tenant switching UI
  - [ ] API token scoping per tenant
- [ ] Agent clustering
- [ ] High availability for controller (auth flow stores are now DB-backed and HA-ready; settings cache uses version-gated periodic reload for
  cross-instance consistency; CRL rebuilds propagate cross-instance via `revocation_version` polling; cross-controller push notification delivery via
  outbox pattern is implemented; JWT signing key is DB-backed and shared across instances; master key mismatch detection via startup verification
  token is implemented; token denylist is per-instance — DB-backed HA sync deferred)
- [ ] Update preview/dry-run mode
- [ ] Cost tracking for cloud-based updates
- [ ] Compliance reporting (update audit trails)
- [ ] Mobile app
- [ ] Browser extensions for quick status checks
- [ ] Terraform/Ansible provider integrations
- [ ] GitOps integration for configuration
- [ ] Machine learning for update risk prediction
- [ ] A/B testing framework for updates
- [ ] Custom metrics and alerting DSL

______________________________________________________________________

## Deferred Dependency Upgrades

Dependencies that could not be upgraded in the current pass and require upstream changes before they can move forward.

- [ ] **`strum` 0.27 → 0.28** — blocked by `sea-orm rc.x`, which pins `strum = "^0.27"` (excludes 0.28.x). Workspace crates that
  derive both `strum::EnumIter` and `sea_orm::DeriveActiveEnum` would have two incompatible `Iterable` trait versions. Revisit once
  sea-orm ships a release that moves its own strum pin to `^0.28`.

- [ ] **`rand` 0.9 → 0.10** — blocked by `russh`, `rsa`, and `crypto-bigint`, which depend on `rand_core = "0.10.0-rc-3"`. Cargo
  cannot unify this pre-release with the stable `rand_core ^0.10.0` that `rand 0.10.0` requires. Revisit once the russh/RustCrypto
  stack stabilises on a non-RC release of `rand_core 0.10`.

- [ ] **`der` 0.7 / `const-oid` 0.9 / `spki` 0.7 / `x509-cert` 0.2 / `x509-ocsp` 0.2** — blocked by `rcgen 0.14` and `x509-ocsp 0.2`,
  which both require `der ^0.7`. Bumping `der` to 0.8 while they remain on 0.7 introduces two incompatible versions and causes type
  collisions in the PKI/OCSP code. Revisit once `rcgen` and `x509-ocsp` release versions compatible with `der ^0.8`.

______________________________________________________________________

## Notes

- This roadmap is a living document and should be updated as priorities shift
- Items can be reordered based on user feedback and project needs
- Some items may be split into smaller tasks during implementation
- Cross-phase dependencies should be carefully managed
- Security and quality items should be addressed continuously, not just in their dedicated phases
