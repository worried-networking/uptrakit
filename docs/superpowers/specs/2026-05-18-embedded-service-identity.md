# Spec: Embedded Service Identity — Fix Surface Registration

**Date:** 2026-05-18
**Status:** Draft

---

## Problem

`run_embedded_service` skips `on_connected`. That callback is the only place service
handlers receive their identity (`service_id` + keypair → `encryption_public_key`).
Three services are broken as a result:

| Service   | Symptom                                                                                                       |
| --------- | ------------------------------------------------------------------------------------------------------------- |
| agent-ssh | `service_id` and `encryption_public_key` stay `None`; surface registration rejected (6 reasons) every startup |
| mqtt      | workaround shunts identity init into `on_settings` via `embedded_identity` field — mode-aware anti-pattern    |
| scheduler | `service_id` stays `None`; crashes when `ServiceCredentials` arrives and handler requires it                  |

The standalone path calls `on_connected` correctly. Services must have zero knowledge of
which runtime they're in. The SDK must make both paths identical from the handler's point
of view.

---

## Goals

1. `run_embedded_service` calls `handler.on_connected` before `handler.on_settings`, just
   like the standalone lifecycle does.
2. Handlers contain no embedded/standalone branches — no `ecies_keypair`, no
   `embedded_identity`, no `persist_tenant_id` flag.
3. The controller does not generate or own service identity keypairs — that logic belongs
   to the SDK.
4. `persist_tenant_id` unifies: the same code path runs for both modes; the no-op
   behaviour for embedded falls out naturally from the identity state, not from a flag.

---

## Non-Goals / Deferred

- ECIES algorithm migration (P-256 → P-384, reusing the TLS keypair). Separate future
  spec: "P-384 for CA certs, P-256 for server/service/ECIES certs."
- Wire-protocol changes (`ServiceSettingsPayload`).
- Frontend changes.

---

## Design

### 1 — `EmbeddedServiceHost::add()` passes `service_id` to the run closure

`EmbeddedServiceHost::add()` already provisions the service record (step 1 in its
implementation) before it spawns the run closure (step 5). The provisioned `Uuid` is
available inside `add()` but not currently forwarded.

Change the `run_fn` parameter type from:

```rust
run_fn: impl FnOnce(EmbeddedTransport, EmbeddedShutdownTokens) -> Pin<Box<dyn Future<Output = ()> + Send>>
```

to:

```rust
run_fn: impl FnOnce(Uuid, EmbeddedTransport, EmbeddedShutdownTokens) -> Pin<Box<dyn Future<Output = ()> + Send>>
```

`add()` passes the provisioned `service_id` as the first argument when it calls
`run_fn(service_id, transport, tokens)`. No wire changes required.

Callers in `builtins.rs` that wrap `run_embedded_service` (SSH agent, MQTT) update their
closure signatures to `|service_id: Uuid, transport, tokens|` and forward `service_id` as
the first argument. The scheduler closure also accepts `service_id: Uuid` in its new
signature (required by the uniform `run_fn` type) but ignores it — the scheduler calls
`run_embedded_scheduler` directly, which calls `on_connected` via the standard lifecycle.
Use `let _ = service_id;` in the scheduler closure to satisfy the deny-warnings lint.

### 2 — `run_embedded_service` builds an in-memory identity and calls `on_connected`

New signature:

```rust
pub async fn run_embedded_service<H: ServiceHandler>(
    service_id: Uuid,
    mut handler: H,
    mut transport: impl ServiceTransport,
    drain: CancellationToken,
    abort: CancellationToken,
)
```

Startup sequence change (after the existing `ServiceSettings` wait). Note: in the
standalone path, `on_connected` is called before `ServiceSettings` arrives; in the
embedded path, `ServiceSettings` is already queued by the controller before
`run_embedded_service` starts. The ordering guarantee that matters — `on_connected`
runs before `on_settings` — is identical in both paths. Handlers must not assume
`ServiceSettings` is unavailable inside `on_connected`.

```text
1. Wait for ServiceSettings  (unchanged)
2. Generate P-256 keypair in-memory:
       let keypair = match rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256) {
           Ok(kp) => kp,
           Err(e) => {
               tracing::error!(error = %e, "failed to generate embedded service keypair; aborting");
               return;
           }
       };
3. Build identity:
       let identity = ServiceIdentityState::for_embedded(service_id, keypair);
4. Call on_connected:
       if let Err(e) = handler.on_connected(&mut transport, &identity).await {
           tracing::error!(error = %e, "embedded on_connected failed; aborting");
           return;
       }
5. Compute agreed capabilities  (unchanged)
6. Call on_settings             (unchanged)
7. Enter event loop             (unchanged)
```

Key properties:

- Keypair is ephemeral — generated fresh on each controller startup. Clients always
  receive the current public key via the surface registration sent in step 6.
- P-256 algorithm is intentional: `sealed_box_decrypt` in `service-sdk/src/sensitive_params.rs`
  is hardcoded to `agreement::ECDH_P256`. Migration to P-384 (unifying with TLS keypair) is
  deferred to the follow-on spec.
- If keygen fails (should be infallible in practice), log and abort — same abort-on-error
  policy as `on_connected`.

### 3 — `ServiceIdentityState::for_embedded`

New `pub(crate)` constructor in `service-sdk/src/identity.rs`:

```rust
pub(crate) fn for_embedded(service_id: Uuid, keypair: rcgen::KeyPair) -> Self {
    Self {
        config_dir: PathBuf::new(),   // sentinel — never used for I/O
        state_dir: PathBuf::new(),    // sentinel — never used for I/O
        service_id: Some(service_id),
        tenant_id: None,
        enrollment_secret: None,
        keypair: Some(keypair),
        certificate_pem: None,
        ca_cert_pem: None,
    }
}
```

The `for_embedded` instance is used exclusively inside `run_embedded_service` to call
`handler.on_connected(&identity)` and is dropped immediately after. No disk I/O methods
(`load`, `save_*`) are ever called on it. The sentinel dirs are present only to satisfy
the struct's fields; they are never dereferenced. **Do not call any I/O method on a
`for_embedded` instance** — `PathBuf::new()` resolves to the process working directory
and would silently read/write there.

The `keypair` field carries a P-256 key when constructed via `for_embedded`, and a P-384
key when constructed via the enrollment path (`ensure_keypair`). This asymmetry is
intentional and bounded to this spec's lifetime: the follow-on ECIES migration spec will
unify both paths to P-256. Code that reads `keypair` must not assume a single algorithm.

`AgentSshRuntimeSupport::persist_tenant_id()` is unrelated: it constructs a separate,
independent `ServiceIdentityState::new_single_dir(&self.state_dir)` and loads from disk
each time it runs. That instance has no connection to the `for_embedded` one.

No new trait or wrapper type needed; the existing struct suffices.

### 4 — Handler cleanup

#### `AgentSshHandler` (`agent-ssh-runtime/src/handler.rs`)

Remove:

- `EciesKeypair` struct and its `pub` fields
- `ecies_keypair: Option<EciesKeypair>` field on `AgentSshHandler`
- The `ecies_keypair` parameter from `AgentSshHandler::new()`
- The `match &self.ecies_keypair { Some(kp) => ... }` branch in `on_connected`

After cleanup, `on_connected` always derives both values from `identity`:

```rust
async fn on_connected(&mut self, conn: &mut dyn ServiceTransport, identity: &ServiceIdentityState) -> LoopResult<()> {
    let enc_pub = identity
        .public_key_raw()
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
    self.runtime
        .on_connected(conn, SshAgentIdentity {
            service_id: identity.service_id(),
            private_key_der: identity.private_key_pkcs8_der(),
            encryption_public_key: enc_pub,
        })
        .await
        .map_err(|error| report!(LoopError::Other(error.to_string())))
}
```

`on_settings` loses the `persist_tenant_id: self.ecies_keypair.is_none()` expression
(see §5).

#### `MqttHandler` (`mqtt-runtime/src/handler.rs`)

Remove:

- `embedded_identity: Option<MqttRuntimeIdentity>` field
- `new_embedded(identity: MqttRuntimeIdentity)` constructor
- The `if let Some(identity) = self.embedded_identity.take()` block in `on_settings`

`on_connected` already reads from `identity` correctly for both modes — no change
required there. `MqttHandler::new()` becomes the only constructor.

#### `SchedulerHandler` (`scheduler-runtime/src/standalone.rs`)

No change. `on_connected` already reads `identity.service_id()` correctly. It simply
gets called now for embedded too, which is the fix.

### 5 — `persist_tenant_id` unification

Remove `persist_tenant_id: bool` from:

- `SshAgentSettings` (field + all construction sites)
- `AgentSshRuntimeSupport` (field + `new()` parameter + guard in `persist_tenant_id()`)

`AgentSshRuntimeSupport::persist_tenant_id()` becomes unconditional:

```rust
async fn persist_tenant_id(&self, tenant_id: uuid::Uuid) {
    let mut identity = ServiceIdentityState::new_single_dir(&self.state_dir);
    if let Err(error) = identity.load().await {
        tracing::warn!(error = %error, "failed to load identity for tenant_id persistence");
        return;
    }
    if let Err(error) = identity.save_tenant_id(tenant_id).await {
        tracing::warn!(error = %error, "failed to persist tenant_id to service.json");
    }
}
```

For embedded mode: `state_dir` exists but contains no `service.json` (never enrolled).
`load()` succeeds with all fields `None`. `save_tenant_id()` sees `self.service_id = None`
and returns `Ok(())` immediately — no disk write. Identical code, naturally divergent
behaviour.

`SshAgentSettings` (in `lib.rs`) loses one field; all construction sites update
accordingly.

### 6 — Controller cleanup

**`crates/core/controller-runtime/src/ssh_agent/mod.rs`**

Remove `generate_ecies_keypair()` function entirely.

**`crates/core/controller-runtime/src/mqtt/mod.rs`**

Remove the equivalent MQTT ECIES keypair generation function (mirrored pattern).

**`crates/core/controller-runtime/src/service_host/builtins.rs`** — `register_agent_ssh`

Before:

```rust
let (private_key_der, encryption_public_key) = crate::ssh_agent::generate_ecies_keypair()?;
let keypair = uptrakit_agent_ssh_runtime::EciesKeypair { private_key_der, encryption_public_key };
let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(
    db_for_ssh, state_dir, AgentSshMode::Embedded, Some(keypair),
);
// ...
move |transport, tokens| {
    Box::pin(uptrakit_service_sdk::run_embedded_service(handler, transport, tokens.drain, tokens.abort))
}
```

After:

```rust
let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(
    db_for_ssh, state_dir, AgentSshMode::Embedded,
);
// ...
move |service_id, transport, tokens| {
    Box::pin(uptrakit_service_sdk::run_embedded_service(
        service_id, handler, transport, tokens.drain, tokens.abort,
    ))
}
```

Apply same pattern to `register_mqtt` and any other embedded service registrations in
`builtins.rs`.

### 7 — `ServiceMessage::Register` for embedded

SSH agent and MQTT both send `ServiceMessage::Register` inside `on_connected`. For
embedded, this message flows through the in-process bridge — the same path that processes
all other `ServiceMessage`s from embedded services. The bridge already handles `Register`
to mark the service as actively connected in `ServiceConnectionRegistry`. No change
required.

---

## Error Handling

| Failure point                  | Action                                                                             |
| ------------------------------ | ---------------------------------------------------------------------------------- |
| P-256 keypair generation fails | `tracing::error!` + return (abort embedded service before entering event loop)     |
| `on_connected` returns `Err`   | `tracing::error!` + return (existing behaviour mirrored from standalone lifecycle) |

---

## Testing

### Unit — `service-sdk/src/identity.rs`

- `ServiceIdentityState::for_embedded` returns correct `service_id()` and non-empty
  `public_key_raw()` (65 bytes, `0x04` prefix).
- `for_embedded` returns correct `private_key_pkcs8_der()` (non-empty).

### Unit — `service-sdk/src/embedded.rs`

- `on_connected` is called **before** `on_settings`: mock `ServiceHandler` records call
  order; assert `on_connected` index < `on_settings` index.
- Abort when `on_connected` returns `Err`: verify event loop never entered and function
  returns without calling `on_settings`.
- All existing `run_embedded_service(...)` call sites in the test module must be updated
  to pass `service_id: Uuid` as the new first argument; they will not compile otherwise.

### Integration — `mqtt-runtime/src/handler.rs` tests

Existing test `embedded_mqtt_registers_surface_with_default_tenant_binding` currently
constructs `MqttHandler::new_embedded(identity)` with an explicit `MqttRuntimeIdentity`.
After this change it must be rewritten to use `MqttHandler::new()` (no identity
parameter); the test's `make_identity()` setup and the `new_embedded` call are both
removed. The SDK supplies identity via `on_connected`. Verify the test still passes
end-to-end, including that the service is visible in `ServiceConnectionRegistry` after
`on_connected` completes (confirming `ServiceMessage::Register` is processed by the
bridge before `on_settings` runs).

---

## Affected Files

| File                                                          | Change                                                                                                         |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `crates/shared/service-sdk/src/embedded.rs`                   | Add `service_id: Uuid` param; generate P-256 keypair; call `on_connected` before `on_settings`; abort on error |
| `crates/shared/service-sdk/src/identity.rs`                   | Add `pub(crate) fn for_embedded(service_id: Uuid, keypair: rcgen::KeyPair) -> Self`                            |
| `crates/shared/service-sdk/src/shared_types.rs`               | Remove doc comment on `on_connected` noting it is not called by `run_embedded_service`                         |
| `crates/core/agent-ssh-runtime/src/handler.rs`                | Remove `EciesKeypair` struct + field; simplify `on_connected`; remove `persist_tenant_id` from `on_settings`   |
| `crates/core/agent-ssh-runtime/src/lib.rs`                    | Remove `persist_tenant_id` field from `SshAgentSettings`                                                       |
| `crates/core/agent-ssh-runtime/src/runtime_support.rs`        | Remove `persist_tenant_id: bool` field + guard; unconditional `persist_tenant_id()`                            |
| `crates/core/mqtt-runtime/src/handler.rs`                     | Remove `embedded_identity` field, `new_embedded()`, workaround in `on_settings`                                |
| `crates/core/controller-runtime/src/embedded/mod.rs`          | `run_fn` signature gains `Uuid` first arg; pass provisioned `service_id`                                       |
| `crates/core/controller-runtime/src/service_host/builtins.rs` | Update all `host.add()` closures; remove ECIES keypair generation                                              |
| `crates/core/controller-runtime/src/ssh_agent/mod.rs`         | Remove `generate_ecies_keypair()`                                                                              |
| `crates/core/controller-runtime/src/mqtt/mod.rs`              | Remove MQTT ECIES keypair generation equivalent                                                                |

---

## Documentation Impact

No public API or user-facing behaviour changes. No ADR required — this is a bug fix
restoring intended behaviour, not an architectural decision with genuine alternatives.
`CONTEXT.md` glossary terms (Service, Surface, Enrollment) are unchanged.
