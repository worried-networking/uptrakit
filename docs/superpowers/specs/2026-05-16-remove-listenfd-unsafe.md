# Remove `unsafe` from `listenfd.rs`

**Date:** 2026-05-16
**Scope:** `crates/core/controller-runtime` — reexec path only
**Effort:** Small (~35 lines net change, three files)

## Problem

`clear_cloexec_raw` in `reexec/listenfd.rs:88` takes a `RawFd` — an unguarded integer with
no lifetime or validity guarantee. Converting it to `BorrowedFd` for the `nix::fcntl` call
requires:

```rust
let borrowed = unsafe { BorrowedFd::borrow_raw(fd) }; // line 95
```

This is the only `unsafe` block in `controller-runtime`. The `# Safety` contract ("caller
guarantees `fd` is valid and open") cannot be enforced by the type system.

## Root Cause

`ControllerReexecHook` stores `listener_fds: Vec<RawFd>` — raw integers extracted via
`as_raw_fd()` from `std::net::TcpListener` values before those listeners are consumed by the
server tasks. By the time `perform_reexec` fires, the compiler has no way to prove those
integers still refer to open file descriptors.

## Fix

Move cloexec-clearing to **bind time**, while the `std::net::TcpListener` is still in scope.
`std::net::TcpListener` implements `AsFd`; calling `as_fd()` on it is a safe, lifetime-checked
borrow. The `unsafe` block is eliminated entirely.

The fd-iteration loop in `perform_reexec` (whose only job was calling `clear_cloexec_raw` on
each stored raw fd) is also removed — the fds are already non-cloexec when `perform_reexec`
runs.

**Safety invariant:** `controller-runtime` spawns no other child processes. Verified by
grep: `Command::new` appears only in `reexec/mod.rs` (the reexec itself). No risk of
unintended fd inheritance by unrelated children from clearing cloexec at bind time.

**Failure semantics:** `clear_cloexec` failure is fatal — propagated via `?` and aborts
startup. This is intentional: a listener whose cloexec flag cannot be cleared cannot
participate in reexec inheritance, making the process unusable for that purpose. Failing
fast at bind time is safer than discovering the problem at reexec time.

## Changes

### 1. `crates/core/controller-runtime/src/reexec/listenfd.rs`

Replace `clear_cloexec_raw` with a safe generic function. Preserve the fd number in the
error message (obtained via `fd.as_fd().as_raw_fd()`) for diagnosability:

```rust
// Before
pub(crate) fn clear_cloexec_raw(fd: std::os::unix::io::RawFd) -> Result<(), Report> {
    use std::os::unix::io::BorrowedFd;
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) }; // unsafe
    fcntl(borrowed, FcntlArg::F_SETFD(FdFlag::empty()))
        .map_err(|e| rootcause::report!("fcntl F_SETFD on fd {fd}: {e}"))?;
    Ok(())
}

// After
pub(crate) fn clear_cloexec(fd: impl std::os::unix::io::AsFd) -> Result<(), Report> {
    use std::os::unix::io::AsRawFd as _;
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    let raw = fd.as_fd().as_raw_fd();
    fcntl(fd, FcntlArg::F_SETFD(FdFlag::empty()))
        .map_err(|e| rootcause::report!("fcntl F_SETFD on fd {raw}: {e}"))?;
    Ok(())
}
```

Use `impl AsFd` (not `&F: AsFd`) — matches `nix::fcntl::fcntl`'s own by-value signature
and avoids unnecessary indirection. Drop the `# Safety` doc section and the `BorrowedFd`
import. Remove `clear_cloexec_raw` entirely — no callers will remain.

### 2. `crates/core/controller-runtime/src/lib.rs`

**a. Call `clear_cloexec` immediately after each listener is obtained.**

For fresh-bound listeners (after `set_nonblocking`):

```rust
let https_std = std::net::TcpListener::bind(reconciled.https_addr)?;
https_std.set_nonblocking(true)?;
reexec::listenfd::clear_cloexec(&https_std)?;
```

For inherited listeners (after `into_std()`):

```rust
let https_std = s.https.into_std()?;
// Required — not optional. The parent cleared cloexec for *this* generation's exec.
// This call clears it for the *next* generation's exec. Without it, every reexec
// beyond generation 1 fails: the child's LISTEN_FDS slots arrive empty because the
// kernel closes all FD_CLOEXEC fds on exec.
reexec::listenfd::clear_cloexec(&https_std)?;
```

Apply the same pattern to `pki_std` (both fresh-bind and inherited branches).

**b. Remove intermediate raw-fd `let` bindings.**

The `let https_raw_fd = https_std.as_raw_fd();` binding (and the equivalent `let pki_fd =
pki_std.as_raw_fd();`) become unused once `listener_fds` is removed. Delete both bindings
and the `use std::os::unix::io::AsRawFd as _;` import at the top of the file.

**c. Replace `listener_fds: Vec<RawFd>` with `listener_count: usize`.**

The local variable:

```rust
// Before
let (listener_fds, pki_std_for_spawn): (Vec<std::os::unix::io::RawFd>, Option<std::net::TcpListener>) = ...;

// After — `pki_enabled` = `validated.pki_http_port.is_some()`
let (listener_count, pki_std_for_spawn): (usize, Option<std::net::TcpListener>) = if pki_enabled {
    (2, Some(pki_std))
} else {
    (1, None)
};
```

The hook struct field (update the doc comment to be accurate):

```rust
// Before
struct ControllerReexecHook {
    ...
    /// Raw listener FDs cleared of `FD_CLOEXEC` before `exec()`.
    /// Empty when PKI HTTP is disabled; the child re-binds in that case.
    listener_fds: Vec<std::os::unix::io::RawFd>,
}

// After
struct ControllerReexecHook {
    ...
    /// Number of bound listeners passed via `LISTEN_FDS` to the child process.
    /// 1 when PKI HTTP is disabled, 2 when enabled.
    listener_count: usize,
}
```

Hook construction and `check_and_trigger`:

```rust
// Before
b.coordinator.set_reexec_hook(Box::new(ControllerReexecHook {
    ...
    listener_fds: listener_fds.clone(),
}));
// in check_and_trigger:
let plan = ReexecPlan { listener_count: self.listener_fds.len(), ... };
reexec::perform_reexec(&plan, &self.listener_fds)

// After
b.coordinator.set_reexec_hook(Box::new(ControllerReexecHook {
    ...
    listener_count,
}));
// in check_and_trigger:
let plan = ReexecPlan { listener_count: self.listener_count, ... };
reexec::perform_reexec(&plan)
```

### 3. `crates/core/controller-runtime/src/reexec/mod.rs`

Remove `listener_fds` parameter from `perform_reexec`, drop the fd-clearing loop, and
update the function's doc comment to remove the now-inaccurate step 1 ("Clears `FD_CLOEXEC`
on each entry in `listener_fds`"):

```rust
// Before
pub(crate) fn perform_reexec(
    plan: &ReexecPlan,
    listener_fds: &[std::os::unix::io::RawFd],
) -> Result<std::convert::Infallible, Report> {
    for &fd in listener_fds {
        listenfd::clear_cloexec_raw(fd)?;
    }
    // ... build cmd, exec
}

// After — doc comment step 1 removed; replace with:
// "1. Constructs a `Command` equivalent to the original invocation."
pub(crate) fn perform_reexec(
    plan: &ReexecPlan,
) -> Result<std::convert::Infallible, Report> {
    // FD_CLOEXEC cleared at bind time — nothing to do here.
    // ... build cmd, exec
}
```

## Documentation Impact

No public API change. No ADR references `clear_cloexec_raw` by name (verified across all
15 ADRs in `docs/adr/`). The `ARCHITECTURE.md` describes the reload/reexec flow at a
higher level without mentioning fd flag manipulation. No external documentation updates
required.

## Out of Scope

- PKI listener reload path (`reload/pki_listener.rs`) — uses a separate binding mechanism;
  not part of the reexec inheritance chain.
- Any change to `server.rs`, `spawn_pki_http`, or the `axum_server`/`axum::serve` call sites.
- Platform guards — `fcntl` and `O_CLOEXEC` are POSIX; the reexec path is already
  Linux/macOS only.

## Testing

The change is mechanical; the observable behavior (child process inherits the bound sockets)
is unchanged. Existing integration tests cover the single-reexec path end-to-end
(`cargo test -p uptrakit-integration-tests -- --ignored`).

The inherited-socket branch (`clear_cloexec` on an already-inherited fd) is the mechanism
that enables reexec beyond generation 1. If the existing integration test suite does not
perform two sequential reexecs (generation 0 → 1 → 2 with socket continuity verified at
both hops), add one. A single-hop test does not exercise this branch.

Quality gates: `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --all-features`.
