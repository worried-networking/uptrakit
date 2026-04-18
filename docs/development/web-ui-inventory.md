# Web UI Inventory

This document inventories the current Svelte web UI in `frontend/src/`.
It is a developer reference for redesign work: which pages exist, which dialogs each page owns,
which shared primitives are already in use, and which reusable components make up the current UI.

Read this together with [UI design language](ui-design-language.md) and the approved redesign spec at
[`docs/superpowers/specs/2026-04-16-ui-design-language-design.md`](../superpowers/specs/2026-04-16-ui-design-language-design.md).

## Shell Inventory

### Global shell (`frontend/src/routes/+layout.svelte`)

| Element | Functionality |
| --- | --- |
| Header bar | Shows product name, current-user shortcut, theme-cycle button, auth buttons, and tablet nav toggle. |
| Desktop sidebar | Left navigation for built-in routes and `surface.page` entries. Uses deterministic priority sorting. |
| Tablet sidebar overlay | Slide-in navigation drawer with focus trap, inert background, and backdrop dismiss. |
| Mobile bottom nav | Four primary nav items plus `More` button for overflow routes. |
| Mobile overflow sheet | Bottom sheet listing overflow navigation links on mobile. |
| Offline banner | Global warning callout shown when the network store reports offline status. |
| Session-expired banner | Global danger callout with relogin action and dismiss button. |
| Toast stack | Shared `ToastNotifications` mount point for success, error, and system alerts. |
| Surface registry bootstrap | Loads shared surface registry when authenticated and clears it on logout. |
| Shell auth guard | Redirects unauthenticated users from protected routes to `/login`. |

### Public-entry shells

| Route | Current shell | Functionality |
| --- | --- | --- |
| `/login` | Standalone centered auth card | Password login, OIDC login, first-user setup, account linking, and registration-token completion. |
| `/register` | Standalone centered auth card | Local account registration with optional invite token. |
| `/device` | Standalone centered device-auth card | Device-code approval flow for CLI login. |
| `+error.svelte` | Bare error page | Displays status code and error message without shared product chrome. |

## Route Inventory

### Primary built-in pages

| Route | Purpose | Main sections and UI structure | Dialogs / menus / overlays |
| --- | --- | --- | --- |
| `/` | Dashboard overview | `PageShell`; summary cards; attention-needed section; recent-updates `DataTable` | None |
| `/services` | Runtime service management | Filter `SectionCard`; registered-services `DataTable`; stacked status badges; batch toolbar | Row context menu; batch confirm dialog; batch result dialog; merge-service modal; edit-ping modal; confirm dialog for approve/reject/delete |
| `/system-services` | System-level service management | Filter `SectionCard`; registered-system-services `DataTable`; stacked status badges; batch toolbar | Row context menu; batch confirm dialog; batch result dialog; edit-ping modal; confirm dialog for approve/reject/delete |
| `/hosts` | Host inventory and actions | `PageShell`; registered-hosts `DataTable`; navigable software/history badges; batch toolbar | Row context menu; batch confirm dialog; batch result dialog; edit-host-name modal; deactivate confirm dialog |
| `/hosts/[id]` | Host detail | Host details card; metadata card; surface-tab section cards; tags card; connected-agents card; assigned-software card; discovery allowlist card; recent-history card | Edit-host-name modal; add-discovery-plugin-type modal; set-tags modal; deactivate confirm dialog; remove-allowlist-entry confirm dialog |
| `/host-tags` | Host tag management | Search card; tags `DataTable`; color/status badges; batch toolbar | Row context menu; batch confirm dialog; batch result dialog; create-tag modal; edit-tag modal; delete confirm dialog |
| `/software` | Grouped software inventory | `PageShell`; built-in plus `software.tabs` tab strip; grouped software list with host subrows; footer bar; batch toolbar | Row context menu; batch confirm dialog; batch result dialog; add-software modal; edit-software modal; assign-to-host modal; merge wizard; trigger-update modal; delete confirm dialog |
| `/software/[id]` | Software-item detail | Software-item summary card; host assignments `DataTable`; surface cards; release metadata and attestation; update controls | Host row context menu; single-host confirm-update modal; release-notes modal; live-terminal modal; edit-host-assignment modal; edit-software modal; delete confirm dialog; unassign confirm dialog; update-all modal; assign-to-host modal; merge wizard; host-context-surface modal |
| `/history` | Update history feed | Filters card; chronological feed; inline terminal output sections; live/captured output states | Trigger-update overlay dialog using `ModalBackdrop`; inline interactive terminal controls |
| `/audit-logs` | Tenant/system audit log search | Scope card; search card; filters card; audit-entry `DataTable`; footer bar | None |
| `/profile` | Account and API-token management | Account card; API-token `DataTable` | New-token modal; revoke-token confirm dialog |
| `/settings` | Product settings hub | `PageShell`; shared `TabStrip`; built-in tab bodies plus `settings.tabs` surfaces | Per-tab dialogs listed below |
| `/surfaces/[id]` | Canonical standalone surface page | `PageShell`; surface summary card; `SurfaceReadPanel`; permission and not-found callouts | Surface-defined modals/workflows through shared surface runtime |

### Settings tab inventory

| Settings area | Functionality | Local dialogs / confirms |
| --- | --- | --- |
| `General Settings` container | Hosts registration, auth, OIDC, agent certificates, enrollment tokens, danger-zone reset | Delegates to child panels below |
| `RegistrationSettings.svelte` | Registration mode, registration token requirement, OIDC first-login behavior | None |
| `AuthenticationSettings.svelte` | Password-auth enablement | None |
| `OidcProvidersSettings.svelte` | OIDC provider list, activate/deactivate, create/edit/delete provider | OIDC provider modal; delete-provider confirm dialog |
| `AgentCertificateSettings.svelte` | Agent cert lifetime and renewal strategy | None |
| `EnrollmentTokenSettings.svelte` | Token list, refresh, create, copy, revoke | Create-enrollment-token dialog; revoke confirm dialog |
| `DangerZone.svelte` | Global destructive reset flow | Reset-all-data modal |
| `GlobalSettingsTab.svelte` | NATS URL, zeroconf settings, network settings, controller TLS certificate, CA rotation, system enrollment tokens, below-global surfaces | Rotate-CA confirm dialog |
| `NotificationRulesSettings.svelte` | Notification-rule CRUD and enablement | Add/edit rule modal; delete-rule confirm dialog |
| `NotificationLogView.svelte` | Delivery log with retry state and status badges | None |
| `PluginConfigsTab.svelte` | Plugin config CRUD, config test, batch delete, discovery allowlist, tenant-wide type defaults | Add/edit config modal; delete-config confirm dialog; batch-delete confirm dialog; add-allowlist modal; remove-allowlist confirm dialog; edit-type-defaults modal; reset-type-defaults confirm dialog |
| `SchedulerTab.svelte` | Scheduled task list, edit cadence/jitter/state, manual trigger | Edit-task modal |
| `SystemServicesSettings.svelte` | System enrollment token list, create, copy, revoke | Create-system-enrollment-token dialog; revoke confirm dialog |

### Public/auth/device routes

| Route | Purpose | Current elements |
| --- | --- | --- |
| `/login` | Authentication entrypoint | Password form, OIDC provider buttons, first-user setup messaging, link-account password form, registration-token completion form, offline markers, error callouts rendered as card-local `aside` blocks |
| `/register` | Password-account registration | Email/name/password fields, optional invite-token toggle, link to login, card-local error callout |
| `/device` | CLI device authorization | Device-code display, approve button, redirect to login if needed, card-local success/warning/error callouts |

## Dialog And Overlay Inventory

### Shared reusable dialogs

| Component | Purpose | Used by |
| --- | --- | --- |
| `Modal.svelte` | Legacy card-style modal wrapper on top of `ModalBackdrop` | Add software, assign to host, edit host assignments, danger-zone reset, scheduler edit, notification rules, plugin config dialogs, system/enrollment token dialogs |
| `ModalShell` | Shared design-language modal wrapper | Host tags, hosts, services, system services, profile, software routes, many settings confirm/edit flows |
| `ConfirmDialog.svelte` | Standard confirmation dialog with semantic tone and warning region | Hosts, host tags, services, system services, profile, software routes, plugin configs, OIDC providers, enrollment tokens, allowlist removal, CA rotation |
| `ContextMenu.svelte` / `ContextMenuShell` | Positioned action menu with keyboard support | Hosts, host tags, services, system services, software list/detail |
| `BatchActionBar.svelte` | Floating batch-selection action toolbar with overflow menu | Hosts, host tags, services, system services, software list |
| `BatchResultDialog.svelte` | Structured dialog for partial batch failures | Hosts, host tags, services, system services, software list |
| `ModalBackdrop.svelte` | Low-level focus-trapping backdrop primitive | History trigger-update overlay; `Modal.svelte` foundation |

### Route-owned dialogs and overlays

| Route family | Dialog / overlay | Functionality |
| --- | --- | --- |
| History | Trigger Update overlay | Select software item and host, trigger update from history page |
| Hosts | Edit Host Name | Rename host |
| Host detail | Edit Host Name | Rename host from detail page |
| Host detail | Add Discovery Plugin Type | Add allowlist entry for discovery |
| Host detail | Set Tags | Multi-select tags on host |
| Host detail | Deactivate Host | Destructive host deactivation |
| Host detail | Remove Allowlist Entry | Remove one allowlist plugin type |
| Host tags | Create Tag | Name and color creation flow |
| Host tags | Edit Tag | Rename/recolor existing tag |
| Host tags | Delete Tag | Destructive delete |
| Services | Merge Service | Merge duplicate/related service identities |
| Services | Edit Ping Interval | Change service heartbeat interval |
| Services | Approve / Reject / Delete confirmations | Service lifecycle actions |
| System services | Edit Ping Interval | Change system-service heartbeat interval |
| System services | Approve / Reject / Delete confirmations | System-service lifecycle actions |
| Profile | New API Token | Create token and reveal secret exactly once |
| Profile | Revoke API Token | Destructive revoke |
| Software list | Add Software | Create tracked software item |
| Software list | Edit Software Item | Rename/retag item fields |
| Software list | Assign To Hosts | Bulk host assignment and hook configuration |
| Software list | Trigger Update | Select host subset for update trigger |
| Software list | Merge Software Items | Multi-step merge wizard |
| Software list | Delete Software Item | Destructive delete |
| Software list | Batch action confirmation | Batch feature/unfeature/update/delete |
| Software detail | Confirm Update | Host-specific update confirmation with version and attestation context |
| Software detail | Release Notes | View release notes and upstream link |
| Software detail | Live Terminal | Interactive/captured update output |
| Software detail | Configure Plugins | Edit host assignment/plugin configuration |
| Software detail | Edit Software Item | Rename / featured state |
| Software detail | Delete Software Item | Destructive delete |
| Software detail | Unassign Host | Destructive unassignment |
| Software detail | Trigger Update — all hosts | Bulk update trigger |
| Software detail | Assign To Hosts | Assignment workflow |
| Software detail | Merge Software Items | Multi-step merge wizard |
| Software detail | Host-context surface modal | Shared surface rendered in host/software context |
| Settings: OIDC providers | Add/Edit OIDC Provider | Provider definition, endpoints, role mapping |
| Settings: Notification rules | Add/Edit Rule | Notification rule CRUD |
| Settings: Plugin configs | Add/Edit Config | Plugin config CRUD with form/JSON modes |
| Settings: Plugin configs | Add Discovery Plugin Type | Discovery allowlist entry |
| Settings: Plugin configs | Edit Type Defaults | Tenant-wide type-setting editor |
| Settings: Scheduler | Edit Task | Interval/jitter/enabled state |
| Settings: Enrollment tokens | Create Enrollment Token | Token generation and one-time reveal |
| Settings: System enrollment tokens | Create System Enrollment Token | Token generation and one-time reveal |
| Settings: Danger zone | Reset All Data | Global destructive reset |
| Settings: Global settings | Rotate CA Certificate | Global destructive PKI rotation |

## Shared Component Inventory

### Route-level shared components (`frontend/src/lib/components/`)

| Component | Functionality |
| --- | --- |
| `AddSoftwareModal.svelte` | Create software item with name, plugin selection, and featured toggle. |
| `AssignToHostModal.svelte` | Assign software to multiple hosts; configure plugin and hook selections per host/role. |
| `BatchActionBar.svelte` | Selection count, direct primary actions, optional `More` dropdown, and select-all-pages affordance. |
| `BatchResultDialog.svelte` | Shows succeeded/failed batch results after bulk operations. |
| `CheckboxList.svelte` | Reusable checkbox list with label, optional sublabel, disabled state, and counter. |
| `ConfirmDialog.svelte` | Shared confirm/cancel dialog with optional warnings and semantic confirm styling. |
| `ContextMenu.svelte` | Positioned context menu shell with focus management and outside-click handling. |
| `EditHostAssignmentModal.svelte` | Advanced plugin assignment editor for a software item on one host, including overrides and hook configuration. |
| `Modal.svelte` | Legacy high-level modal wrapper that provides title, size, and optional footer slot. |
| `ModalBackdrop.svelte` | Focus trap, Escape handling, click-outside close, and backdrop presentation. |
| `Pagination.svelte` | Previous/next controls and page-number window with ellipsis behavior. |
| `SoftwareMergeWizard.svelte` | Two-step search/select/preview/merge workflow for software items. |
| `TagBadge.svelte` | Thin wrapper that renders a host tag through the shared pill style. |
| `TerminalOutput.svelte` | Captured or live terminal renderer with xterm integration and optional stdin forwarding. |
| `ToastNotifications.svelte` | Global stack for success/error/system alert notifications. |

### Shared design-language primitives (`frontend/src/lib/components/ui/`)

| Component | Functionality |
| --- | --- |
| `PageShell.svelte` | Top-level route framing with title, description, and action slot rhythm. |
| `SectionCard.svelte` | Standard bordered section container with optional title, description, and actions slot. |
| `TabStrip.svelte` | Shared tab navigation with active/inactive/disabled states and keyboard semantics. |
| `Callout.svelte` | Inline semantic messaging for info, warning, success, and danger states. |
| `EmptyState.svelte` | Centered no-data / no-provider / no-result state block. |
| `DataTable.svelte` | Canonical table shell with loading, empty, and error affordances plus custom row/header/footer hooks. |
| `TableFooterBar.svelte` | Shared totals-and-pagination footer row for table/list wrappers. |
| `StatusBadge.svelte` | Static compact state label (`success`, `warning`, `danger`, `neutral`, `info`). |
| `ActionBadge.svelte` | Interactive badge with hover-swap labels for navigational and bulk-update actions. |
| `PillBadge.svelte` | Neutral taxonomy pill for plugin/type/tag-like metadata. |
| `ContextMenuItem.svelte` | Shared context-menu row with destructive and disabled variants. |
| `FormFieldRow.svelte` | Two-column label/control row for settings and forms. |
| `ProviderSelector.svelte` | Shared provider picker for targeted surface states. |

### Shared surface-runtime components (`frontend/src/lib/components/surfaces/`)

| Component | Functionality |
| --- | --- |
| `SchemaForm.svelte` | Surface form renderer driven by schema metadata and optional dynamic option loading. |
| `SurfaceActionBar.svelte` | Renders available surface interactions or fallback empty/unavailable states. |
| `SurfaceForm.svelte` | Runs one surface interaction through a form with optional confirmation. |
| `SurfaceInteractionButton.svelte` | Trigger button that dispatches immediate, confirmed, or modal-backed interactions. |
| `SurfaceKeyValue.svelte` | Key/value renderer for simple surface metadata blocks. |
| `SurfaceModal.svelte` | Modal wrapper used by surface-defined modal interactions. |
| `SurfaceReadPanel.svelte` | Entry point for surface runtime-state handling, hydration errors, provider selection, and rendering. |
| `SurfaceRenderer.svelte` | Renders surface node trees: callouts, empty states, tabs, tables, key/value rows, workflows, and modals. |
| `SurfaceSlot.svelte` | Mount point for slot-hosted surfaces. |
| `SurfaceTable.svelte` | Shared surface table renderer built on the same table shell as built-in UI. |
| `SurfaceWorkflow.svelte` | Multi-step interaction workflow with review/confirm states and security-impact display. |

## Current Coverage Summary

### PageShell routes

`/`, `/services`, `/system-services`, `/hosts`, `/hosts/[id]`, `/host-tags`, `/software`,
`/software/[id]`, `/history`, `/audit-logs`, `/profile`, `/settings`, and `/surfaces/[id]`
currently use `PageShell`.

### Routes still outside the shared shell

`/login`, `/register`, `/device`, and `+error.svelte` still use standalone centered-card or bare
error layouts rather than the shared design-language shell primitives.

### Shared feedback surfaces already global

- Offline banner
- Session-expired banner
- Toast notifications
- Live terminal modal shell
- Surface runtime warning / mismatch / no-provider states

These should stay part of any full redesign-alignment audit because they are user-facing UI, not
just implementation plumbing.
