# uptrakit

uptrakit manages software updates across a fleet of hosts, coordinating agents on remote
machines through a central controller. Single-tenant deployment is the only tested and
supported mode; multi-tenancy exists in the data model but is not validated.

## Language

**Tenant**:
An isolated account that owns all resources. Single-tenant is the only tested deployment mode.
_Avoid_: organization, workspace, account

**Host**:
A server whose software is managed by uptrakit. Supported OS families: Linux,
macOS, FreeBSD, and RouterOS (MikroTik). Agent-SSH bootstraps RouterOS hosts
via a non-POSIX SSH path; all other OS families use a standard POSIX SSH path.
_Avoid_: device (reserved for the OAuth CLI flow), machine, node, server

**Software Item**:
A managed piece of software installed on a Host (app, daemon, package, container).
_Avoid_: service (reserved for uptrakit's own satellite components), app, workload

**Release**:
An available version of a Software Item that can be applied via an Update. For container-based
items, a Release maps to an image tag, not a GitHub release object.
_Avoid_: version, tag (too ambiguous across package managers and container registries)

**Update**:
An instruction dispatched to an Agent to apply a change to a Software Item on a Host.
_Avoid_: upgrade, install, deployment

**UpdateStatus**:
The lifecycle state of a single Update row. Terminal statuses: `Completed`, `Failed`,
`Interrupted`. Non-terminal statuses: `Queued`, `Pending`, `InProgress`, `AwaitingRestart`.
`Interrupted` = outcome unknown (connection lost / deadline exceeded); verify before
re-running. Grouping helpers — `unfinished()` = [Queued, Pending, InProgress, AwaitingRestart]
(all non-terminal; use for duplicate-trigger checks); `host_blocking()` = [Pending, InProgress,
AwaitingRestart] (occupies host execution slot; excludes Queued). Terminal statuses appear in
neither helper set.

**Controller**:
The central server that coordinates all Services.
_Avoid_: backend, server, hub

**Service**:
A satellite component of the Controller — one of: Agent, Agent-SSH, MQTT, Scheduler.
_Avoid_: microservice, module; never use to mean a managed Software Item

**Agent**:
A Service type that runs on a local Host; executes Updates and tracks installed Software Item versions.
_Avoid_: daemon (too generic)

**Agent-SSH**:
A Service type that manages remote Hosts over SSH; same responsibilities as Agent but proxied.
One Host may be tracked by more than one Agent-SSH instance.
_Avoid_: SSH agent

**Embedded Mode**:
A deployment configuration (built via the `controller-standalone` crate) where some or all
Services run inside the Controller binary. Embedded Services are still displayed as separate
Services but marked "embedded." Embedded Services share the controller's `DatabaseConnection`
rather than opening their own — no per-service DB files are created in embedded mode.
_Avoid_: standalone (ambiguous), monolith

**Plugin**:
An alternative implementation of a common extension point in the Controller, Agent, or
Agent-SSH (discovery, release fetching, package management, notifications, etc.).
Plugins are not compatible with MQTT or Scheduler Services. Every Plugin has a **Plugin
Scope** (see below) that determines who manages it.
_Avoid_: integration, adapter, extension

**Plugin Scope**:
Either **Tenant-Scoped** (the default — configured by Operators with tenant-level permissions
through `plugin_configs`/`plugin_type_settings`) or **Instance-Scoped** (configured only by
Operators with `ManageGlobalSettings`; settings live in `global_settings`; if disabled, the
Plugin is invisible and inert for tenant Operators). Distinct from `GlobalProviderConsumer`,
which is an unrelated cross-plugin shared-resource mechanism.
_Avoid_: "global plugin" in code/spec (term collision with `global_settings` and
`GlobalProviderConsumer`); UI label may still read "Global Plugins".

**Instance-Scoped Plugin**:
A Plugin whose enable/disable state and configuration are managed exclusively at the instance
level via `global_settings`, gated by `ManageGlobalSettings`. When disabled, no API surface,
Surface, or runtime hook exposes its existence to tenant Operators.
_Avoid_: global plugin (in code), system plugin, root plugin

**Effective Enablement**:
An Instance-Scoped Plugin's runtime availability: effective = boot catalog state AND live snapshot
state (ADR-0033). Disable is immediate; enable stays pending until restart. Governs every surfaces
leg for every permission tier.

**Contribution Monotonicity**:
The invariant that enabling a Cargo feature may only add plugin descriptor contributions
(Surfaces, migrations, role slots), never remove or alter them. Guarded by the registry
catalog guard tests (ADR-0032).

**Installed Version Enricher**:
A controller-only plugin role (trait + slot) that derives a human-friendly
`installed_display_version` from the raw `installed_version` reported by an Agent. Used when
the raw value is an opaque identifier (e.g. a git tree SHA for an LLM Skill) and the
display string must come from upstream metadata that only the Controller can reach. Mirrors
the `ReleaseFetcher` controller-side pattern but targets the detect_version output instead of
the fetch_releases output. Gated by the `EnrichInstalledVersion` capability.
_Avoid_: version translator, display resolver (too generic), version formatter (frontend
concern).

**Enrollment**:
The process by which a Service registers with and gets approved by the Controller.
_Avoid_: registration, onboarding

**Software Discovery**:
The process of inventorying what Software Items are installed on a Host. Distinct from
Proxmox VE Discovery.

**Proxmox VE Discovery**:
The process of discovering VMs and containers from a Proxmox VE host. Distinct from
Software Discovery.

**Dashboard**:
The web UI through which Operators manage the system.
_Avoid_: frontend, web app, admin panel

**Dynamic Client Verifier**:
The Controller-side wrapper around `rustls::server::WebPkiClientVerifier` that exposes an
`ArcSwap` inner verifier. Lets CRL rebuilds and CA-bundle updates hot-swap the verifier
without rebuilding `rustls::ServerConfig`.
_Avoid_: verifier reload (overloaded with graceful-reload terminology).

**Operator**:
A person using the Dashboard or CLI to manage Updates and Hosts.
_Avoid_: user, admin (too generic)

**MQTT Service**:
The Service that exposes an MQTT interface for third-party integrations (e.g. Home Assistant).
Not the core Controller↔Service transport (which is WSS).

**SPIFFE Service Identity**:
A Service's identity carried as a URI Subject Alternative Name on its client certificate,
of the form `spiffe://<trust_domain>/service/<service_id>`. Replaces CN-only identity over
the natural cert renewal cycle.
_Avoid_: service URI, workload ID (SPIFFE has a precise term).

**Service Merge Redirect**:
A persisted mapping from a deactivated Service UUID to the merge-target Service UUID, written
by `merge_service` so that an Agent which enrolled against the deactivated row can reconnect,
be re-keyed onto the target identity, and have its `service.json` rewritten without operator
intervention.

**Surface**:
A named UI extension point in the Dashboard, declared at a specific Slot, that built-ins,
Plugins, or Services can register content and interactions into.
_Avoid_: widget, panel, page (too generic)

**Slot**:
A declared location in the Dashboard where one or more Surfaces can appear (e.g.
`settings.tabs`, `host_detail.tabs`). Single-entry Slots allow only one provider; multi-entry
Slots allow many.
_Avoid_: placeholder, region

**Surface Provider**:
A built-in, Plugin, or Service that registers a Surface into a Slot at runtime.
_Avoid_: plugin provider (conflates with Plugin), contributor

**TOFU mode**:
One of `system`, `pin-fingerprint`, `pin-spki`, `insecure-tofu`. Selected at Service boot
via a mutually-exclusive CLI flag. Determines how the Service verifies the Controller's
TLS certificate during bootstrap and on reconnects when no CA bundle has been persisted.
_Avoid_: TOFU enabled (ambiguous — every mode is "enabled" in some sense).

**CLI CA Trust**:
The mechanism by which the `uptrakit` CLI establishes and persists trust in a Controller's
self-managed CA certificate. The CLI fetches the CA from `GET /api/v1/pki/ca.crt`, verifies
the SHA-256 fingerprint interactively or against a supplied value, and stores the PEM in
`config.json` as the sole TLS trust anchor for future connections.
Entry points: `uptrakit auth login --tofu` (bootstrap) and `uptrakit auth ca trust` (rotation).
_Avoid_: TOFU mode (reserved for Service bootstrap flags).

**Trust Domain**:
A string in the `[tls]` Config Section naming the Controller's SPIFFE namespace. Defaults
to the first server-cert SAN. Must match the trust-domain segment of every Service's
SPIFFE URI SAN.
_Avoid_: domain (overloaded), namespace (Kubernetes overload).

**Consent Grant**:
A User's persisted approval of an OAuth Client's scope set, recorded in `oauth_consents`. Revocable
from the User's Authorized Apps view or via cascade when the OAuth Client is revoked.
_Avoid_: authorization (already overloaded in OAuth context), permission grant (collides with
Permission).

**MCP Authorization Server**:
The OAuth 2.1 Authorization Server embedded in the Controller. Issues access and refresh tokens for
the MCP Resource Server. Canonical URL `https://<oauth.canonical_host>`. Lives inside
`uptrakit-web-api` v1.
_Avoid_: auth server (collides with OIDC Provider auth), token server.

**MCP Resource Server**:
The component of `uptrakit-mcp` that validates OAuth access tokens for inbound MCP requests.
Canonical URL `https://<oauth.canonical_host>/mcp`. Identified by the `resource` parameter in
RFC 8707 token requests.
_Avoid_: MCP endpoint, MCP service (collides with Service).

**OAuth Client**:
An application identity registered with the controller's MCP Authorization Server. Created via
Dynamic Client Registration, Client ID Metadata Document fetch, or manual Operator registration.
Holds redirect URIs, default scopes, and one or more grants from Users.
_Avoid_: application, third party (too vague), integration (conflicts with Plugin).

**Scope (OAuth)**:
A string that names an action class an OAuth Client may perform on the MCP Resource Server. v1
values: `mcp:read`, `mcp:write`. Distinct from **Permission**, which names what a User can do
regardless of which client they use. Effective rights = `scope ∩ Permission`.
_Avoid_: using `Scope` bare for either concept without the `(OAuth)` / `(Permission)` qualifier in
code comments or docs.

**Config Section**:
Logical grouping of settings whose lifetime is bound together for reload; one `Arc<SectionConfig>`
and one `watch::Sender` per section.

**ConfigReconciler**:
Tokio task that polls `settings_version` for bumps and enqueues reload requests for affected DB
sections.

**Irreversibly-bound key**:
Configuration key whose change cannot be applied to a running process without compromising
correctness, safety, or operability. Current set: `db.url`, `master_key`, `log.path`,
embedded-services topology. Set membership changes are ADR amendments.

**Reexec**:
In-place process replacement via `exec()` with inherited listening sockets; used for
irreversibly-bound key changes. _Avoid_: "graceful restart".

**Reloadable**:
Long-lived subsystem implementing the `Reloadable` trait, participating in reload.

**Reload Coordinator**:
Single Tokio task that serialises reload requests, runs two-phase validate-then-apply, drives the
watchdog, commits or reverts. Exactly one per Controller process. _Avoid_: "config manager",
"reload manager".

**Watchdog window**:
Per-subsystem time budget within which a newly-applied configuration must pass `health_check()`.
Default values are constants in `uptrakit-config-reload::defaults`.

**Surface Action Controller**:
Workflow-scoped trait (`SurfaceActionController`) given to surface-action handlers via `SurfaceActionContext`;
exposes `tenant_id()`, `user_id()` (re-exposed to handlers as `caller_user_id()`), and `tenant_db()`.

**Update Protection Controller**:
Workflow-scoped trait (`UpdateProtectionController`) given to update-protection plugins;
exposes `tenant_db()` for the protection workflow only.

**Update Hook Controller**:
Workflow-scoped trait (`UpdateHookController`) given to pre/post update-hook plugins;
exposes `tenant_db()` for the update-hook workflow only.

**Surface Action Context**:
Struct wrapping `&dyn SurfaceActionController` passed to every surface-action handler;
delegates `tenant_id()`, `caller_user_id()`, and `tenant_db()`.

**Surface Action Error**:
Typed error enum (`SurfaceActionError`) returned by surface-action handlers;
variants: `InvalidInput`, `ControllerIntegration`, `PluginInternal`.

**Plugin Config Validation Error**:
Typed error enum (`PluginConfigValidationError`) returned by config validation;
variants: `InvalidField { field, message }`, `InvalidIdentifier`, `Contract`.

## Relationships

- A **Tenant** owns **Hosts**, **Software Items**, **Updates**, and all configuration
- A **Host** is managed by exactly one **Agent**, or tracked by one or more **Agent-SSH** instances
- A **Controller** coordinates one or more **Services**
- **Agent** and **Agent-SSH** are both types of **Service**
- A **Plugin** extends the **Controller**, **Agent**, or **Agent-SSH** only — not MQTT or Scheduler
- In **Embedded Mode**, some or all **Services** run inside the **Controller** binary
- An **Update** targets one **Software Item** on one **Host**
- A **Release** belongs to a **Software Item**; an **Update** applies a **Release**
- **Enrollment** links a **Service** to a **Controller**
- A **Slot** holds one or more **Surfaces**; a **Surface Provider** registers content into a **Slot**
- **Surface Providers** can be built-ins, **Plugins**, or **Services**

## Example dialogue

> **Dev:** "When an Operator adds a new Host, does Enrollment happen automatically?"
>
> **Domain expert:** "Not quite — a Host is added when an Agent (or an Agent-SSH target) comes
> online. Enrollment is the Service registering with the Controller and getting approved. The
> Host is a side-effect of that process."
>
> **Dev:** "And when we dispatch an Update, are we targeting the Host or the Software Item?"
>
> **Domain expert:** "Both — an Update targets a specific Software Item on a specific Host.
> The Agent on that Host executes it."
>
> **Dev:** "So a Plugin for apt is providing a Release for a Software Item managed as a
> Debian package?"
>
> **Domain expert:** "Exactly. The apt Plugin discovers installed Software Items via Software
> Discovery and fetches available Releases. The Operator then triggers an Update."

## Flagged ambiguities

- **"service"** — in uptrakit this always means a satellite component of the Controller
  (Agent, Agent-SSH, MQTT, Scheduler). Never use to mean a managed Software Item on a Host.
- **"discovery"** — three distinct concepts exist: Software Discovery (installed software
  inventory on a Host), Proxmox VE Discovery (infrastructure inventory from a PVE host), and
  Zeroconf Discovery (locating the Controller itself via mDNS/DNS-SD on the local network;
  Services and the CLI browse, the Controller advertises). Always qualify with the prefix.
- **"release"** — means "available version to upgrade to." For Docker containers this maps to
  an image tag; for GitHub-based items it maps to a GitHub release. The term is canonical
  regardless of the underlying mechanism.
- **"scope"** — two distinct concepts exist: **Scope (OAuth)** (action class on the MCP Resource
  Server) and **Permission** (user right). Always qualify when ambiguous; the typed enum names
  (`McpScope`, `Permission`) keep code clear.
