# Journald-Native Logging — Design

- **Date:** 2026-08-16
- **Status:** Draft (pending review)
- **Scope:** `uptrakit-tracing-init`, `uptrakit-controller-runtime` (audit-layer dedup), docs, one new ADR

## Problem

Under systemd, every daemon's `tracing_subscriber::fmt` output reaches journald as opaque text lines on stdout:

```text
Aug 14 22:31:41 uptrakit uptrakit-controller-standalone[1007]: 2026-08-14T21:31:41.724566Z DEBUG uptrakit_agent_ssh_runtime::ssh_pool: evicted SSH session from pool host_id=019fffe6-…
```

Defects:

- **Duplicate timestamp** — journald stamps every entry; the fmt timestamp is noise.
- **No priority mapping** — every line lands at journald's default priority, so `journalctl -p err` / `-p warning` filtering is useless.
- **No structured fields** — `host_id=…` pairs are flat text; journald field filtering (`journalctl FIELD=value`) is unavailable.
- **Level/target as prose** — `DEBUG uptrakit_agent_ssh_runtime::ssh_pool:` prefixes every message.
- **ANSI escapes stored in the journal** — `fmt::layer()` defaults `with_ansi(true)` with no tty check, so every stored
  `MESSAGE` embeds color codes (live-verified on the deployment:
  `'\x1b[2m2026-08-16T08:42:35.855792Z\x1b[0m \x1b[34mDEBUG\x1b[0m …'`; journalctl JSON output degrades such messages to
  byte arrays, and `-o cat`/`-o json` consumers see raw escapes).

Interactive terminal runs are fine as-is and must keep the current format.

## Decisions (settled in grilling, 2026-08-16)

| Decision         | Choice                                                                                                                    |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Mechanism        | Native `tracing_journald::Layer` replaces the fmt layer when stdout is the journal                                        |
| Detection        | `JOURNAL_STREAM` env var (`dev:inode`, systemd.exec(5) contract) verified by `fstat(stdout)` dev:inode equality           |
| Scope            | Every `TracingBuilder::init()` daemon: controller(+standalone), agent, agent-ssh, scheduler, mqtt                         |
| Override         | `UPTRAKIT_LOG_FORMAT=auto\|text\|journald` env var; default `auto`; no new CLI flag (controller flag budget), no TOML key |
| Priority mapping | Upstream lossless default: ERROR→3, WARN→4, INFO→5 (Notice), DEBUG→6, TRACE→7                                             |
| Field prefix     | Upstream default `F` (`F_HOST_ID=…`) — collision-safe with journald reserved fields                                       |
| Dependency shape | `tracing-journald` becomes an unconditional dependency of `uptrakit-tracing-init`                                         |
| Audit dedup      | Exact-match `filter_fn` excludes target `uptrakit_audit` from the main journald layer on the controller                   |
| Record           | Short ADR via `adrs new`                                                                                                  |

## Design

### Log-format resolution (`uptrakit-tracing-init`)

New types in `crates/shared/tracing-init/src/lib.rs` (or a small submodule):

- `LogFormat { Auto, Text, Journald }` with `FromStr` + typed `ParseLogFormatError` (project `FromStr` rule; no ad-hoc parse fns).
- `JournalStream { dev: u64, ino: u64 }` with `FromStr` + typed error, parsing the `JOURNAL_STREAM` value `"<dev>:<inode>"`
  (decimal, colon-separated — systemd.exec(5) contract).

Resolution inside `TracingBuilder::init()`:

1. Read `UPTRAKIT_LOG_FORMAT`. Unset ⇒ `Auto`. Invalid value ⇒ `eprintln!` warning + `Auto`
   (lenient-parse precedent: `RUST_LOG` handling, `crates/shared/tracing-init/src/lib.rs:146-161`).
2. `Auto` ⇒ journald mode iff `stdout_is_journal()`; `Text` ⇒ fmt layer always; `Journald` ⇒ journald mode forced.

Detection is split for testability (pure-helper pattern, cf. commit 9d5a1947e):

- `fn journal_stream_matches(env_value: Option<&str>, stdout_stat: Option<(u64, u64)>) -> bool` — pure, fully unit-tested.
- `fn stdout_is_journal() -> bool` — thin wrapper: reads the env var, `rustix::fs::fstat` on stdout, delegates. Body is
  `#[cfg(unix)]`; non-unix compiles to `false` (platform `cfg`, not a cargo feature — the additive-features rule is about
  feature predicates).

The fmt layer's default writer is stdout, so stdout (not stderr) is the stream whose identity matters.

### Layer construction in `init()`

```text
resolved format == journald:
    tracing_journald::Layer::new()
        Ok(layer)  -> install journald layer (with EnvFilter + exclusions, below)
        Err(e)     -> eprintln! warning, fall back to fmt text layer
resolved format == text:
    fmt layer as today, plus .with_ansi(std::io::stdout().is_terminal())
```

Text-mode addition (post-grilling, motivated by the live ANSI finding above): gate ANSI on `std::io::IsTerminal`
(stable since 1.70; workspace `rust-version = "1.91"`). Interactive terminals keep colors byte-identically; redirected
stdout (docker logs, files, journald-with-detection-overridden-to-text) gets clean plain text.

- `Layer::new()` probes the socket and fails with `NotFound` when journald is absent or on non-unix
  (`tracing-journald-0.3.2/src/lib.rs:98+`) — covers forced `journald` on hosts without systemd. Logging setup must never
  abort a daemon; `eprintln!` is the correct channel pre-subscriber (existing precedent at `lib.rs:123,157`).
- Upstream defaults kept verbatim: priority mappings (`lib.rs:64-70`), field prefix `F` (`lib.rs:109`),
  `SYSLOG_IDENTIFIER` from argv\[0].
- The same `EnvFilter` (verbosity directives + `RUST_LOG`, unchanged precedence) filters whichever layer is installed.
- `extra_layer()` stack unchanged.
- `build_filter()`, `init_cli_tracing` (stderr, interactive), and `init_test_tracing` are untouched.

### Controller audit-layer dedup

`EnvFilter` directive matching is bare `starts_with` with no `::` boundary
(`tracing-subscriber-0.3.23/src/filter/directive.rs:182,250`), so the verbosity directive `uptrakit=…` matches target
`uptrakit_audit`. With the main layer switched to journald and the controller's dedicated `uptrakit_audit` journald layer
compiled in (`crates/core/controller-runtime/src/boot/config.rs:129-139`), audit events would reach the journal twice.

Fix — keep the dedicated layer as the guaranteed audit-delivery path (immune to `RUST_LOG=uptrakit_audit=off`), and exclude
the target from the main layer:

- New builder method `TracingBuilder::journald_exclude_exact(target: &'static str)` collecting exact-match target strings.
- When (and only when) the journald main layer is installed, its filter becomes
  `env_filter.and(filter_fn(move |meta| !excluded.contains(meta.target())))`
  (`FilterExt::and` + `filter_fn`, tracing-subscriber `filter/layer_filters/mod.rs:224`, `filter/filter_fn.rs:104`).
- Exact match, not prefix: crate-internal events with targets like `uptrakit_audit_log::…` are unaffected.
- `controller-runtime` calls `.journald_exclude_exact("uptrakit_audit")` inside the existing `#[cfg(feature = "journald")]`
  block that adds the audit layer. Feature absent ⇒ no dedicated layer, no exclusion.
- Text mode: exclusion not applied — stdout keeps today's behavior (audit lines visible in plain output).

### Dependencies

No new registry entries; both crates already sit in `[workspace.dependencies]` at their latest stable:

- `tracing-journald = "0.3"` (resolves 0.3.2, latest published) — flips from `controller-runtime`-only optional to an
  unconditional dependency of `uptrakit-tracing-init`. Compiles on all targets (socket internals are `cfg(unix)`).
  `controller-runtime` keeps its own optional dep + `journald` feature for the audit layer — semantics unchanged.
- `rustix = { version = "1", default-features = false }` (root `Cargo.toml:113`; latest 1.1.4 satisfies `"1"`) — added to
  `uptrakit-tracing-init` as `features = ["fs", "std"]`: `rustix::fs::fstat` lives behind `fs`
  (rustix-1.1.4 `src/fs/fd.rs:156`), and `std` must be named explicitly because the workspace entry disables defaults
  (`crates/shared/command/Cargo.toml:21` is the in-repo precedent for per-crate rustix features).

No new cargo features anywhere; `cargo deny check` expected clean (no new crates in the graph).

### Behavior after the change

```text
# journalctl -u uptrakit-controller        (default view — clean messages)
Aug 14 22:31:41 uptrakit uptrakit-controller-standalone[1007]: evicted SSH session from pool
Aug 14 22:31:41 uptrakit uptrakit-controller-standalone[1007]: host configuration changed — sending updated ReportHosts

# journalctl -p notice -u …                (INFO and up, DEBUG excluded)
# journalctl -u … -o verbose               (structured: PRIORITY, TARGET, CODE_FILE/LINE, F_HOST_ID=…)
# journalctl -u … F_HOST_ID=019fffe6-…     (field filtering)
```

Interactive runs (`./uptrakit-controller -v` in a terminal): byte-identical to today.

### Live verification (2026-08-16, `root@uptrakit`, Debian 13, systemd 257)

Read-only checks against the single live deployment (`uptrakit.service` → `/usr/local/bin/uptrakit-controller-standalone
-vvv`, `StandardOutput=journal`, `StandardError=inherit`):

- Detection contract holds exactly: `JOURNAL_STREAM=10:352799757` in `/proc/1007/environ`; `stat -L /proc/1007/fd/1` ⇒
  `dev=10 ino=352799757` (fd/1 and fd/2 are the same journal stream socket). Decimal, colon-separated, equality on both
  halves.
- `/run/systemd/journal/socket` is `srw-rw-rw-` — the unprivileged `uptrakit` user can open it, so `Layer::new()`
  succeeds without privilege changes.
- Priority flatness confirmed: 2000 most recent unit entries = 1999× priority 6, 1× priority 5 — and the priority-5
  entries are sudo's own syslog lines, not daemon tracing. Daemon tracing is 100% priority 6 today despite `-vvv`
  (trace-level) output.
- Installer token scrape observed working against the live journal: `journalctl -o cat | grep -A1 "one-time registration
token"` returns the `eprintln!`-origin prompt plus token line.

### Compatibility notes (verified)

- PVEHS installer token scrape (`scripts/pvehs/install/uptrakit-install.sh:144`) parses lines printed via `eprintln!`
  (`crates/core/controller-runtime/src/boot/settings.rs:40`), which bypasses tracing entirely; stderr still journals as
  plain `MESSAGE` lines, and `journalctl -o cat` output for those lines is unchanged.
- Docker / redirected stdout: no `JOURNAL_STREAM` match ⇒ text format, as today.
- Verbosity flags (`-v`), `RUST_LOG` precedence, and per-binary directive schemes: unchanged.

## Error handling

All failure paths degrade to the text layer with an `eprintln!` warning; no `unwrap`/`panic!`; parse errors are typed
(`thiserror`) per the error-handling standard. No `Report` plumbing needed — errors never cross the crate boundary
(consumed inside `init()`).

## Testing

All in `uptrakit-tracing-init` (success + failure paths per AGENTS rule; no upstream-behavior tests — tracing-journald's
mapping/encoding is not ours to pin):

1. `LogFormat::from_str` — `auto`/`text`/`journald` accepted; junk yields the typed error.
2. `JournalStream::from_str` — valid `"123:456"`; failures: missing colon, non-numeric halves, empty value.
3. `journal_stream_matches` — pure matrix: env absent; env present + stat equal ⇒ true; dev or ino mismatch ⇒ false;
   malformed env ⇒ false; stat unavailable ⇒ false. No env-var mutation in tests (values injected as parameters).
4. Format resolution — pure fn matrix over `(LogFormat, detected)`.
5. Exclusion predicate — attach `env_filter.and(filter_fn(...))` to a **fmt** layer in the existing `capture()` harness
   (`lib.rs` tests) and assert target `uptrakit_audit` is denied while `uptrakit_audit_log`-prefixed targets and ordinary
   `uptrakit_*` targets pass. This pins our combinator wiring without needing a journald socket.

Not unit-tested (thin glue, environment-dependent): the `stdout_is_journal()` wrapper's live `fstat`, the
`is_terminal()` ANSI gate, and the actual journald socket handshake. Manual verification on the live deployment: `systemctl restart`, then the four `journalctl`
invocations above.

## Documentation deliverables (grep-derived sweep of `journald|journalctl|JOURNAL_STREAM|LOG_FORMAT`)

1. **`docs/development/logging.md`** — new "Journald mode" section: detection contract, `UPTRAKIT_LOG_FORMAT` values,
   priority-mapping table, field-prefix filtering examples, fallback semantics. (Canonical home; `RUST_LOG` already
   documented here.)
2. **`docs/development/tracing.md`** — Subscriber Architecture section: layer selection inside `TracingBuilder::init()`;
   controller audit-layer + exact-target exclusion note.
3. **New ADR** — created via `adrs new "Journald-native log sink selection"` (number CLI-allocated, never hand-picked; no
   placeholder tokens — `adrs doctor` runs with `warnings_as_errors`).
4. **`crates/shared/tracing-init/src/lib.rs` module docs** — extend the overview (format resolution alongside the existing
   `RUST_LOG`-precedence section).

Verified no-change (part of the sweep, listed so reviewers need not re-derive):

- `AGENTS.md:203` "Logging goes to journald or stdout" — still accurate; mechanics belong in logging.md (AGENTS
  anti-inventory rule), no new invariant line.
- End-user `journalctl` usages (`docs/end-user/operator-runbook-reload.md:143`, `docs/end-user/autodiscovery.md:116`,
  `docs/security/key-rotation.md:115`) — commands remain valid under either format.
- `ARCHITECTURE.md:386` audit commit-hook mention — unchanged subsystem.
- No wire-type change ⇒ no asyncapi regen; no REST change ⇒ no `regen-api.sh`; no new state-changing sites ⇒ no
  `audit-catalog.toml` entries.

## Out of scope

- OpenTelemetry integration (tracing.md future section untouched).
- CLI (`uptrakit-cli`) logging changes.
- Windows-specific work (non-unix compiles to text mode).
- Verbosity directive scheme, `RUST_LOG` semantics, TOML/CLI configuration knobs.
- Reworking the audit journald backend itself.

## Alternatives considered

- **Compact fmt + `<N>` sd-daemon prefix** — keeps `key=value` inline in the default `journalctl` view but forgoes
  structured field indexing and requires a bespoke `FormatEvent` we maintain. Rejected: less idiomatic, less capable.
- **Journald layer with fields re-embedded into `MESSAGE`** — upstream doesn't support it; most bespoke code. Rejected.
- **Config-knob-only (no autodetect)** — more ceremony per deployment; controller CLI flag budget forbids a new flag.
  Rejected in favor of systemd's own `JOURNAL_STREAM` contract with an env escape hatch.
- **Audit dedup by dropping the dedicated audit layer in journald mode** — fewer layers, but `RUST_LOG=uptrakit_audit=off`
  could then silence the journald audit backend. Rejected: delivery guarantee wins.
