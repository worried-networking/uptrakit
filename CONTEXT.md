# uptrakit

uptrakit manages software updates across a fleet of hosts, coordinating agents on remote
machines through a central controller. Single-tenant deployment is the only tested and
supported mode; multi-tenancy exists in the data model but is not validated.

## Language

**Tenant**:
An isolated account that owns all resources. Single-tenant is the only tested deployment mode.
_Avoid_: organization, workspace, account

**Host**:
A server whose software is managed by uptrakit.
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
Services but marked "embedded."
_Avoid_: standalone (ambiguous), monolith

**Plugin**:
An alternative implementation of a common extension point in the Controller, Agent, or
Agent-SSH (discovery, release fetching, package management, notifications, etc.).
Plugins are not compatible with MQTT or Scheduler Services.
_Avoid_: integration, adapter, extension

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

**Operator**:
A person using the Dashboard or CLI to manage Updates and Hosts.
_Avoid_: user, admin (too generic)

**MQTT Service**:
The Service that exposes an MQTT interface for third-party integrations (e.g. Home Assistant).
Not the core Controller↔Service transport (which is WSS).

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
- **"discovery"** — two distinct concepts exist: Software Discovery (installed software
  inventory on a Host) and Proxmox VE Discovery (infrastructure inventory from a PVE host).
  Always qualify with the prefix.
- **"release"** — means "available version to upgrade to." For Docker containers this maps to
  an image tag; for GitHub-based items it maps to a GitHub release. The term is canonical
  regardless of the underlying mechanism.
