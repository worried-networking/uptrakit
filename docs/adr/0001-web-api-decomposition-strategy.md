# web-api Decomposition Strategy

`uptrakit-web-api` is large enough to cause real navigation and compile-time
friction. The naive fix — extract all business logic into one monolithic parent
crate (`uptrakit-domain-ops`) that `web-api` depends on — would rename the problem
rather than solve it: `domain-ops` becomes the new fat crate, and compile wins are
minimal because the heavy dependencies (`sea-orm`, crypto) already live in
`uptrakit-web-api-queries`. The `actions/` module ranges from trivial delegation to
substantive orchestration (21–589 lines per file) but has no single coherent concept
and is called from many route files, so it does not qualify as an extraction unit on
its own.

We adopt **targeted per-concept extraction** instead. A subsystem is extracted into
its own crate only when it passes all three of:

1. **Coherent concept** — the crate has a name that tells a reader exactly what lives
   there (`uptrakit-mcp`, `uptrakit-surface-proxy`, `uptrakit-notification-dispatch`).
   Falsifiability anchor: if you cannot name the crate without a conjunction ("X and
   Y ops"), the concept is not coherent enough.
2. **Clear seam** — limited inbound coupling; the full set of callers (routes,
   middleware, `AppState` fields) is small and all flow one way.
3. **Self-contained test surface** — its tests compile and pass without pulling in the
   rest of `web-api`.

Candidates approved by this test:

| Subsystem | Status | Notes |
| --------- | ------ | ----- |
| MCP server | Completed — merging | Spec: `docs/superpowers/specs/2026-05-01-extract-mcp-crate-design.md` |
| Notification dispatcher | Approved — next | No Axum dependency; requires `build_settings_bag` refactor first (see Consequences) |
| `surface_proxy/` | Approved — after notification | Own test suite; six route callers + two middleware callers + one `AppState` field, all one-way |

Explicitly rejected: extracting `actions/` as a unit. It covers host, service,
batch, software item, and settings mutations — no single coherent concept. The real
reason extraction fails is that `MutationContext` (the capabilities bag that every
action takes) would have to follow wherever `actions/` goes, adding an indirection
layer without concentrating any knowledge. The `actions/` module already functions as
a deep module: its public interface (`MutationContext` + per-action functions) is
narrow relative to the orchestration it hides. Extracting it would not deepen it
further; it would just relocate it.

`AppState` stays in `web-api`. Extracted crates will not take `Arc<AppState>`
directly; they will receive only the sub-fields they need via a dedicated sub-state
struct (e.g. `McpDeps`, `NotificationDeps`). This is enforced structurally — a
dedicated struct is a different type from `AppState` and Rust will reject accidental
substitution. The pattern follows the existing `NotificationState`, `BroadcastState`,
and `AuthState` sub-structs already inside `AppState`.

## Consequences

- **Notification dispatcher pre-condition:** `dispatcher.rs` currently imports
  `uptrakit_web_api_auth::settings_store` to build SMTP/Telegram settings in
  `build_settings_bag`. A second call site exists in
  `surface_proxy/controller_local/notifications.rs`. Before extraction, refactor
  `build_settings_bag` to accept pre-loaded settings values as parameters rather than
  loading them internally, decoupling the dispatcher from `web-api-auth` and
  `uptrakit-shared-db`. Until this refactor is done, extracting the dispatcher
  carries the full `sea-orm`/`sqlx` compile graph and achieves no compile-time
  benefit.
- **surface_proxy sequencing note:** `surface_proxy` calls `build_settings_bag`
  from the notification dispatcher. Extracting the dispatcher first means that when
  `surface_proxy` is subsequently extracted, it will depend on
  `uptrakit-notification-dispatch` in addition to `uptrakit-web-api-queries`. Plan
  for this rather than discovering it mid-extraction.
- **AppState sub-state pattern:** Each extraction must introduce a dedicated
  sub-state struct (e.g. `McpDeps`) containing only the `AppState` sub-fields the
  crate needs. The MCP extraction serves as the first concrete example of this
  pattern. Future extractions must follow it.
- **Build-time gate:** Before each extraction, identify one representative source
  file in the target subsystem and measure `cargo build -p uptrakit-web-api` after
  touching it. After extraction, measure `cargo build -p <new-crate>` for the same
  logical change. The dominant build cost is `sea-orm`/`sqlx` and `aws-lc-rs` — if
  the new crate still transitively depends on these, the incremental build saving
  will be small. Use the before/after delta to decide whether extraction achieved its
  compile-time goal before proceeding to the next candidate.
- **`actions/` testability:** Tests for `actions/` can be added within `web-api`
  today using `NotificationDispatcher::test_channel()` and `TenantDb::new_for_test`
  under the `db-sqlite` feature — they require a migrated in-process DB, not a
  running server. Extracting `actions/` is not a pre-condition for test coverage.
