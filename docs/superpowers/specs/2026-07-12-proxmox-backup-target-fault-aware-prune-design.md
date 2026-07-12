# Proxmox Backup-Target Cache: Fault-Aware Pruning — Design

**Date:** 2026-07-12 **Status:** Draft **Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Partial discovery
failure silently deletes valid backup targets and disables pre-update protection"
(`crates/plugins/infrastructure/proxmox/src/discovery.rs:152` + `policy_store.rs:374-392`).

## Problem

`discover_backup_targets` (`discovery.rs:130-163`) walks cluster nodes and, per node, **silently drops** three
distinct non-enumeration cases into the same hole:

- node `status != "online"` → `continue` (line 138),
- node excluded by `node_filter` → `continue` (line 141),
- `client.list_backup_targets_for_node()` returns `Err` → `tracing::warn!` + `continue` (line 152-159).

It returns a **flat `Vec<CachedBackupTarget>`** with no signal about which nodes were actually enumerated. That vec
feeds `upsert_cached_backup_targets` (`policy_store.rs:358-448`), whose prune logic is:

- discovered keys **empty** → `delete_many()` filtered only by `(tenant_id, plugin_config_id)` — **wipes every cached
  row** (lines 374-377);
- otherwise → delete every row whose `target_key.is_not_in(discovered_keys)` (lines 379-385).

So **one** node offline / erroring / filtered during a run erases that node's node-local backup targets, and an
all-nodes-down run wipes the entire cache. `prepare_backup_protection` (`update_protection.rs`) then fails every update
for guests whose policy references a pruned target ("configured backup target was not found in cache") until an operator
re-runs discovery with all nodes healthy. A transient infra blip silently disables pre-update protection → HIGH
stability/safety.

Root cause: the prune set is derived from "keys seen this run" with **no knowledge of which nodes we actually
enumerated**. A node we never asked (offline, filtered, errored) is indistinguishable from a node whose targets were
genuinely removed.

## Approach

Make the prune **scoped by the set of successfully-enumerated nodes**. Never prune a node we did not enumerate this run.
Two pieces:

1. `discover_backup_targets` returns the enumerated-node set alongside the targets.
2. The prune decision moves into a **pure function** (`compute_backup_target_prune_ids`) that takes the existing cache
   rows, the discovered keys, and the enumerated-node set — testable with plain structs, no DB. `upsert_cached_backup_targets`
   loads existing rows once, calls the pure function, deletes exactly those ids, then upserts.

### 1. `discover_backup_targets` → `BackupTargetDiscovery`

New return struct (in `discovery.rs`):

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupTargetDiscovery {
    pub targets: Vec<CachedBackupTarget>,
    /// Nodes whose `list_backup_targets_for_node` returned Ok this run — the
    /// authoritative "we currently know this node's target set" signal.
    pub enumerated_nodes: std::collections::BTreeSet<String>,
    /// Nodes whose enumeration returned Err (for surfacing partial failure).
    /// Offline/filtered nodes are NOT failures and do not appear here.
    pub failed_nodes: Vec<String>,
}
```

Fold: `enumerated_nodes` receives a node **only** on the `Ok` branch (line 146). Offline-skip, filter-skip, **and**
`Err` all leave the node absent from `enumerated_nodes` — that single set is the whole safety signal. `failed_nodes`
records the `Err`-only subset, purely for operator visibility (offline/filtered are expected, not failures).

`BTreeSet` (std, no new dep) — the pure function only does membership (`.contains()`), but `BTreeSet` gives
deterministic ordering for free in test assertions and log output, and is consistent with the `BTreeMap` this same
function already uses for target dedup (`discovery.rs:135`). `HashSet` would also be correct; the determinism is the
tiebreaker, not a correctness need.

### 2. Pure prune-decision function

```rust
/// One existing cache row, reduced to what the prune decision needs.
pub(crate) struct ExistingCacheRow {
    pub id: Uuid,
    pub proxmox_node: String,
    pub target_key: String,
}

/// Compute which cached rows to delete, fault-aware.
///
/// **Zero-discovery guardrail (first):** if `discovered_keys` is empty we prune
/// **nothing**. Empty means no positive evidence of any current target from any
/// source — no basis to delete. A genuine single-target removal always leaves
/// `discovered_keys` non-empty (other targets remain); fully-empty is either total
/// enumeration loss (every node down/errored) or an upstream parse/schema-drift bug
/// returning `Ok(vec![])`. Both are treated as suspected-total-loss faults, never a
/// wipe — this closes the `Ok(empty)` door that re-opens the HIGH.
///
/// Otherwise a row is pruned only when its `target_key` is absent from
/// `discovered_keys` AND we actually enumerated the node that owns it this run:
///   - node-local row (`target_key` starts with `"{proxmox_node}:"`):
///     prune iff `proxmox_node ∈ enumerated_nodes`.
///   - shared row (not node-prefixed): prune iff `enumerated_nodes` is non-empty
///     (≥1 node succeeded — the target is cluster-visible, so a genuine removal
///     is observable from any healthy node).
pub(crate) fn compute_backup_target_prune_ids(
    existing: &[ExistingCacheRow],
    discovered_keys: &BTreeSet<String>,
    enumerated_nodes: &BTreeSet<String>,
) -> Vec<Uuid> {
    // Guardrail: no discovered keys ⇒ no basis to prune. Never wipe on empty.
    if discovered_keys.is_empty() {
        return Vec::new();
    }
    existing
        .iter()
        .filter(|row| !discovered_keys.contains(&row.target_key))
        .filter(|row| {
            let node_prefix = format!("{}:", row.proxmox_node);
            if row.target_key.starts_with(&node_prefix) {
                enumerated_nodes.contains(&row.proxmox_node) // node-local
            } else {
                !enumerated_nodes.is_empty() // shared
            }
        })
        .map(|row| row.id)
        .collect()
}
```

The node-local-vs-shared derivation (`target_key.starts_with("{node}:")`) is the codebase's established one — already
used at `surfaces.rs:1427`. `target_key` format is set by `ProxmoxClient::backup_target_key` (`client.rs:224-235`):
node-local = `"{node}:{storage_id}:{storage_type}"`, shared = `"{storage_id}:{storage_type}"`. Any prefix-collision
ambiguity (a shared `storage_id` that happens to equal a node name) is **inherited** from that existing derivation, not
introduced here, and is bounded to at most retaining a row slightly longer — never an over-prune. Not in scope to fix.

**Consequences (assert in tests):**

- **All nodes down** (`enumerated_nodes` empty, `discovered_keys` empty) → the zero-discovery guardrail returns `[]`.
  The exact wipe repro is neutralised: cache preserved, protection stays armed.
- **Enumerated-but-empty** (`enumerated_nodes` non-empty, `discovered_keys` empty — e.g. every enumerated node reported
  zero targets, or an upstream `Ok(vec![])` parse bug) → **still** returns `[]` via the same guardrail. This is the
  `Ok(empty)` door: without the guardrail the per-node logic would prune every node-local row for every enumerated node
  and every shared row (enumerated non-empty), re-opening the HIGH. Suspected-total-loss is a fault, not a wipe.
- **One node failed** (`enumerated_nodes = {B}`, `discovered_keys` non-empty) → node A's rows are all retained
  (A ∉ enumerated); B's genuinely-removed rows are pruned; B's current rows upserted.
- **Genuine removal** on a healthy node → pruned normally (`discovered_keys` non-empty, missing only the removed key).
- **Filtered node** → never enters `enumerated_nodes`, so its rows are always retained (naturally covered).

**Why enumerated-node scoping, not "skip prune if any node failed"** (contrarian): a blanket "any failure ⇒ prune
nothing" starves pruning cluster-wide whenever one flapping node errors every run — stale rows for genuinely-removed
targets on healthy nodes never get cleaned. Scoping by the enumerated set prunes exactly the healthy nodes and retains
exactly the un-enumerated ones, which is the tightest correct rule.

**Accepted residual (YAGNI, called out per audit):** a shared storage that still exists but whose every carrying node
failed in the same run would be pruned once ≥1 **other** node succeeded (shared rows prune on non-empty
`enumerated_nodes`). This matches the audit's "shared targets pruned only when at least one node succeeded" guidance and
is the correct tradeoff — a shared target is cluster-visible, so if any healthy node stops reporting it, it is genuinely
gone. Not worth per-shared-target node tracking.

### 3. `upsert_cached_backup_targets` rewrite

Signature gains `enumerated_nodes: &BTreeSet<String>`. Body:

1. Open the transaction with **`begin_immediate()`** (read-then-write on the live path — SQLite requires BEGIN
   IMMEDIATE; see Dependencies). No-op on Postgres.
2. **Load existing rows once**: `ProxmoxBackupTargetCache::find().filter(tenant_id).filter(plugin_config_id).all(&tx)`
   → map each Model to `ExistingCacheRow`. This single query replaces both the blanket-delete branch **and** the
   per-target `find().one()` inside the upsert loop (a pre-existing N+1 — batch rule 59). Build a
   `HashMap<String, Model>` (or `<String, Uuid>`) keyed by `target_key` for the upsert lookups.
3. `let delete_ids = compute_backup_target_prune_ids(&existing, &discovered_keys, enumerated_nodes);`
4. If `delete_ids` non-empty: `delete_many().filter(Column::Id.is_in(delete_ids)).exec(&tx)`. **Skip the delete
   entirely when empty** (an empty `is_in` is a degenerate query; also avoids a pointless round-trip).
5. Upsert loop unchanged in effect, but decides update-vs-insert from the in-memory map (no per-target SELECT):
   `target_key` present → `update` that row's ActiveModel; absent → `insert` with `Uuid::now_v7()`.
6. Commit. Return `upserted`.

This removes **both** old prune branches and the N+1 in one stroke — the row load required for the fault-aware decision
is the same load that feeds the upsert lookups.

**Observe suspected-total-loss:** when `discovered_keys` is empty **and** `enumerated_nodes` is non-empty **and**
`existing` is non-empty, the guardrail refused a mass-prune — emit a single `tracing::warn!` (`skip_all`, explicit
fields: `enumerated = enumerated_nodes.len()`, `retained = existing.len()`) noting all enumerated nodes reported zero
backup targets and the cache was preserved as a suspected enumeration/parse fault. No new surface field beyond
`backup_target_nodes_failed` — the warn plus the existing failed-node count are enough (YAGNI).

**Accepted permanence (called out honestly, contrarian pass 2):** the guardrail cannot distinguish a transient
total-enumeration loss from a _genuine_ decommission of the last backup target cluster-wide — both present as
`discovered_keys` empty + `enumerated_nodes` non-empty. So when an operator really does remove every backup target,
the orphaned cache row(s) survive **every** subsequent discovery run and are cleared only by `reset.rs` (full
tenant-data wipe); there is no targeted "forget this decommissioned target" op today. This is accepted: (a) a stale
row cannot arm false protection (verified above — `start_backup` rejects the dead storage loudly), so the only cost is
a permanently-stale surface row and a doomed-but-loud backup attempt if a policy still references it; (b) auto-clearing
the enumerated-but-empty case is exactly the `Ok(vec![])` wipe door we closed. A targeted forget-target affordance is
**out of scope** (YAGNI) — add it only if operators hit the stale-row annoyance in practice.

### 4. Surface partial failure

`DiscoveryPersistSummary` (`discovery.rs:36-39`, currently `#[derive(Copy)]`) gains one **`usize`** field —
`backup_target_nodes_failed` — keeping the `Copy` derive (a `Vec` would break it; the count is enough, and the warn
logs already name which nodes). `discover_and_persist` (`discovery.rs:363-382`) sets it from
`discovery.failed_nodes.len()` and passes `&discovery.enumerated_nodes` into `upsert_cached_backup_targets`.

The discover-action JSON (`surfaces.rs:943-946`) gains one additive field:

```rust
Ok(serde_json::json!({
    "discovered": persisted.guests_upserted,
    "backup_targets_discovered": persisted.backup_targets_upserted,
    "backup_target_nodes_failed": persisted.backup_target_nodes_failed,
}))
```

so a transient blip is visible in the action result, not buried in a warn log. Additive only — no consumer breaks.

### Guests are not at risk (audit sub-task, verified)

`persist_discovered_guests` (`discovery.rs:254`) does **insert/update only** — no `delete`/`is_not_in`/prune (confirmed
by reading the body). The warn-and-skip pattern in `discover_guests` therefore cannot wipe guest rows. Leave guests
untouched (YAGNI) — the fault-aware treatment is needed only where a destructive prune exists.

### Retain-is-safer: the cache is an index, not the gate (contrarian #1)

This fix trades a loud false-**negative** (wipe → "target not found in cache" → every update blocked) for retaining
possibly-stale rows on un-enumerated nodes. That is strictly safer because **a stale cached row cannot silently arm
protection against a dead target**: `prepare_backup_protection` (`update_protection.rs:544`) uses the cache **only as
an index** — on a cache hit it proceeds directly to `client.start_backup(node, vmid, type, storage_id)` against **live
PVE** with no separate re-validation. If the backing storage was genuinely decommissioned, `start_backup` fails loudly
at backup time (PVE rejects the unknown storage), so the update is still blocked loudly — same outcome class as the
cache-miss, just one step later. The decommission-leak worst case is therefore a doomed-but-loud backup attempt on a
stale row, never a silent "protected" claim. Wiping, by contrast, blocks **all** guests cluster-wide the instant one
run mis-enumerates. Retain wins; no live-reconciliation of the cache is added (YAGNI — `start_backup` is the
reconciliation).

## Dependencies

- **`begin_immediate()` helper** — introduced by
  `docs/superpowers/specs/2026-07-11-sqlite-tx-mode-conformance-design.md` as `uptrakit_shared_db::begin_immediate(conn)`.
  Not yet in code. If that spec lands first, adopt the helper. If this spec is implemented first, use
  `begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), ..Default::default() })`
  directly with the crate-standard `// BEGIN IMMEDIATE: read-then-write` comment, and the conformance spec's later
  helper migration sweeps it up. **Do not invent a second mechanism.**
- No new external crates. `BTreeSet` / `HashMap` are std.

## Tests

Core fault-aware logic — **pure-function unit tests on `compute_backup_target_prune_ids`** (plain structs, no DB, no
tokio-time API so no `start_paused`):

1. **All-nodes-down wipe repro (the HIGH):** 2 nodes' rows in `existing`, `discovered_keys` empty, `enumerated_nodes`
   empty → returns `[]` (nothing pruned).
2. **One-node-failed:** rows for node A + node B; `enumerated_nodes = {B}`; `discovered_keys` = B's current keys (one B
   key removed vs cache) → returns exactly the stale B row's id; all A ids absent.
3. **Genuine removal:** `enumerated_nodes = {A}`; an A node-local key previously cached now absent from
   `discovered_keys` → its id returned.
4. **Shared target:** a shared row (no node prefix) → not pruned when `enumerated_nodes` empty; pruned when
   `enumerated_nodes` non-empty and its key absent from `discovered_keys`.
5. **Filtered/never-enumerated node:** a node-local row whose node ∉ `enumerated_nodes` and whose key ∉
   `discovered_keys` → **not** returned (retained).
6. **Enumerated-but-empty guardrail (the `Ok(empty)` door):** non-empty `existing` (node-local **and** shared rows),
   `enumerated_nodes` non-empty (all nodes), `discovered_keys` **empty** → returns `[]`. Proves a suspected-total-loss
   run cannot empty the cache even when nodes enumerated successfully.

Wiring — **one real-SQLite in-memory integration test** (`Database::connect("sqlite::memory:")` + `SchemaManager` +
apply **only** the `CreateProxmoxBackupTargetCache` migration via `Migration::up` — the crate's existing pattern in
`controller_migration.rs` tests; **not** `MockDatabase`, which returns canned results and cannot verify persisted
rows). The migration's FKs to `tenants`/`plugin_configs` are **not enforced** here: a raw `sqlite::memory:` connection
leaves `PRAGMA foreign_keys` OFF (SeaORM/sqlx do not enable it), and SQLite permits a `CREATE TABLE` referencing a
not-yet-existing parent — so the cache migration applies standalone and cache rows seed directly without parent
`tenants`/`plugin_configs` rows. This is exactly how the existing `controller_migration.rs` scaling-migration tests
seed `tenant_id`-bearing rows (e.g. the `proxmox_scaling_defaults` tests), so no core-schema bootstrap is needed:

**Test 7 (wiring):** seed cache rows for node A + node B, run `upsert_cached_backup_targets` with
`enumerated_nodes = {B}` and B's current targets (one B target dropped), then **query the cache back**: assert all A
rows survive, B's stale row is gone, B's current rows present, and the returned `upserted` count matches. Proves the
load → compute → delete → upsert wiring end-to-end, including that BEGIN IMMEDIATE commits. Two assertions are
load-bearing beyond row survival: (a) the seeded node A row must carry a **distinct `proxmox_node` and `target_key`**
so a swapped `Model → ExistingCacheRow` field mapping (proxmox_node ↔ target_key) would mis-scope the prune and fail
the test — the pure-function tests cannot catch a mapping bug in the DB load; (b) the A-only-survives shape also
exercises a non-empty `delete_ids`, while the guardrail/one-node cases exercise the **empty-`delete_ids` skip path** —
between them both branches of the "skip delete when empty" step are covered and upserts still commit.

(The `MockDatabase` transaction-log style used elsewhere in `policy_store.rs` cannot assert actual row survival — the
whole point of this fix — so the wiring test uses the real-SQLite migration harness the crate already has.)

## Documentation deliverables

- **Doc comments** on `discover_backup_targets` (now returns `BackupTargetDiscovery`; why the enumerated-node set
  exists and why prune must be node-scoped) and `upsert_cached_backup_targets` (fault-aware prune contract: never prune
  a node we did not enumerate this run) and `compute_backup_target_prune_ids` (the node-local/shared rule) — included in
  the code changes above.
- **`docs/development/plugin-guidelines.md`** — checked: search for an existing discovery/prune-contract section; if one
  documents the cache-prune behavior, add the fault-aware-scoping rule there. If none exists (likely — this is
  proxmox-internal), state "no external doc surface" and skip. No proxmox-specific end-user doc documents cache
  pruning.
- **No API / wire / OpenAPI change.** The surface-action JSON gains one additive field (`backup_target_nodes_failed`) —
  internal plugin surface response, not an OpenAPI-modelled endpoint; note the additive field, no regen needed.
- **No ADR** — bugfix using existing patterns (BTreeSet, pure-function extraction, node-key derivation already in the
  codebase), no architectural decision.

## Out of scope / deferred

- Migrating already-compliant `begin_with_options` sites or the broader SQLite-tx-mode sweep (owned by the
  tx-mode-conformance spec).
- Guest-discovery fault handling (`persist_discovered_guests` has no destructive prune — nothing to fix).
- Bulk-write micro-optimisation of the upsert loop (per-row insert/update is fine; the N+1 **read** is removed, which
  is the batch-rule concern). No new dep, YAGNI.
- Per-shared-target carrying-node tracking to close the accepted residual (§2) — not worth the complexity.
