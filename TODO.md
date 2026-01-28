# Uptrakit Project Roadmap

## About This Document

This roadmap tracks the development of Uptrakit, a self-hosted software update monitoring and management system. It organizes planned work into logical phases with clear priorities and dependencies.

**Current Status**: Early MVP stage with foundational infrastructure in place.

**Key Documentation**:

- [README.md](README.md) - Project overview and architecture

**How to Use This Roadmap**:

- Items are organized by priority and dependencies
- Check off items as they're completed
- Review regularly to adjust priorities based on project needs
- Foundation items should generally be completed before moving to higher-level features

---

## Phase 1: Foundation Layer (Priority 1)

Essential infrastructure needed before feature development.

### Database & Persistence

- [x] Select and integrate database solution (SQLite for simplicity, PostgreSQL for production)
- [ ] Design core database schema
  - [ ] Hosts table (agent ID, hostname, last seen, status)
  - [ ] Software items table (host ID, provider, package name, current version)
  - [ ] Available versions table (software item ID, version, release date, channels)
  - [ ] Update history table (software item ID, from version, to version, timestamp, status)
  - [ ] Scheduled checks table (software item ID, schedule, last run, next run)
- [x] Implement database migrations system
- [x] Create database access layer with connection pooling
- [x] Add database initialization on controller startup

### Core Data Models

- [ ] Define Rust structs for core entities
  - [ ] Host model with serialization
  - [ ] SoftwareItem model with provider-specific fields
  - [ ] Version model with comparison and ordering
  - [ ] UpdateRecord model for tracking history
- [ ] Implement version comparison logic (semver, custom formats)
- [ ] Create repositories/DAOs for each entity
- [ ] Add validation logic for data models

### Authentication & Security

- [ ] Implement mTLS for agent-controller communication
  - [ ] Generate client certificates for agents during enrollment
  - [ ] Add certificate-based authentication middleware
  - [ ] Extract agent identity from client certificates
  - [ ] Implement certificate validation on controller
- [ ] CA certificate persistence on agents
  - [ ] Secure storage location for CA certificate
  - [ ] CA certificate pinning to prevent MITM attacks
  - [ ] Verification of controller certificate against pinned CA
  - [ ] Fallback handling for CA certificate issues
- [ ] CA rotation support
  - [ ] Design dual-CA validation mechanism
  - [ ] Implement CA certificate update endpoint
  - [ ] Create agent CA update workflow
  - [ ] Add rotation state tracking in database
  - [ ] Implement automatic transition after rotation period
- [ ] Certificate lifecycle management
  - [ ] Certificate expiration monitoring
  - [ ] Automated certificate renewal for agents
  - [ ] Certificate revocation mechanism (CRL or OCSP)
  - [ ] Revoked certificate checking on each connection

### Wire Protocol

- [ ] Design message types beyond ping/pong
  - [ ] Agent registration message
  - [ ] Software inventory report message
  - [ ] Version check request/response
  - [ ] Update command message
  - [ ] Status update message
  - [ ] Error reporting message
- [ ] Implement message serialization/deserialization
- [ ] Add message routing in controller
- [ ] Implement message handling in agent
- [ ] Add protocol versioning for future compatibility

### Agent Registration & Discovery

- [ ] Design agent enrollment flow
  - [ ] Initial registration request from agent
  - [ ] Controller approval mechanism (auto or manual)
  - [ ] Client certificate issuance
  - [ ] Agent ID assignment
- [ ] Implement agent registration endpoint
- [ ] Create agent inventory tracking
- [ ] Add agent heartbeat mechanism
- [ ] Implement agent status monitoring (online/offline)
- [ ] Add agent metadata collection (OS, architecture, capabilities)

### Provider Trait System

- [ ] Refine Provider trait definition
  - [ ] Methods for version detection
  - [ ] Methods for version checking
  - [ ] Methods for update execution
  - [ ] Error handling patterns
- [ ] Create provider registry system
- [ ] Implement provider configuration mechanism
- [ ] Add provider capability discovery
- [ ] Design provider-specific configuration storage

---

## Phase 2: Core Features (Priority 2)

Main functionality that delivers the core value proposition.

### Version Detection (Agent-Side)

- [ ] Implement provider-specific version detection
  - [ ] GitHub releases provider
  - [ ] System package managers (apt, yum, pacman)
  - [ ] Docker container provider
  - [ ] Proxmox Helper Scripts provider
- [ ] Add caching for detected versions
- [ ] Implement periodic inventory scanning
- [ ] Send version inventory to controller
- [ ] Handle detection errors gracefully

### Version Checking (Controller-Side)

- [ ] Implement provider-specific version checking
  - [ ] GitHub releases API integration
  - [ ] Package repository API integration
  - [ ] Docker registry API integration
  - [ ] Proxmox Helper Scripts repository check
- [ ] Add version comparison logic per provider
- [ ] Implement channel support (stable, beta, nightly)
- [ ] Cache available versions with TTL
- [ ] Handle API rate limiting
- [ ] Add retry logic for failed checks

### Provider Implementations

- [ ] Complete GitHub releases provider
  - [ ] Asset detection and selection
  - [ ] Release notes extraction
  - [ ] Pre-release handling
- [ ] Implement Proxmox Helper Scripts provider
  - [ ] Script version detection
  - [ ] Script update mechanism
  - [ ] Script integrity verification
- [ ] Add system package manager provider
  - [ ] Support for multiple package managers
  - [ ] Package dependency handling
- [ ] Create Docker container provider
  - [ ] Image version checking
  - [ ] Registry authentication
  - [ ] Multi-registry support

### Update Execution

- [ ] Design update execution framework
  - [ ] Pre-update hooks
  - [ ] Update steps execution
  - [ ] Post-update verification
  - [ ] Rollback triggers
- [ ] Implement update state machine
  - [ ] Pending → In Progress → Completed/Failed
  - [ ] State persistence
- [ ] Add update progress reporting
- [ ] Implement update logging
- [ ] Handle update failures and retries
- [ ] Add update timeout handling

### Scheduling System

- [ ] Design scheduling architecture
  - [ ] Cron-like schedule expressions
  - [ ] Next run time calculation
  - [ ] Schedule persistence
- [ ] Implement scheduler service
  - [ ] Background task runner
  - [ ] Scheduled check execution
  - [ ] Schedule conflict resolution
- [ ] Add per-software-item schedule configuration
- [ ] Implement global schedule defaults
- [ ] Add schedule enable/disable functionality
- [ ] Support manual trigger overrides

### Concurrency Control

- [ ] Implement update locking mechanism
  - [ ] Per-host update locks
  - [ ] Global concurrent update limits
  - [ ] Lock timeout handling
- [ ] Add update queue management
- [ ] Implement priority queue for updates
- [ ] Handle concurrent version checks efficiently
- [ ] Add resource-based throttling

---

## Phase 3: User Interfaces (Priority 3)

Ways users interact with the system.

### Web API

- [ ] Expand REST API beyond MQTT connection status
  - [ ] List hosts endpoint
  - [ ] List software items endpoint
  - [ ] Get software item details endpoint
  - [ ] Trigger version check endpoint
  - [ ] Trigger update endpoint
  - [ ] Get update history endpoint
  - [ ] Get system status endpoint
- [ ] Add API authentication
- [ ] Implement API rate limiting
- [ ] Add API documentation (OpenAPI/Swagger)
- [ ] Add WebSocket endpoint for real-time updates

### Web UI

- [ ] Create basic web UI framework
  - [ ] Choose framework (Svelte, React, Vue)
  - [ ] Set up build system
  - [ ] Implement API client
- [ ] Build dashboard view
  - [ ] System overview statistics
  - [ ] Recent update activity
  - [ ] Alert/notification display
- [ ] Implement host list view
  - [ ] Sortable/filterable table
  - [ ] Host detail drill-down
  - [ ] Host status indicators
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

- [ ] Design CLI command structure
  - [ ] `uptrakit-cli hosts` - list hosts
  - [ ] `uptrakit-cli software` - list software
  - [ ] `uptrakit-cli check` - trigger version check
  - [ ] `uptrakit-cli update` - trigger update
  - [ ] `uptrakit-cli history` - view update history
  - [ ] `uptrakit-cli status` - system status
- [ ] Implement CLI commands
- [ ] Add output formatting (table, JSON, YAML)
- [ ] Implement filtering and query options
- [ ] Add interactive mode for confirmations
- [ ] Support configuration file for CLI

### MQTT/Home Assistant Integration

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
- [ ] Implement MQTT connection resilience
- [ ] Add MQTT authentication support

---

## Phase 4: Provider Ecosystem (Priority 3-4)

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
- [ ] Add Homebrew provider (macOS)
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

---

## Phase 5: Advanced Features (Priority 4)

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

### Update Windows

- [ ] Implement maintenance window concept
  - [ ] Time-based windows
  - [ ] Day-of-week restrictions
  - [ ] Blackout periods
- [ ] Add window validation for scheduled updates
- [ ] Implement update queuing outside windows
- [ ] Create window configuration UI
- [ ] Support timezone handling

---

## Phase 6: Security Enhancements (Priority 2-3)

Comprehensive security hardening.

### mTLS Implementation Details

- [ ] Automated client certificate issuance
  - [ ] Certificate signing request (CSR) handling
  - [ ] Automated CA signing
  - [ ] Certificate delivery to agents
- [ ] Certificate revocation mechanism
  - [ ] CRL generation and distribution
  - [ ] OCSP responder implementation
  - [ ] Revocation checking on agent connections
- [ ] Certificate expiration handling
  - [ ] Expiration monitoring dashboard
  - [ ] Automated renewal workflow
  - [ ] Pre-expiration notifications

### CA Management

- [ ] CA certificate backup and recovery
  - [ ] Automated CA backup
  - [ ] Secure backup storage
  - [ ] Recovery procedures documentation
- [ ] CA rotation automation
  - [ ] Rotation scheduling system
  - [ ] Automated rotation execution
  - [ ] Rollback capability for failed rotations
- [ ] Multi-CA validation support
  - [ ] Trust store management
  - [ ] CA priority handling
  - [ ] Gradual CA migration

### Agent Authentication

- [ ] Certificate-based agent identity
  - [ ] Identity extraction from certificates
  - [ ] Identity-to-agent mapping
  - [ ] Identity persistence
- [ ] Agent authorization policies
  - [ ] Role-based access control
  - [ ] Per-agent permissions
  - [ ] Policy enforcement points
- [ ] Secure agent enrollment flow
  - [ ] Enrollment token generation
  - [ ] Token expiration and validation
  - [ ] Enrollment approval workflow

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

- [ ] Implement rate limiting for all endpoints
- [ ] Add brute force protection
- [ ] Implement security headers
- [ ] Add input validation and sanitization
- [ ] Implement secrets management
  - [ ] Secure credential storage
  - [ ] Credential rotation
  - [ ] Vault integration
- [ ] Add security scanning to CI/CD
  - [ ] Dependency vulnerability scanning
  - [ ] Static code analysis
  - [ ] Container image scanning

---

## Phase 7: Quality & Reliability (Ongoing)

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

- [ ] Implement connection retry logic with exponential backoff
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
- [ ] Implement graceful shutdown
- [ ] Add state recovery on restart
- [ ] Implement data integrity checks
- [ ] Add automatic backup and restore

---

## Phase 8: Documentation & Operations (Ongoing)

Making the system usable and maintainable.

### API Documentation

- [ ] Generate OpenAPI/Swagger specification
- [ ] Document all REST endpoints
- [ ] Add request/response examples
- [ ] Document WebSocket messages
- [ ] Create API client libraries

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
- [ ] Create certificate management guide
  - [ ] Certificate lifecycle overview
  - [ ] Revocation procedures
  - [ ] Backup and recovery
- [ ] Document agent authentication
  - [ ] Enrollment workflow
  - [ ] Identity management
  - [ ] Authorization policies
- [ ] Write security best practices guide
  - [ ] Secure deployment recommendations
  - [ ] Network security
  - [ ] Secret management
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

- [ ] Write CONTRIBUTING.md
- [ ] Document development setup
- [ ] Create architecture documentation
- [ ] Add code style guide
- [ ] Document testing strategy
- [ ] Create PR template and guidelines

---

## Phase 9: Project Infrastructure (Ongoing)

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

---

## Future Considerations

Items to consider for future versions but not currently prioritized:

- [ ] Multi-tenant support
- [ ] Agent clustering
- [ ] High availability for controller
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

---

## Notes

- This roadmap is a living document and should be updated as priorities shift
- Items can be reordered based on user feedback and project needs
- Some items may be split into smaller tasks during implementation
- Cross-phase dependencies should be carefully managed
- Security and quality items should be addressed continuously, not just in their dedicated phases
