# Remove `unsafe` from `listenfd.rs` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the only `unsafe` block in `controller-runtime` by replacing
`clear_cloexec_raw(fd: RawFd)` with `clear_cloexec(fd: impl AsFd)` and moving the
`FD_CLOEXEC` clearing call to bind time.

**Architecture:** `std::net::TcpListener` implements `AsFd`; calling `as_fd()` on a live
listener is a safe, lifetime-checked borrow — no `unsafe` needed. Both listeners (HTTPS and
PKI, inherited or fresh) get their `FD_CLOEXEC` flag cleared immediately after they are
obtained, before being handed off to the server tasks. `ControllerReexecHook` no longer
stores raw fd integers; it stores `listener_count: usize` instead.

**Tech Stack:** Rust 2021, `nix 0.31.3` (`nix::fcntl::{FcntlArg, FdFlag, fcntl}`),
`std::os::unix::io::{AsFd, AsRawFd, BorrowedFd}`.

---

## File Map

| File                                                    | Change                                                                                      |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `crates/core/controller-runtime/src/reexec/listenfd.rs` | Replace `clear_cloexec_raw` with `clear_cloexec`                                            |
| `crates/core/controller-runtime/src/reexec/mod.rs`      | Remove `listener_fds` param + cloexec loop; update doc                                      |
| `crates/core/controller-runtime/src/lib.rs`             | Clear cloexec at bind time; replace `listener_fds: Vec<RawFd>` with `listener_count: usize` |

> **Note:** All three files must be edited before `cargo check` will pass. Tasks 1–3 are
> interdependent — the crate won't compile until Task 3 is complete.

---

### Task 1: Replace `clear_cloexec_raw` with `clear_cloexec` in `listenfd.rs`

**Files:**

- Modify: `crates/core/controller-runtime/src/reexec/listenfd.rs:72-99`

- [ ] **Step 1: Open the file and locate `clear_cloexec_raw`**

  ```text
  crates/core/controller-runtime/src/reexec/listenfd.rs
  ```

  The function starts at line 88 and occupies lines 88–99:

  ```rust
  /// Clear the `FD_CLOEXEC` flag on a raw file descriptor so it survives `exec()`.
  ///
  /// Called by the reexec path for each bound listener before replacing the
  /// process image.  Without this step the OS closes all `O_CLOEXEC` descriptors
  /// on exec and the new process image receives empty `LISTEN_FDS` slots.
  ///
  /// # Errors
  ///
  /// Returns an error if the `fcntl` call fails (e.g. the file descriptor is
  /// invalid or not open).
  ///
  /// # Safety
  ///
  /// The caller must ensure `fd` is a valid, open file descriptor for the
  /// lifetime of this call.  `RawFd` is unguarded; passing a closed or
  /// reused descriptor yields undefined behaviour in the kernel call.
  pub(crate) fn clear_cloexec_raw(fd: std::os::unix::io::RawFd) -> Result<(), Report> {
      use std::os::unix::io::BorrowedFd;

      use nix::fcntl::{FcntlArg, FdFlag, fcntl};

      // SAFETY: The caller guarantees `fd` is valid and open.  We borrow it
      // only for the duration of the `fcntl` call.
      let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
      fcntl(borrowed, FcntlArg::F_SETFD(FdFlag::empty()))
          .map_err(|e| rootcause::report!("fcntl F_SETFD on fd {fd}: {e}"))?;
      Ok(())
  }
  ```

- [ ] **Step 2: Replace `clear_cloexec_raw` with `clear_cloexec`**

  Delete the entire `clear_cloexec_raw` function (doc comment + body, lines 72–99) and
  write the following in its place:

  ```rust
  /// Clear the `FD_CLOEXEC` flag on a listener so it survives `exec()`.
  ///
  /// Called at bind time (and after `into_std()` for inherited sockets) so that
  /// each bound listener is non-cloexec from the moment it is ready.  Both
  /// paths are required: the fresh-bind path clears the flag for the first exec,
  /// and the inherited-socket path clears it for every subsequent exec
  /// (generation N → N+1).  Without the inherited call, every reexec beyond
  /// the first would fail silently — the kernel would close all `FD_CLOEXEC`
  /// descriptors and the child would receive empty `LISTEN_FDS` slots.
  ///
  /// `T: AsFd` means the compiler enforces that the file descriptor is valid and
  /// open for the lifetime of the call — no `unsafe` required.
  ///
  /// # Errors
  ///
  /// Returns an error if the underlying `fcntl(F_SETFD)` syscall fails.
  pub(crate) fn clear_cloexec(fd: impl std::os::unix::io::AsFd) -> Result<(), Report> {
      use std::os::unix::io::AsRawFd as _;

      use nix::fcntl::{FcntlArg, FdFlag, fcntl};

      let raw = fd.as_fd().as_raw_fd(); // capture for error message before move
      fcntl(fd, FcntlArg::F_SETFD(FdFlag::empty()))
          .map_err(|e| rootcause::report!("fcntl F_SETFD on fd {raw}: {e}"))?;
      Ok(())
  }
  ```

  Key differences from the old function:
  - Signature: `fd: impl std::os::unix::io::AsFd` instead of `fd: RawFd` — matches
    `nix::fcntl::fcntl`'s own by-value signature, no `&F` indirection needed
  - `let raw = fd.as_fd().as_raw_fd()` — capture the fd number for the error message
    _before_ the move into `fcntl`, so diagnostics remain useful
  - No `unsafe` block, no `BorrowedFd::borrow_raw`
  - Updated doc comment explaining why both fresh and inherited paths are required

- [ ] **Step 3: Verify no other code in this file needs updating**

  The file also contains `take_inherited_listeners`, `current_generation`. Neither calls
  `clear_cloexec_raw`. No other changes needed in this file.

---

### Task 2: Update `perform_reexec` in `reexec/mod.rs`

**Files:**

- Modify: `crates/core/controller-runtime/src/reexec/mod.rs:29-75`

- [ ] **Step 1: Replace the `perform_reexec` doc comment and signature**

  Current doc comment (lines 29–47):

  ```rust
  /// Replace the current process image with a new instance of the same binary.
  ///
  /// This function:
  /// 1. Clears `FD_CLOEXEC` on each entry in `listener_fds` so the descriptors
  ///    survive `exec()` and the new process image can claim them via `LISTEN_FDS`.
  /// 2. Constructs a `Command` equivalent to the original invocation.
  /// 3. Forwards `LISTEN_FDS` / `LISTEN_PID` so the new process can inherit
  ///    the already-bound TCP sockets.
  /// 4. Sets `UPTRAKIT_REEXEC_GENERATION` so observability tooling can track
  ///    how many times the process has re-execed.
  /// 5. Calls `exec()` which, on success, replaces the process image and never
  ///    returns.  On failure the OS error is wrapped and returned.
  ///
  /// # Errors
  ///
  /// Returns an error if clearing `FD_CLOEXEC` fails or if `exec()` fails
  /// (e.g. the binary path is no longer accessible).  The error is always
  /// non-fatal from the caller's perspective because the original process is
  /// still running.
  ```

  Replace with:

  ```rust
  /// Replace the current process image with a new instance of the same binary.
  ///
  /// This function:
  /// 1. Constructs a `Command` equivalent to the original invocation.
  /// 2. Forwards `LISTEN_FDS` / `LISTEN_PID` so the new process can inherit
  ///    the already-bound TCP sockets (their `FD_CLOEXEC` flag is cleared at
  ///    bind time — see [`super::listenfd::clear_cloexec`]).
  /// 3. Sets `UPTRAKIT_REEXEC_GENERATION` so observability tooling can track
  ///    how many times the process has re-execed.
  /// 4. Calls `exec()` which, on success, replaces the process image and never
  ///    returns.  On failure the OS error is wrapped and returned.
  ///
  /// # Errors
  ///
  /// Returns an error if `exec()` fails (e.g. the binary path is no longer
  /// accessible).  The error is always non-fatal from the caller's perspective
  /// because the original process is still running.
  ```

- [ ] **Step 2: Replace the `perform_reexec` signature and body**

  Current (lines 48–75):

  ```rust
  pub(crate) fn perform_reexec(
      plan: &ReexecPlan,
      listener_fds: &[std::os::unix::io::RawFd],
  ) -> Result<std::convert::Infallible, Report> {
      use std::os::unix::process::CommandExt as _;

      // Clear FD_CLOEXEC on each listener so it survives exec().
      for &fd in listener_fds {
          listenfd::clear_cloexec_raw(fd)?;
      }

      let mut cmd = std::process::Command::new(&plan.current_exe);
      ...
  }
  ```

  Replace with:

  ```rust
  pub(crate) fn perform_reexec(
      plan: &ReexecPlan,
  ) -> Result<std::convert::Infallible, Report> {
      use std::os::unix::process::CommandExt as _;

      // FD_CLOEXEC cleared at bind time — nothing to do here before exec.

      let mut cmd = std::process::Command::new(&plan.current_exe);
      cmd.arg("--config").arg(&plan.config_path);

      if let Some(mk) = &plan.master_key_file {
          cmd.arg("--master-key-file").arg(mk);
      }

      cmd.env("LISTEN_FDS", plan.listener_count.to_string());
      cmd.env("LISTEN_PID", std::process::id().to_string());
      cmd.env(
          "UPTRAKIT_REEXEC_GENERATION",
          (plan.generation + 1).to_string(),
      );

      let err = cmd.exec();
      Err(rootcause::report!("exec failed: {err}"))
  }
  ```

  The `cmd.env(...)` / `cmd.exec()` block is unchanged; only the `listener_fds` parameter
  and the cloexec loop are removed.

---

### Task 3: Update `lib.rs` — clear cloexec at bind time, replace `listener_fds` with `listener_count`

This task has four sub-changes. Make all four before running `cargo check`.

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs`

#### 3a — Remove the `AsRawFd` import

- [ ] **Step 1: Find and delete line 48**

  ```rust
  // DELETE this line:
  use std::os::unix::io::AsRawFd as _;
  ```

#### 3b — Call `clear_cloexec` on the HTTPS listener

- [ ] **Step 2: Locate the HTTPS listener match block (~line 422–436)**

  It looks like:

  ```rust
  // Pre-bind HTTPS socket so we have the raw FD for reexec listener inheritance.
  let https_std = match inherited_https {
      Some(l) => l,
      None => {
          let l = std::net::TcpListener::bind(reconciled.https_addr).map_err(|e| {
              report!(AppError::Config(format!(
                  "bind HTTPS {}: {e}",
                  reconciled.https_addr
              )))
          })?;
          l.set_nonblocking(true)
              .map_err(|e| report!(AppError::Config(format!("set_nonblocking HTTPS: {e}"))))?;
          l
      }
  };
  let https_raw_fd = https_std.as_raw_fd();
  ```

- [ ] **Step 3: Replace with the clear_cloexec call; remove `https_raw_fd`**

  ```rust
  // Pre-bind HTTPS socket and clear FD_CLOEXEC so it survives reexec exec().
  let https_std = match inherited_https {
      Some(l) => l,
      None => {
          let l = std::net::TcpListener::bind(reconciled.https_addr).map_err(|e| {
              report!(AppError::Config(format!(
                  "bind HTTPS {}: {e}",
                  reconciled.https_addr
              )))
          })?;
          l.set_nonblocking(true)
              .map_err(|e| report!(AppError::Config(format!("set_nonblocking HTTPS: {e}"))))?;
          l
      }
  };
  reexec::listenfd::clear_cloexec(&https_std)
      .map_err(|e| report!(AppError::Config(format!("clear_cloexec HTTPS: {e}"))))?;
  ```

  The `let https_raw_fd = https_std.as_raw_fd();` line is **deleted** (not replaced).

#### 3c — Replace the `listener_fds` Vec with `listener_count` and add PKI `clear_cloexec`

- [ ] **Step 4: Locate the PKI block (~line 439–460)**

  ```rust
  // Pre-bind PKI socket and collect listener FDs for reexec inheritance.
  let (listener_fds, pki_std_for_spawn): (
      Vec<std::os::unix::io::RawFd>,
      Option<std::net::TcpListener>,
  ) = if let Some(pki_port) = validated.pki_http_port {
      let pki_std = match inherited_pki {
          Some(l) => l,
          None => {
              let addr = std::net::SocketAddr::from(([0, 0, 0, 0], pki_port));
              let l = std::net::TcpListener::bind(addr)
                  .map_err(|e| report!(AppError::Config(format!("bind PKI HTTP {addr}: {e}"))))?;
              l.set_nonblocking(true)
                  .map_err(|e| report!(AppError::Config(format!("set_nonblocking PKI: {e}"))))?;
              l
          }
      };
      let pki_fd = pki_std.as_raw_fd();
      (vec![https_raw_fd, pki_fd], Some(pki_std))
  } else {
      // PKI disabled: only HTTPS socket inherited (1 FD).
      (vec![https_raw_fd], None)
  };
  ```

- [ ] **Step 5: Replace with `listener_count` and `clear_cloexec` for PKI**

  ```rust
  // Pre-bind PKI socket (if enabled) and clear FD_CLOEXEC for reexec inheritance.
  let (listener_count, pki_std_for_spawn): (usize, Option<std::net::TcpListener>) =
      if let Some(pki_port) = validated.pki_http_port {
          let pki_std = match inherited_pki {
              Some(l) => l,
              None => {
                  let addr = std::net::SocketAddr::from(([0, 0, 0, 0], pki_port));
                  let l = std::net::TcpListener::bind(addr)
                      .map_err(|e| report!(AppError::Config(format!("bind PKI HTTP {addr}: {e}"))))?;
                  l.set_nonblocking(true)
                      .map_err(|e| report!(AppError::Config(format!("set_nonblocking PKI: {e}"))))?;
                  l
              }
          };
          reexec::listenfd::clear_cloexec(&pki_std)
              .map_err(|e| report!(AppError::Config(format!("clear_cloexec PKI: {e}"))))?;
          (2, Some(pki_std))
      } else {
          // PKI disabled: only HTTPS socket inherited (1 FD).
          (1, None)
      };
  ```

  Changes from the old block:
  - Tuple type: `(Vec<RawFd>, Option<TcpListener>)` → `(usize, Option<TcpListener>)`
  - `let pki_fd = pki_std.as_raw_fd();` deleted
  - `reexec::listenfd::clear_cloexec(&pki_std)` call added after match
  - `(vec![https_raw_fd, pki_fd], Some(pki_std))` → `(2, Some(pki_std))`
  - `(vec![https_raw_fd], None)` → `(1, None)`

#### 3d — Update `ControllerReexecHook` struct and its usages

- [ ] **Step 6: Update the `ControllerReexecHook` struct field (~line 105–108)**

  Current:

  ```rust
  struct ControllerReexecHook {
      /// Resolved from `std::env::current_exe()` at startup.
      current_exe: std::path::PathBuf,
      config_path: std::path::PathBuf,
      master_key_file: Option<String>,
      generation: u64,
      /// Raw listener FDs cleared of `FD_CLOEXEC` before `exec()`.
      /// Empty when PKI HTTP is disabled; the child re-binds in that case.
      listener_fds: Vec<std::os::unix::io::RawFd>,
  }
  ```

  Replace with:

  ```rust
  struct ControllerReexecHook {
      /// Resolved from `std::env::current_exe()` at startup.
      current_exe: std::path::PathBuf,
      config_path: std::path::PathBuf,
      master_key_file: Option<String>,
      generation: u64,
      /// Number of bound listeners passed via `LISTEN_FDS` to the child process.
      /// 1 when PKI HTTP is disabled, 2 when enabled.
      listener_count: usize,
  }
  ```

- [ ] **Step 7: Update `check_and_trigger` (~lines 122–134)**

  Current:

  ```rust
  let plan = reexec::ReexecPlan {
      current_exe: self.current_exe.clone(),
      config_path: self.config_path.clone(),
      master_key_file: self.master_key_file.clone(),
      listener_count: self.listener_fds.len(),
      generation: self.generation,
  };

  match reexec::perform_reexec(&plan, &self.listener_fds) {
      Ok(infallible) => match infallible {},
      Err(e) => ReexecOutcome::ExecFailed(e),
  }
  ```

  Replace with:

  ```rust
  let plan = reexec::ReexecPlan {
      current_exe: self.current_exe.clone(),
      config_path: self.config_path.clone(),
      master_key_file: self.master_key_file.clone(),
      listener_count: self.listener_count,
      generation: self.generation,
  };

  match reexec::perform_reexec(&plan) {
      Ok(infallible) => match infallible {},
      Err(e) => ReexecOutcome::ExecFailed(e),
  }
  ```

- [ ] **Step 8: Update hook construction (~line 844–850)**

  Current:

  ```rust
  b.coordinator
      .set_reexec_hook(Box::new(ControllerReexecHook {
          current_exe,
          config_path: config_path_for_coord.clone(),
          master_key_file: args.master_key_from.clone(),
          generation: reexec::listenfd::current_generation(),
          listener_fds: listener_fds.clone(),
      }));
  ```

  Replace with:

  ```rust
  b.coordinator
      .set_reexec_hook(Box::new(ControllerReexecHook {
          current_exe,
          config_path: config_path_for_coord.clone(),
          master_key_file: args.master_key_from.clone(),
          generation: reexec::listenfd::current_generation(),
          listener_count,
      }));
  ```

---

### Task 4: Compile check

- [ ] **Step 1: Run `cargo check` on the crate**

  ```bash
  cargo check -p uptrakit-controller-runtime --no-default-features --features db-sqlite
  ```

  Expected: `Finished` with zero errors and zero warnings. If you see:
  - `cannot find function 'clear_cloexec_raw'` → Task 1 or 2 missed a call site
  - `unused import: std::os::unix::io::AsRawFd` → Step 3a not completed
  - `expected 2 arguments, found 1` on `perform_reexec` → Task 3d Step 8 incomplete
  - `no field 'listener_fds'` → Task 3d Steps 6–8 incomplete

- [ ] **Step 2: Run `cargo check` with all features**

  ```bash
  cargo check -p uptrakit-controller-runtime --all-features
  ```

  Expected: `Finished` with zero errors.

---

### Task 5: Quality gates + commit

- [ ] **Step 1: Format**

  ```bash
  cargo fmt --all
  ```

  Expected: no output (already formatted), or minimal whitespace normalisation.

- [ ] **Step 2: Clippy (SQLite)**

  ```bash
  cargo clippy -p uptrakit-controller-runtime --no-default-features --features db-sqlite -- -D warnings
  ```

  Expected: no warnings. If you see `unused import` or `dead_code`, re-check Tasks 1–3.

- [ ] **Step 3: Clippy (all features)**

  ```bash
  cargo clippy -p uptrakit-controller-runtime --all-features -- -D warnings
  ```

  Expected: no warnings.

- [ ] **Step 4: Unit tests**

  ```bash
  cargo test -p uptrakit-controller-runtime --all-features
  ```

  Expected: all tests pass. The `reexec::triage` tests (the only tests in this crate's
  reexec module) should all still pass — they test config-change triage, not the fcntl path.

- [ ] **Step 5: Check integration test suite for two-generation reexec coverage**

  Open `crates/core/integration-tests/tests/system.rs` and search for any test that:
  - triggers reexec twice (generation 0 → 1 → 2), and
  - verifies socket continuity after the second exec

  If no such test exists: add a `#[tokio::test] #[ignore]` test named
  `reexec_two_generations_inherit_sockets` in `system.rs` that:
  1. Starts the controller (generation 0)
  2. Triggers a config-change reexec → waits for generation 1 to be healthy
  3. Triggers a second config-change reexec → waits for generation 2 to be healthy
  4. Verifies the HTTPS port responds after both transitions

  If integration tests in this repo use a harness or helper (check
  `crates/core/integration-tests/src/lib.rs`), follow the same pattern for
  controller startup.

- [ ] **Step 6: Commit**

  ```bash
  git add \
    crates/core/controller-runtime/src/reexec/listenfd.rs \
    crates/core/controller-runtime/src/reexec/mod.rs \
    crates/core/controller-runtime/src/lib.rs
  git commit -m "$(cat <<'EOF'
  refactor(reexec): eliminate unsafe BorrowedFd::borrow_raw in listenfd.rs

  Replace clear_cloexec_raw(RawFd) with clear_cloexec(impl AsFd). Call it at
  bind time on std::net::TcpListener — safe lifetime-checked borrow, no unsafe.

  Remove listener_fds: Vec<RawFd> from ControllerReexecHook and perform_reexec.
  Replace with listener_count: usize. The fd-clearing loop in perform_reexec
  is deleted; sockets are already non-cloexec when exec() fires.

  Verified: mio::TcpListener::from_std and tokio::TcpListener::from_std do not
  re-set FD_CLOEXEC (checked in mio 1.2.0 source — IoSource::new is a plain
  wrapper, no fcntl calls).

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Self-Review

**Spec coverage:**

| Spec requirement                                                 | Covered by                        |
| ---------------------------------------------------------------- | --------------------------------- |
| `clear_cloexec_raw` → `clear_cloexec(impl AsFd)`                 | Task 1                            |
| `fd.as_fd().as_raw_fd()` for error message                       | Task 1, Step 2                    |
| Call on `https_std` after bind/`into_std`                        | Task 3b                           |
| Call on `pki_std` after bind/`into_std`                          | Task 3c                           |
| Remove `https_raw_fd` / `pki_fd` bindings                        | Task 3b Step 3, Task 3c Step 5    |
| Remove `AsRawFd` import                                          | Task 3a                           |
| `listener_fds: Vec<RawFd>` → `listener_count: usize` (local var) | Task 3c Step 5                    |
| `ControllerReexecHook` field + doc comment                       | Task 3d Step 6                    |
| Hook construction update                                         | Task 3d Step 8                    |
| `check_and_trigger` update                                       | Task 3d Step 7                    |
| `perform_reexec` signature + loop removal                        | Task 2 Step 2                     |
| `perform_reexec` doc comment updated                             | Task 2 Step 1                     |
| No doc impact (internal, no public API)                          | Noted in spec; no doc tasks added |
| Two-generation reexec test                                       | Task 5 Step 5                     |

**No placeholders found.** All steps show exact code.

**Type consistency:** `clear_cloexec` is defined in Task 1 as
`pub(crate) fn clear_cloexec(fd: impl AsFd)` and called as
`reexec::listenfd::clear_cloexec(&https_std)` in Task 3b and
`clear_cloexec(&pki_std)` in Task 3c. `&https_std` where
`https_std: std::net::TcpListener` — `&TcpListener` implements `AsFd` via
the blanket `impl<T: AsFd> AsFd for &T`. Consistent throughout.
