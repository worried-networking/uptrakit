# 0043 — Journald-native log sink selection

Date: 2026-08-16

## Status

Accepted

## Context

Under systemd, every daemon wrote `tracing_subscriber::fmt` text to stdout. The journal stored duplicate
timestamps, a single flat priority (6) for all levels, no structured fields, and raw ANSI escape codes inside
`MESSAGE` (live-verified). `journalctl -p err` filtering and field queries were unusable, while interactive
terminal output was fine and had to stay unchanged.

## Decision

`TracingBuilder::init()` (the single tracing entry point in `uptrakit-tracing-init`, now genuinely re-exported
by `uptrakit-service-sdk` after replacing its hand-maintained copy with a dependency) resolves the log sink at
startup:

- `UPTRAKIT_LOG_FORMAT=auto|text|journald`, default `auto`; invalid values warn on stderr and fall back to
  `auto`.
- Under `auto`, the native `tracing_journald::Layer` is installed if and only if stdout IS the journal:
  `JOURNAL_STREAM` (systemd.exec(5) `dev:inode` contract) must match `fstat(stdout)`. This is deliberately
  narrower than "journald is reachable" — redirected stdout on a systemd host keeps text format.
- Upstream defaults are kept: lossless priority mapping (ERROR→3, WARN→4, INFO→5, DEBUG→6, TRACE→7) and the `F`
  field prefix (`F_HOST_ID=…`).
- Every failure (socket absent, non-unix, forced `journald` off-systemd) degrades to the text layer with a
  stderr warning; logging setup never aborts a daemon. The text layer enables ANSI only when stdout is a
  terminal.
- Controller only: because `EnvFilter` target matching is prefix-based, the broad `uptrakit=…` directive also
  matches `uptrakit_audit`; when the dedicated journald audit layer is installed, target `uptrakit_audit` is
  excluded (exact match) from the main journald layer. One predicate drives both: audit layer constructed means
  layer plus exclusion; construction failed means neither.

## Consequences

- `journalctl -p`, `-o verbose`, and `F_*` field filtering work; default view shows clean messages. Plain
  `grep key=value` over journal output no longer matches fields — operators use field filters (see
  `docs/end-user/logging.md`).
- Post-init journald send errors are silently dropped by the upstream layer and the datagram socket blocks on
  backpressure — accepted residuals, comparable to the previous stdout-stream failure modes.
- `uptrakit-tracing-init` joined the publishable crate chain (release-plz Public-API group) so the SDK can
  depend on it.
- Rollback lever: `UPTRAKIT_LOG_FORMAT=text` via a systemd drop-in (survives the PVEHS installer's unit
  rewrite).
