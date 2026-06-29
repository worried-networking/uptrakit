# OpenAPI Codegen Audit — R1 Coverage + R7 Drift Reconciliation + Go/No-Go

- **Date:** 2026-06-28
- **Spec:** `docs/superpowers/specs/2026-06-28-frontend-openapi-client-codegen-design.md` (§8 R1/R7, §13 D1)
- **Inputs:** `frontend/src/lib/api.ts` (152 exported fns), `frontend/src/lib/api/generated/sdk.gen.ts`
  (176 operations), `frontend/src/lib/api/generated/types.gen.ts`, `frontend/src/lib/types.ts`,
  `crates/ui/web-api/src/router.rs`, task-5 (S-A) + task-6 (R5) reports.
- **Purpose:** decision gate for Plan C (the 103-site migration). This audit produces the explicit
  go/no-go and the per-delta triage Phase 2 is contingent on.

## TL;DR

- **R1:** 152 exported `api.ts` functions. **139 covered** by a generated SDK function (matched by
  path + method), **13 permanent hand-written shims**, **0 annotation gaps**. Every REST endpoint
  the SPA calls — including the four Plan A email/password/profile endpoints and `confirmEmailChange`
  — is in the spec and generated. No handler needs new `#[utoipa::path]` work for migration.
- **R7:** Response-type field drift is purely **additive / optional-widening (mechanical)**. All
  behavior-dependent drift is in **Other(String) catch-all enums**, where `utoipa` emits the Rust
  PascalCase variant identifier instead of the serde wire string. **4 enums are behavior-dependent**
  (Permission, NotificationEventType, NotificationDeliveryStatus, PluginRole). All four share **one
  root cause** and **one backend fix** (correct the schema for catch-all enums + regen).
- **Spikes:** S-A **PASSED** (green), R5 **confirmed both enum drifts** (suite green via `it.fails`).
- **Verdict:** **GO.** Behavior-dependent delta count = **4** (threshold is `> 5`). Both blocking
  spikes are green. **GO is conditioned** on landing the single backend schema fix (catch-all enum
  wire values) and regenerating before any call site is migrated — adopting the current generated
  enums as-is would silently break permission checks, notification-rule filtering, plugin-role UI,
  and delivery-status badges. D1 is **not** reopened (NO-GO path not taken).

## (a) R1 — function → operationId coverage map

Matched by `path` + `method` from `sdk.gen.ts` against the `request(...)` path + method in `api.ts`.
Every one of the 152 exported functions is classified below. Covered functions are grouped by domain
(name deltas called out); the 13 gaps are enumerated individually. No function is omitted.

### Covered (139) — grouped by domain, with operationId name deltas

| api.ts function                   | method + path                                                        | generated operationId               | note                            |
| --------------------------------- | -------------------------------------------------------------------- | ----------------------------------- | ------------------------------- |
| register                          | POST /auth/register                                                  | register                            | —                               |
| login                             | POST /auth/login                                                     | login                               | —                               |
| logout                            | POST /auth/logout                                                    | logout                              | —                               |
| me                                | GET /auth/me                                                         | me                                  | —                               |
| getAuthMethods                    | GET /auth/methods                                                    | authMethods                         | rename                          |
| getOidcAuthorizeUrl               | GET /auth/oidc/{id}/authorize                                        | oidcAuthorize                       | rename                          |
| oidcLink                          | POST /auth/oidc/link                                                 | oidcLink                            | —                               |
| oidcCompleteRegistration          | POST /auth/oidc/complete-registration                                | oidcCompleteRegistration            | currently raw fetch; SDK exists |
| oidcExchange                      | POST /auth/oidc/exchange                                             | oidcExchange                        | currently raw fetch; SDK exists |
| mfaVerify                         | POST /auth/mfa/verify                                                | mfaVerify                           | Plan A annotated                |
| mfaSendEmail                      | POST /auth/mfa/email                                                 | mfaSendEmail                        | Plan A annotated                |
| mfaStatus                         | GET /auth/me/2fa                                                     | mfaStatus                           | —                               |
| mfaEnroll                         | POST /auth/me/2fa/totp/enroll                                        | totpEnroll                          | rename                          |
| mfaConfirm                        | POST /auth/me/2fa/totp/confirm                                       | totpConfirm                         | rename                          |
| mfaDisable                        | POST /auth/me/2fa/totp/disable                                       | totpDisable                         | rename                          |
| mfaRegenerateCodes                | POST /auth/me/2fa/recovery-codes/regenerate                          | regenerateRecoveryCodes             | rename                          |
| approveDeviceAuth                 | POST /auth/device/approve                                            | deviceAuthApprove                   | rename                          |
| denyDeviceAuth                    | POST /auth/device/deny                                               | deviceAuthDeny                      | rename                          |
| lookupDeviceAuth                  | GET /auth/device/lookup                                              | deviceAuthLookup                    | rename                          |
| listApiTokens                     | GET /auth/api-tokens                                                 | listApiTokens                       | —                               |
| createApiToken                    | POST /auth/api-tokens                                                | createApiToken                      | —                               |
| revokeApiToken                    | DELETE /auth/api-tokens/{id}                                         | revokeApiToken                      | —                               |
| confirmEmailChange                | GET /auth/email-change/confirm                                       | confirmEmailChange                  | Plan A annotated                |
| getServices                       | GET /services                                                        | listServices                        | rename                          |
| approveService                    | POST /services/{id}/approve                                          | approveService                      | —                               |
| rejectService                     | POST /services/{id}/reject                                           | rejectService                       | —                               |
| deleteService                     | DELETE /services/{id}                                                | deactivateService                   | rename                          |
| updateService                     | PUT /services/{id}                                                   | updateService                       | —                               |
| mergeService                      | POST /services/{target_id}/merge                                     | mergeService                        | —                               |
| batchServices                     | POST /services/batch                                                 | batchServices                       | —                               |
| getHosts                          | GET /hosts                                                           | listHosts                           | rename                          |
| getHost                           | GET /hosts/{id}                                                      | getHost                             | —                               |
| updateHost                        | PUT /hosts/{id}                                                      | updateHost                          | —                               |
| deactivateHost                    | DELETE /hosts/{id}                                                   | deactivateHost                      | —                               |
| batchHosts                        | POST /hosts/batch                                                    | batchHosts                          | —                               |
| triggerHostDiscovery              | POST /hosts/{id}/discover                                            | discoverHost                        | rename                          |
| getHostTags                       | GET /host-tags                                                       | listHostTags                        | rename                          |
| getHostTag                        | GET /host-tags/{id}                                                  | getHostTag                          | —                               |
| createHostTag                     | POST /host-tags                                                      | createHostTag                       | —                               |
| updateHostTag                     | PUT /host-tags/{id}                                                  | updateHostTag                       | —                               |
| deleteHostTag                     | DELETE /host-tags/{id}                                               | deleteHostTag                       | —                               |
| setHostTags                       | PUT /hosts/{id}/tags                                                 | setHostTags                         | —                               |
| batchHostTags                     | POST /host-tags/batch                                                | batchHostTags                       | —                               |
| getAgentCertificateSettings       | GET /settings/agent-certificates                                     | getAgentCertificateSettings         | —                               |
| updateAgentCertificateSettings    | PUT /settings/agent-certificates                                     | updateAgentCertificateSettings      | —                               |
| getCombinedSettings               | GET /settings                                                        | getCombinedSettings                 | —                               |
| listEnrollmentTokens              | GET /enrollment-tokens                                               | listEnrollmentTokens                | —                               |
| createEnrollmentToken             | POST /enrollment-tokens                                              | createEnrollmentToken               | —                               |
| getEnrollmentToken                | GET /enrollment-tokens/{id}                                          | getEnrollmentToken                  | —                               |
| revokeEnrollmentToken             | DELETE /enrollment-tokens/{id}                                       | revokeEnrollmentToken               | —                               |
| getNetworkSettings                | GET /global-settings/network                                         | getNetworkSettings                  | —                               |
| updateNetworkSettings             | PUT /global-settings/network                                         | updateNetworkSettings               | —                               |
| getNatsSettings                   | GET /global-settings/nats                                            | getNatsSettings                     | —                               |
| updateNatsSettings                | PUT /global-settings/nats                                            | updateNatsSettings                  | —                               |
| getGitHubProviderSettings         | GET /global-settings/providers/github                                | getGithubProviderSettings           | rename (case)                   |
| updateGitHubProviderSettings      | PUT /global-settings/providers/github                                | updateGithubProviderSettings        | rename (case)                   |
| getZeroconfSettings               | GET /global-settings/zeroconf                                        | getZeroconfSettings                 | —                               |
| updateZeroconfSettings            | PUT /global-settings/zeroconf                                        | updateZeroconfSettings              | —                               |
| rotateCA                          | POST /global-settings/ca/rotate                                      | rotateCa                            | rename (case)                   |
| getOidcProviders                  | GET /settings/oidc-providers                                         | listProviders                       | rename                          |
| createOidcProvider                | POST /settings/oidc-providers                                        | createProvider                      | rename                          |
| updateOidcProvider                | PUT /settings/oidc-providers/{id}                                    | updateProvider                      | rename                          |
| deleteOidcProvider                | DELETE /settings/oidc-providers/{id}                                 | deleteProvider                      | rename                          |
| activateOidcProvider              | POST /settings/oidc-providers/{id}/activate                          | activateProvider                    | rename                          |
| deactivateOidcProvider            | POST /settings/oidc-providers/{id}/deactivate                        | deactivateProvider                  | rename                          |
| renewServerCertificate            | POST /settings/renew-server-certificate                              | renewServerCertificate              | —                               |
| getSystemAlerts                   | GET /system/alerts                                                   | getSystemAlerts                     | —                               |
| getSystemServices                 | GET /system-services                                                 | listSystemServices                  | rename                          |
| approveSystemService              | POST /system-services/{id}/approve                                   | approveSystemService                | —                               |
| rejectSystemService               | POST /system-services/{id}/reject                                    | rejectSystemService                 | —                               |
| deleteSystemService               | DELETE /system-services/{id}                                         | deactivateSystemService             | rename                          |
| updateSystemService               | PUT /system-services/{id}                                            | updateSystemService                 | —                               |
| batchSystemServices               | POST /system-services/batch                                          | batchSystemServices                 | —                               |
| listSystemEnrollmentTokens        | GET /system-enrollment-tokens                                        | listSystemEnrollmentTokens          | —                               |
| createSystemEnrollmentToken       | POST /system-enrollment-tokens                                       | createSystemEnrollmentToken         | —                               |
| getSystemEnrollmentToken          | GET /system-enrollment-tokens/{id}                                   | getSystemEnrollmentToken            | —                               |
| revokeSystemEnrollmentToken       | DELETE /system-enrollment-tokens/{id}                                | revokeSystemEnrollmentToken         | —                               |
| listPluginTypes                   | GET /plugin-types                                                    | listPluginTypes                     | —                               |
| listPluginTypeSettings            | GET /plugin-type-settings                                            | listPluginTypeSettings              | —                               |
| getPluginTypeSettings             | GET /plugin-type-settings/{plugin_type}                              | getPluginTypeSettings               | —                               |
| upsertPluginTypeSettings          | PUT /plugin-type-settings/{plugin_type}                              | upsertPluginTypeSettings            | —                               |
| deletePluginTypeSettings          | DELETE /plugin-type-settings/{plugin_type}                           | deletePluginTypeSettings            | —                               |
| listInstancePlugins               | GET /instance-plugins                                                | listInstancePlugins                 | —                               |
| setInstancePluginEnabled          | PUT /instance-plugins/{plugin_type}/enabled                          | setInstancePluginEnabled            | —                               |
| upsertInstancePluginConfig        | PUT /instance-plugins/{plugin_type}/config                           | upsertInstancePluginConfig          | —                               |
| getPluginConfigs                  | GET /plugin-configs                                                  | listPluginConfigs                   | rename                          |
| getPluginConfig                   | GET /plugin-configs/{id}                                             | getPluginConfig                     | —                               |
| createPluginConfig                | POST /plugin-configs                                                 | createPluginConfig                  | —                               |
| updatePluginConfig                | PUT /plugin-configs/{id}                                             | updatePluginConfig                  | —                               |
| deletePluginConfig                | DELETE /plugin-configs/{id}                                          | deletePluginConfig                  | —                               |
| triggerPluginConfigDiscovery      | POST /plugin-configs/{id}/discover                                   | discoverPluginConfig                | rename                          |
| testPluginConfig                  | POST /plugin-configs/test                                            | testPluginConfig                    | —                               |
| batchPluginConfigs                | POST /plugin-configs/batch                                           | batchPluginConfigs                  | —                               |
| getSoftwareItems                  | GET /software-items                                                  | listSoftwareItems                   | rename                          |
| getSoftwareItem                   | GET /software-items/{id}                                             | getSoftwareItem                     | —                               |
| createSoftwareItem                | POST /software-items                                                 | createSoftwareItem                  | —                               |
| updateSoftwareItem                | PUT /software-items/{id}                                             | updateSoftwareItem                  | —                               |
| deleteSoftwareItem                | DELETE /software-items/{id}                                          | deleteSoftwareItem                  | —                               |
| batchSoftwareItems                | POST /software-items/batch                                           | batchSoftwareItems                  | —                               |
| previewSoftwareItemMerge          | POST /software-items/merge/preview                                   | previewSoftwareItemMerge            | —                               |
| executeSoftwareItemMerge          | POST /software-items/merge/execute                                   | executeSoftwareItemMerge            | —                               |
| assignHostsToSoftwareItem         | POST /software-items/{id}/hosts                                      | assignHosts                         | rename                          |
| unassignHostFromSoftwareItem      | DELETE /software-items/{id}/hosts/{host_id}                          | unassignHost                        | rename                          |
| updateHostAssignment              | PUT /software-items/{id}/hosts/{host_id}                             | updateHostAssignment                | —                               |
| deletePluginAssignment            | DELETE /software-items/{id}/hosts/{host_id}/plugins/{role}/{ordinal} | deletePluginAssignment              | —                               |
| checkSoftwareItemVersions         | POST /software-items/{id}/check-versions                             | checkVersions                       | rename                          |
| checkSoftwareItemVersionsHost     | POST /software-items/{id}/hosts/{host_id}/check-versions             | checkVersionsHost                   | rename                          |
| triggerSoftwareUpdate             | POST /software-items/{id}/hosts/{host_id}/update                     | triggerUpdate                       | rename                          |
| getSoftwareIgnores                | GET /autodiscovery/ignores                                           | listAutodiscoveryIgnores            | rename                          |
| createSoftwareIgnore              | POST /autodiscovery/ignores                                          | createAutodiscoveryIgnore           | rename                          |
| deleteSoftwareIgnore              | DELETE /autodiscovery/ignores/{id}                                   | deleteAutodiscoveryIgnore           | rename                          |
| batchSoftwareIgnores              | POST /autodiscovery/ignores/batch                                    | batchAutodiscoveryIgnores           | rename                          |
| listUpdateHistory                 | GET /update-history                                                  | listUpdateHistory                   | —                               |
| getUpdateHistoryEntry             | GET /update-history/{id}                                             | getUpdateHistory                    | rename                          |
| listSchedulerTasks                | GET /scheduler/tasks                                                 | listScheduledTasks                  | rename                          |
| getSchedulerTask                  | GET /scheduler/tasks/{id}                                            | getScheduledTask                    | rename                          |
| updateSchedulerTask               | PUT /scheduler/tasks/{id}                                            | updateScheduledTask                 | rename                          |
| triggerSchedulerTask              | POST /scheduler/tasks/{id}/trigger                                   | triggerScheduledTask                | rename                          |
| listDiscoveryAllowlist            | GET /discovery-allowlist                                             | listTenantDiscoveryAllowlist        | rename                          |
| addDiscoveryAllowlistEntry        | POST /discovery-allowlist                                            | addTenantDiscoveryAllowlistEntry    | rename                          |
| deleteDiscoveryAllowlistEntry     | DELETE /discovery-allowlist/{id}                                     | removeTenantDiscoveryAllowlistEntry | rename                          |
| listHostDiscoveryAllowlist        | GET /hosts/{id}/discovery-allowlist                                  | listHostDiscoveryAllowlist          | —                               |
| addHostDiscoveryAllowlistEntry    | POST /hosts/{id}/discovery-allowlist                                 | addHostDiscoveryAllowlistEntry      | —                               |
| deleteHostDiscoveryAllowlistEntry | DELETE /hosts/{id}/discovery-allowlist/{entry_id}                    | removeHostDiscoveryAllowlistEntry   | rename                          |
| listAuditLogs                     | GET /audit-logs                                                      | listAuditLogs                       | —                               |
| listSystemAuditLogs               | GET /system-audit-logs                                               | listSystemAuditLogs                 | —                               |
| listNotificationChannels          | GET /notifications/channels                                          | listChannels                        | rename                          |
| listNotificationRules             | GET /notifications/rules                                             | listRules                           | rename                          |
| createNotificationRule            | POST /notifications/rules                                            | createRule                          | rename                          |
| updateNotificationRule            | PUT /notifications/rules/{id}                                        | updateRule                          | rename                          |
| deleteNotificationRule            | DELETE /notifications/rules/{id}                                     | deleteRule                          | rename                          |
| listNotificationLog               | GET /notifications/log                                               | listLog                             | rename                          |
| resetData                         | POST /settings/reset-data                                            | resetData                           | feature-gated (reset-data)      |
| updateProfile                     | PUT /users/{id}/profile                                              | updateProfile                       | Plan A annotated                |
| initiateEmailChange               | POST /users/{id}/email                                               | initiateEmailChange                 | Plan A annotated                |
| cancelEmailChange                 | DELETE /users/{id}/email                                             | cancelEmailChange                   | Plan A annotated                |
| changePassword                    | PUT /users/{id}/password                                             | changePassword                      | Plan A annotated                |
| getConfigState                    | GET /instance/config-state                                           | getConfigState                      | —                               |
| clearCoordinatorDegraded          | POST /instance/config-reload/clear-degraded                          | clearCoordinatorDegraded            | —                               |

Row count above = 139. (Name deltas are R4 churn, not gaps; barrel re-exports keep import paths
stable. ~62 functions adopt a generated name; pin via explicit `operation_id` only if desired —
OQ2.)

### Gaps (13) — all permanent hand-written shims; 0 need backend annotation

| api.ts function                   | reason                                                               | resolution (target module)                                  |
| --------------------------------- | -------------------------------------------------------------------- | ----------------------------------------------------------- |
| `_resetSettingsEtagCacheForTests` | test-only ETag cache reset; not an endpoint                          | test seam in `client.ts`                                    |
| `extractErrorMessage`             | error-body parsing helper; not an endpoint                           | `errors.ts`                                                 |
| `request`                         | internal authenticated-request helper                                | folded into `client.ts` interceptors                        |
| `authenticatedFetch`              | raw-Response escape hatch (auth + timeout + 401 retry)               | `raw.ts`                                                    |
| `apiGet`                          | generic authenticated GET used by surfaces/components                | `raw.ts`                                                    |
| `loginRaw`                        | POST /auth/login raw Response (202 MFA challenge inspection)         | `raw.ts`                                                    |
| `refreshAccessToken`              | POST /auth/refresh kept as raw fetch to avoid recursive interception | `client.ts` (SDK `refresh` exists but intentionally unused) |
| `executeBatchChunked`             | client-side 100-id chunking helper; not an endpoint                  | `batch.ts`                                                  |
| `sealedBoxEncrypt`                | Web Crypto ECIES; not HTTP                                           | `crypto.ts`                                                 |
| `listSurfaces`                    | GET /surfaces — 0 `#[utoipa::path]` (dynamic UI extension)           | `surfaces.ts`                                               |
| `listSurfaceProviders`            | GET /surfaces/{id}/providers — not in spec                           | `surfaces.ts`                                               |
| `getSurfaceRead`                  | GET /surfaces/{id}/read — not in spec                                | `surfaces.ts`                                               |
| `invokeSurfaceInteraction`        | POST /surfaces/{id}/interactions/{iid} — not in spec                 | `surfaces.ts`                                               |

**Key R1 finding:** the R7-flagged raw-`.route()` sub-class (`initiate_email_change`,
`cancel_email_change`, `change_password`, `confirm_email_change`) is **no longer a gap** — Plan A
annotated all four and they generate as `initiateEmailChange` / `cancelEmailChange` /
`changePassword` / `confirmEmailChange`. **No annotation work remains for migration.**

### Generated operations with no current `api.ts` caller (informational)

Not gaps — extra coverage the SPA may adopt or that other modules use. Includes: `getAsMetadata`,
`token`, `deviceAuthorization` (OAuth, used by `api/oauth.ts`); `getOauthSettings`,
`updateOauthSettings` (OAuth settings); `getAccessSettings`, `updateAccessSettings` (used by
`api/settings.ts`, to migrate per §3.4); `listAccessPresets`, `applyPreset`, `listUsers`, `getUser`,
`updateUserActive`, `updateUserRoles`, `listRoles`, `getRole`, `listPermissions` (user/role admin);
`setUpdateFreeze`, `triggerHostBatchUpdate`, `triggerItemBatchUpdate`, `approveSoftwareItem`,
`listBatches`, `getBatch`, `getGlobalCombinedSettings`, `oidcCallback`. SSE operations
`streamBatchProgress` / `streamUpdateOutput` are generated but stay hand-written (`sse.ts`; hey-api
does not generate streaming clients).

### Post-split raw `.route()` endpoints — intentionally out of spec (R1 completeness)

Registered after `split_for_parts()` (`router.rs` L971+) or as plain pre-split axum routes; not in
`openapi.json`, no generated SDK function, **and not called by the SPA's typed client**.

| raw route                                                     | kind                                | api.ts caller?            | record                                      |
| ------------------------------------------------------------- | ----------------------------------- | ------------------------- | ------------------------------------------- |
| POST /notifications/callback/{channel_type}/{channel_id}      | inbound webhook (external services) | none (confirmed)          | intentionally out of spec, no shim needed   |
| GET /ws/service                                               | service WebSocket                   | none                      | out of spec (not SPA)                       |
| GET /update-history/{id}/interactive                          | interactive WS (feature)            | none                      | out of spec (not SPA)                       |
| GET /events/stream                                            | admin SSE                           | `sse.ts` (own fetch+auth) | out of spec (streaming)                     |
| GET /healthz, /readyz                                         | health probes                       | none                      | non-frontend                                |
| GET /pki/ca.crt, /pki/ca.crl, POST/GET /pki/ocsp              | PKI / OCSP                          | none                      | non-frontend                                |
| /api/oauth/clients[/{id}[/trust]], /api/oauth/consents[/{id}] | OAuth mgmt (pre-split raw)          | `api/oauth.ts` shims      | permanent hand-written shim (`oauth.ts`)    |
| /api/v1/surfaces/\*                                           | surfaces (pre-split raw)            | `surfaces.ts` shims       | permanent hand-written shim (counted above) |
| /test/\*                                                      | test-utils (feature)                | none                      | non-frontend                                |

## (b) R7 — types.ts vs types.gen.ts wire-value diff

Default authority: **spec/backend**. A delta is **behavior-dependent** only when the running app
currently relies on the drifted value/shape; otherwise **mechanical**.

### Non-enum fields (response/request types) — all mechanical

Both `types.ts` and `types.gen.ts` derive from the same Rust structs (serde `rename_all =
"snake_case"`), so field **names match by construction**. Sampled high-traffic types
(`ServiceResponse`, `HostResponse`, `UpdateHistoryResponse`, `User`, `ServiceStatus`): the only
deltas are

- **Additive optional fields** the generated side gained (e.g. `ServiceResponse.cert_serial_number`,
  `ServiceResponse.spiffe_id`) — `types.ts` is simply stale; the app does not read them. Mechanical.
- **Optionality representation:** generated emits `field?: T | null` (key may be absent) where
  `types.ts` had `field: T | null` (key present, value nullable). For reads in JS both collapse to a
  falsy / nullish-coalescing check; generated reflects the real `Option<T>` serialization. Mechanical; spec
  authoritative.

No sampled response field was **renamed** or had a value-shape the app depends on changed. No
behavior-dependent non-enum delta found.

**Residual risk (sampling):** a renamed or optionality-changed field in an _unsampled_ response type
would be a behavior-dependent delta missed by sampling. With the count at 4 — one below the `> 5`
gate — it would take 2+ such missed deltas to flip the verdict. Low likelihood given the
serde-by-construction argument, but not zero.

### Enums — the only behavior-dependent drift

Root cause (single): enums carrying an `Other(String)` wire-safe catch-all (the `wire_safe_enum!`
macro family, plus `Permission` and `PluginRole`) derive a `utoipa::ToSchema` that emits the **Rust
PascalCase variant identifier** as the schema enum value, instead of the serde-renamed wire string
(`=> "..."`). hey-api faithfully reproduces the (wrong) schema as a PascalCase union with an
`{ Other: string }` arm. The **actual wire format is unchanged** (snake_case / lowercase, proven by
the Rust serde round-trip tests in `crates/shared/web-api-types/src/notifications/event_types.rs`).
Enums **without** a catch-all (`ServiceStatus`, `MfaMethod`, `AlertSeverity`, `UpdateStatus`,
`TriggerUpdateStatus`, …) generate correctly (`MfaMethod` even keeps its lowercase `{ other }` arm)
— they match the app.

| enum                                  | wire (authoritative)            | generated (types.gen.ts)                 | app (types.ts)       | app compares it?                                            | class                  |
| ------------------------------------- | ------------------------------- | ---------------------------------------- | -------------------- | ----------------------------------------------------------- | ---------------------- |
| Permission                            | snake_case (`view_services`)    | PascalCase (`ViewServices`) + `{Other}`  | snake_case enum      | YES — `permissions.includes(...)` everywhere                | **behavior-dependent** |
| NotificationEventType                 | snake_case (`update_available`) | PascalCase + `{Other}`                   | snake_case union     | YES — rule filters + `sse.ts`                               | **behavior-dependent** |
| NotificationDeliveryStatus            | lowercase (`delivered`)         | PascalCase (`Delivered`) + `{Other}`     | lowercase union      | YES — `statusTone()` switch in `NotificationLogView.svelte` | **behavior-dependent** |
| PluginRole                            | snake_case (`detect_version`)   | PascalCase (`DetectVersion`) + `{Other}` | snake_case enum      | YES — `EditHostAssignmentModal.svelte`                      | **behavior-dependent** |
| OAuthErrorCode                        | snake_case (RFC 8628)           | PascalCase + `{Other}`                   | n/a (no named type)  | NO — handled via raw `oauth.ts`, no static compare          | mechanical             |
| PluginCapability                      | snake_case (`update_lifecycle`) | snake_case `const` (+2 new values)       | snake_case enum (15) | values match; member-key rename only                        | mechanical (additive)  |
| Permission `AccessMcp`                | new permission value            | present                                  | absent               | additive; not yet referenced                                | mechanical (additive)  |
| McpScope, ProviderEncryptionAlgorithm | catch-all enums                 | **absent from spec**                     | absent               | not in frontend contract                                    | n/a                    |

**Authoritative side for the 4 behavior-dependent enums: the app / the wire (snake_case).** The
generated PascalCase is a **backend schema bug**, not a contract change. The fix belongs in the
backend (correct the catch-all enums' `ToSchema` to emit the serde wire strings) + regenerate — not
in the frontend. After that fix the 4 deltas collapse to zero and the generated enums match the app
byte-for-byte. PluginCapability/`AccessMcp` are independent additive items (extend or defer).

## (c) Spike outcomes (S-A + R5)

### S-A (Task 5) — refresh-retry + ApiError identity: **PASSED / GO (green)**

- Mechanism: a `client.request` Proxy wrapper exposed as `apiClient`. Required because the error
  interceptor always re-throws under `throwOnError` — there is no way to convert an error path into a
  retried success from an interceptor. Wrapper owns only deduped refresh + retry-once + session
  banner; ETag / 2FA / ApiError mapping stay in separate interceptors (did not regrow into
  `authenticatedFetch`). CodeScene `client.ts` **9.68 (Green)**.
- **Caveats Plan C must honor:** generated SDK ops use the raw `client` singleton, so call sites must
  use `apiClient.*` or pass `{ client: apiClient }` or refresh-retry is bypassed; a 2nd consecutive
  401 surfaces a statusless body; SSE bypasses refresh; refresh failures map to plain `Error` (not
  `ApiError`); 2FA-403 throws a generic `ApiError(403)`, not `TwoFactorSetupRequiredError`.

### R5 (Task 6) — golden enum value-equality: **confirmed drift, suite green**

Two enums asserted-failing via `it.fails()`: **Permission** (PascalCase vs snake_case — authority
resolved to the wire/app; backend schema fix) and **PluginCapability** (format matches; generated
adds `software_item_lifecycle` + `enrich_installed_version`, additive). This audit extends R5's
finding: the Permission-class drift is **systematic** across all `Other(String)` catch-all enums
(Permission, NotificationEventType, NotificationDeliveryStatus, PluginRole), one root cause, one fix.

**R5 guard-coverage gap:** R5 (`enum-parity.test.ts`) guards only `Permission` and `PluginCapability`
— the only two runtime `export enum`s in `types.ts`. The other three behavior-dependent enums
(`NotificationEventType`, `NotificationDeliveryStatus`, `PluginRole`) are TypeScript `type` unions;
`Object.values()` cannot enumerate them, so the `it.fails()` mechanism does not cover them. A green
R5 after the backend fix proves `Permission` and `PluginCapability` are correct; it does **not** prove
the remaining three collapsed. See the additional prerequisite in §(d).

## (d) Go / No-Go

Classification recap (per the §13 / brief rule): mechanical = rename / additive-optional / enum
value the app does not compare; behavior-dependent = app relies on the drifted value/shape.

- Blocking spikes: **S-A green, R5 green.** Neither is red.
- **Behavior-dependent delta count = 4** (Permission, NotificationEventType,
  NotificationDeliveryStatus, PluginRole). Threshold for NO-GO is **> 5**. **4 ≤ 5.**
- Everything else (139 covered fns, ~62 name renames, all non-enum field drift, OAuthErrorCode,
  PluginCapability +2, `AccessMcp`) is mechanical.

### Verdict: **GO**

Both blocking spikes pass and behavior-dependent drift (4) is under the gate. Migration may proceed
to Phase 2 — **conditioned on a single mandatory prerequisite**:

> **Prerequisite (must land + regen before any of the 103 call sites are migrated):** fix the
> backend `ToSchema` for `Other(String)` catch-all enums so the schema enum values are the serde
> wire strings (snake_case / lowercase), not the Rust PascalCase identifiers. Regenerate
> `openapi.json` + the client. This is one bounded backend change that collapses all 4
> behavior-dependent deltas at once. Re-run the R5 golden value-equality test afterward; it must turn
> green (drop the `it.fails()` guards). Migrating against the current PascalCase enums would silently
> break permission checks, notification-rule filtering, the plugin-role assignment UI, and
> delivery-status badges — none reliably caught by E2E.

**Additional prerequisite — R5 guard gap (must resolve before migrating `NotificationEventType` /
`NotificationDeliveryStatus` / `PluginRole` call sites):** "Re-run R5 until green" is **not
sufficient** to confirm all 4 drifts collapsed. R5 guards only `Permission` and `PluginCapability`
(the two runtime `export enum`s in `types.ts`); the remaining three enums are TypeScript `type`
unions not reachable by `Object.values()`. Before migrating any call site that touches
`NotificationEventType`, `NotificationDeliveryStatus`, or `PluginRole`: either **extend the parity
guard** to cover these three type-union enums (e.g. a literal-member parity check against a
`KNOWN_VARIANTS` const array), **or manually verify** their post-fix generated values match the wire
strings. This applies to Phase 2 call-site migration; it does not block the backend fix itself.

Independent, non-blocking follow-ups: decide whether to add `PluginCapability`'s two new values and
the `AccessMcp` permission to the frontend (additive); optionally pin `operation_id` to reduce the
~62 name renames (OQ2).

### D1 re-cost note

NO-GO was **not** taken, so D1 (full codegen vs the rejected targeted refactor) is **not reopened**.
The drift is not "large or value-flipping" in a way that voids the "no behavior change" premise: it
is one systematic, mechanically-fixable backend schema bug plus stale-but-additive types — exactly
the class of drift full codegen exists to eliminate permanently. The bounded prerequisite (one macro
fix + regen) does not change the D1 cost comparison; full codegen still pays off via drift-elimination
and the Rust-client follow-on. Were the count to exceed 5, the §13 re-cost against the targeted
refactor would be triggered here — it is not.
