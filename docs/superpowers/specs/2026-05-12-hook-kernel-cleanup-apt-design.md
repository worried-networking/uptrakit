# Opt-In Kernel Cleanup Hook Plugin (APT)

## Goal

Add a strictly opt-in mechanism that purges superseded Linux kernel packages
(`linux-image-*`, `linux-modules-*`, `linux-headers-*`,
`linux-modules-extra-*`) on Debian and Ubuntu Hosts after a kernel update,
keeping at minimum the currently-running kernel and the latest installed
kernel. The mechanism is delivered as a new Hook-family plugin
(`hook_kernel_cleanup_apt`) assigned to an existing kernel meta-package
Software Item (e.g. `linux-image-amd64`, `linux-image-generic`).

Fedora/RHEL/openSUSE Hosts are addressed in v1 by documenting the
distro-native knob (`installonly_limit` in `/etc/dnf/dnf.conf`). A
sibling `hook_kernel_cleanup_dnf` plugin is deferred to v2.

## Scope

### In scope

- New crate `crates/plugins/hooks/kernel-cleanup-apt/` implementing the
  `LifecycleHook` role (`PostUpdateHook` slot only). Mirrors
  `crates/plugins/hooks/systemd/` layout.
- Two additive changes to the lifecycle hook framework:
  - `UpdateLifecycleContext.batch_id: Option<Uuid>` — propagated from the
    existing wire-level `batch_id` on `ExecuteBatchUpdatePayload` / single-item
    dispatch UUID; `None` only when no `batch_id` is in scope.
  - `LifecycleHook::detect_host_compatibility() -> Result<HostCompatibility>`
    with a default implementation returning `Compatible`. Called by the
    agent-side hook dispatcher immediately before `execute_pre_hook` and
    `execute_post_hook`; on `Incompatible(reason)` the hook is skipped
    non-fatally with output `[pre-hook] skipped: <reason>` /
    `[post-hook] skipped: <reason>`.
- Agent-side per-batch dedup so the hook fires at most once per
  `(plugin_type_id, batch_id)` tuple, regardless of how many Software Items
  in the batch carry the assignment.
- One new sudoers entry on Hosts running the plugin:
  `apt-get purge -y linux-image-* linux-modules-* linux-headers-*` (suffix
  pattern; actual invocation is an explicit per-`KVER` list).
- Verbose post-hook decision trace emitted as structured
  `tracing::info!` events (primary v1 audit trail) plus best-effort
  `[post-hook] ...` lines to `output_tx` (surfaces in
  `update_history.output` on the single-item path; discarded on the
  batch path until a separate framework spec rewires
  `client.rs:521`'s `_output_rx`). No new `AdminEvent` variant.
- Operator documentation: per-plugin doc page plus a cross-cutting kernel
  housekeeping runbook covering Debian/Ubuntu (this plugin) and
  Fedora/RHEL/openSUSE (`installonly_limit` snippet).
- ADR `docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md`
  capturing why this feature fits into the per-(host, software_item) hook
  seam through meta-package assignment instead of introducing host-scoped
  hooks or virtual Software Items.

### Out of scope (explicitly deferred)

- DNF/RPM-family cleanup plugin. Fedora is covered by `installonly_limit`,
  enforced inside `dnf install` natively. A sibling plugin `hook_kernel_cleanup_dnf`
  may land in v2 when concrete demand surfaces.
- Removing `linux-image-*-dbg`, `linux-cloud-tools-*`, `linux-tools-*`,
  `linux-buildinfo-*` companion packages. Out of v1 to keep the blast
  radius narrow.
- A reboot-pending probe (`/var/run/reboot-required` / `needs-restarting`).
  Safety derives from the never-purge-running and never-purge-latest
  invariants below; deferral logic is not required.
- A host-scoped hook assignment surface (nullable
  `host_software_item_plugins.software_item_id`, new table, or new role
  scope). This was considered and explicitly rejected — the meta-package
  assignment seam already covers the use case.
- A new `AdminEvent` variant. Reuses existing per-update output stream.
- Auto-attaching the hook to discovered kernel meta-packages. Operator
  attaches explicitly per Host or via existing bulk-assign UX.
- Periodic / scheduled sweep (`Enhancement`-family plugin). Cleanup is
  strictly synchronous-after-update.
- A `dry_run` global "preview the whole fleet" mode. The per-config
  `dry_run` flag is sufficient for staged rollout.
- Rewiring `client.rs:521`'s discarded `_output_rx` (batch-path
  output capture) so hook output and `execute_batch_update` output
  reach `update_history.output`. The batch-path framework gap is
  documented as a known limitation, mitigated via structured
  `tracing::info!` events from the plugin itself, and called out in
  the operator runbook with a journalctl audit recipe. A follow-up
  framework spec must own the fix; once it lands, the plugin's
  tracing approach becomes redundant.

### Explicitly not addressed

- No new term enters `CONTEXT.md`. The plugin operates entirely on
  existing Domain Language: Host, Software Item, Update, Release, Hook
  Plugin, `LifecycleHook` role.
- No multi-tenancy work. Single-tenant remains the only tested mode; the
  plugin honours existing tenant scope.
- No wire-protocol changes. `batch_id` already exists on
  `ExecuteBatchUpdatePayload`; `UpdateLifecycleContext` is an in-process
  type and the additive `Option<Uuid>` field rides existing
  `#[non_exhaustive]` discipline.

## Recommended approach

A new Hook-family plugin, identical in shape to `hook_systemd`, that
operators assign to the kernel meta-package Software Item already
tracked by APT discovery on a given Host. The hook is opt-in by
construction: no Software Item carries it until an Operator attaches
it via `host_software_item_plugins`.

The hook reads a small JSON config, performs a preflight host
compatibility check (`which apt-get`), and runs an idempotent
post-update cleanup driven by three pure decision functions and one
elevated command. The cleanup is non-fatal — any failure logs a
warning into the update output and returns `Ok(())` per the
`PostUpdateHook` non-fatal contract.

Why this shape:

- **Mirrors `hook_systemd` exactly.** No novel concept introduced;
  ramp for future readers is identical to the existing hook plugins.
- **Reuses the per-(host, software_item) assignment seam.** Debian's
  `linux-image-amd64` and Ubuntu's `linux-image-generic` (and flavors)
  already bump on every ABI tick, so the assigned meta-package is in
  every kernel-bumping batch. No new schema, no virtual items, no host
  scoping.
- **Two small framework additions** (`batch_id` field, preflight
  trait method) pay for themselves immediately by closing the
  per-software-item firing duplication and the can't-skip-on-wrong-host
  gap.
- **Explicit enumeration, never `autoremove`.** `apt-get autoremove`
  can sweep dkms-required headers; the plugin instead lists, computes,
  and passes an explicit `apt-get purge -y <KVER list>` invocation.
- **Output-only audit.** No new wire types, no new event handlers; the
  full decision trace lives in the update's stored output.

Alternatives that were considered and rejected during grilling:

- **Flag inside `AptConfig` / `DnfConfig`.** Conflates Software-family
  (install) with Hook-family (cleanup) responsibilities, expands
  sudoers globally for every assignment, hides the lifecycle step in
  the package manager.
- **Single unified hook plugin that branches on apt/dnf at runtime.**
  Forces the union of family-specific sudoers on every Host; hooks
  have no `detect_host_compatibility` precedent today; mirrors no
  existing precedent (notification plugins are sibling crates per
  channel).
- **Instance-scoped sweeper Enhancement plugin.** Requires a new wire
  variant, agent command handler, and per-host state table; far
  larger blast radius than the feature warrants and violates the
  controller-only invariant established by ADR 0006.
- **New `HousekeepingHook` lifecycle phase.** Speculative generality;
  one concrete use case does not justify a new role + wire payload
  array + DB role enum variant.
- **Path 1 / Path 2 hook-assignment variants** (host-scoped hooks or
  synthetic "kernel" virtual Software Item). Investigation confirmed
  the existing meta-package assignment seam already fires on every
  kernel ABI bump on both Debian and Ubuntu, so neither path is
  needed.

## Architecture

### Plugin crate

```text
crates/plugins/hooks/kernel-cleanup-apt/
├── Cargo.toml
└── src/
    ├── lib.rs              # re-exports
    ├── config.rs           # KernelCleanupAptConfig + Validate impl
    ├── error.rs            # KernelCleanupAptError (rootcause + thiserror)
    ├── plugin.rs           # KernelCleanupAptHookPlugin + declare_plugin!
    │                       # + LifecycleHook impl + required_sudo_commands
    └── decisions.rs        # pure parsing/decision functions (testable)
```

### Components

**`KernelCleanupAptConfig`** (config.rs)

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelCleanupAptConfig {
    /// Total kernels to keep (running + latest are always kept; min 2).
    #[serde(default = "default_keep_n")]
    pub keep_n: u8,

    /// If true, emit `[post-hook] would purge: ...` and skip the
    /// actual `apt-get purge` invocation.
    #[serde(default)]
    pub dry_run: bool,

    /// Minimum free space on `/boot` (KiB) required to proceed with
    /// purge. Below this threshold the hook aborts: a tight `/boot`
    /// risks `update-initramfs` failure mid-purge that can leave
    /// dpkg in an inconsistent state. Default: 51200 (50 MiB).
    #[serde(default = "default_min_boot_free_kib")]
    pub min_boot_free_kib: u32,
}

const fn default_keep_n() -> u8 { 2 }
const fn default_min_boot_free_kib() -> u32 { 51_200 }
```

Validation follows the `PluginConfig` trait pattern established by
`SystemdHookConfig` (`crates/plugins/hooks/systemd/src/config.rs:21`)
and `ShellHookConfig`:

```rust
impl PluginConfig for KernelCleanupAptConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if self.keep_n < 2 {
            return Err(PluginConfigValidationError::invalid_field(
                "keep_n",
                "keep_n must be >= 2 to protect running and latest kernels",
            ));
        }
        Ok(())
    }
}
```

The plugin-config validation pipeline (controller-side config
mutation routes via `uptrakit-plugin-infrastructure-registry`) invokes
`PluginConfig::validate()` on every create/update of the plugin
config, surfacing a 400 with field-level detail. By the time
`KernelCleanupAptHookPlugin::new()` runs on the agent, the invariant
already holds.

**`decisions.rs`** — three pure functions, each unit-tested:

| Function | Input | Output | Purpose |
| :--- | :--- | :--- | :--- |
| `parse_dpkg_kernel_list(output: &str)` | step-2 `dpkg-query` stdout (covers `linux-image-*`, `linux-image-unsigned-*`, `linux-modules-*`, `linux-headers-*`, `linux-modules-extra-*` patterns) | `Vec<KernelEntry>` — one entry per KVER, where `KernelEntry { kver: String, installed_variants: KernelVariantSet }` and `KernelVariantSet` is a bit-flag set over `{Signed, Unsigned, Modules, ModulesExtra, Headers}`. Both signed (`linux-image-<KVER>`) and unsigned (`linux-image-unsigned-<KVER>`) presence canonicalise to the same KVER entry so the abort invariant matches `uname -r` if **either** is installed. Step 6 reads `installed_variants` to assemble purge argv, never asking apt to purge a non-installed package name. | Extract concrete `linux-image-<KVER>` and `linux-image-unsigned-<KVER>` packages with `installed` status; drop meta-packages (`linux-image-amd64`, `linux-image-generic`, `linux-image-virtual`, `linux-image-aws`, `linux-image-generic-hwe-*`, `linux-image-unsigned-generic`, etc.); fold companion-package rows (`linux-modules-<KVER>`, `linux-modules-extra-<KVER>`, `linux-headers-<KVER>`) into the matching KVER's `installed_variants` rather than emitting separate `KernelEntry` items. |
| `parse_apt_mark_holds(output: &str)` | `apt-mark showhold` stdout | `HashSet<KVER>` | Surface held kernel KVERs to exempt them from cleanup. |
| `compute_keep_and_purge_sets(entries, running_kver, latest_kver, held, keep_n)` | parsed data | `KeepDecision { keep: Vec<KVER>, purge: Vec<KVER>, abort_reason: Option<String> }` | Decide kill list. Always include running and latest; respect holds (held kernels kept on top of `keep_n`); walk descending until `keep` size reaches `keep_n`. Abort with reason if `running_kver` has no matching entry in `entries`. |

`KernelEntry` and `KeepDecision` are crate-private types; no public
enum surface, so no `#[non_exhaustive]` or `Other(String)` discipline
applies.

**`KernelCleanupAptHookPlugin`** (plugin.rs)

Mirrors `SystemdHookPlugin`:

- `new(config: KernelCleanupAptConfig, runtime: Arc<dyn HostRuntime>) -> std::result::Result<Self, String>`
- `declare_plugin!(...)` block: `family: PluginFamily::Hook`,
  `host_requirements: HostRequirements::new(&[OsFamily::Linux], &REQUIRED_FEATURES, false)`
  where `REQUIRED_FEATURES = [host_features::POSIX_SHELL]` (no
  `host_features::APT_FAMILY` exists yet; preflight covers it).
- `config_test: [ConfigTestKind::PostUpdateHook]`
- `roles: [LifecycleHook]`
- `sudo: KernelCleanupAptHookPlugin::required_sudo_commands` returning
  one `SudoCommandEntry::new("apt-get", "Purge old kernel packages")
  .with_args_suffix("purge --yes linux-image-* linux-modules-* linux-headers-*")
  .with_setenv()` (string literal coerces via `impl Into<Cow<'static, str>>`
  per the precedent at `roles.rs`/`SystemdHookPlugin` sudoers).

`LifecycleHook` impl:

- `detect_host_compatibility()` — runs `which apt-get` via the
  executor; returns `Compatible` on exit 0, else `Incompatible("apt-get not found")`.
- `execute_pre_hook()` — returns `Ok(PreUpdateHookResult::proceed())` unconditionally
  (this plugin is `PostUpdateHook`-only; the pre-hook is a no-op).
- `execute_post_hook()` — runs the cleanup pipeline below; logs every
  error as a `tracing::warn!`; returns `Ok(())` on every code path.

### Cleanup pipeline (inside `execute_post_hook`)

1. **Read running kernel.** `uname -r` via executor. Failure → emit
   `[post-hook] kernel cleanup aborted: failed to read uname -r: <err>`,
   return `Ok(())`.
2. **List installed kernel-family packages.**
   `dpkg-query --show --showformat='${Package}\t${Status}\n' 'linux-image-*' 'linux-image-unsigned-*' 'linux-modules-*' 'linux-headers-*' 'linux-modules-extra-*'`.
   Parse via `parse_dpkg_kernel_list`. Failure → abort similarly,
   return `Ok(())`.
3. **List holds.** `apt-mark showhold`. Failure logged at warn but does
   not abort (held set defaults to empty).
4. **Decide.** Call `compute_keep_and_purge_sets`. If
   `KeepDecision.abort_reason` is `Some`, emit
   `[post-hook] kernel cleanup aborted: <reason>` and return `Ok(())`.
5. **Probe `/boot` free space — gating, not advisory.**
   If `config.min_boot_free_kib == 0`, skip the probe entirely (the
   operator has explicitly opted out of `/boot` gating; document this
   as the escape hatch for bind-mounted `/boot`, btrfs subvolumes,
   LVM-on-`/boot`, or operators on `t2.micro`-class VMs who accept
   the risk). Otherwise run `df --output=avail /boot` (KiB). On `df`
   failure or ambiguous output (non-numeric, multi-line, target is a
   symlink to a different filesystem): abort with
   `[post-hook] kernel cleanup aborted: /boot probe ambiguous; set min_boot_free_kib=0 to override`
   — refusing-to-proceed is the safe default because the whole point
   of step 5 is gating. If free space is below
   `config.min_boot_free_kib` (default `51200`, i.e. 50 MiB), abort
   with `[post-hook] kernel cleanup aborted: /boot has only <X> KiB free (need >= <T> KiB); update-initramfs may fail mid-purge` and emit
   the corresponding event at `tracing::warn!` (NOT `info!`) so
   repeat aborts surface in monitoring — a permanent no-op on a
   small-`/boot` VM is the failure mode this rule must make visible.
   Rationale: `apt-get purge linux-image-<KVER>` triggers
   `update-initramfs -d -k <KVER>` and dependent regeneration; with
   `/boot` tight, the postrm can leave dpkg mid-transaction and
   render the host unbootable on next reboot. Always emit
   `[post-hook] /boot free before: <KiB>` when the probe succeeds.
6. **Build explicit purge argv.** For each `KVER` in `purge`, read
   the corresponding `KernelEntry.installed_variants` and append
   only the package names that are present (`linux-image-<KVER>` if
   `Signed`, `linux-image-unsigned-<KVER>` if `Unsigned`,
   `linux-modules-<KVER>` if `Modules`, `linux-modules-extra-<KVER>`
   if `ModulesExtra`, `linux-headers-<KVER>` if `Headers`). Never
   ask apt to purge a non-installed name (would yield a confusing
   non-zero exit and surface as a purge failure for an otherwise
   benign cleanup).
7. **Dry-run branch.** If `config.dry_run`, emit
   `[post-hook] would purge: <args>` and return `Ok(())`.
8. **Purge.** `apt-get purge --yes <args>` with
   `DEBIAN_FRONTEND=noninteractive`, privileged. Output streamed via
   `UpdateOutputSender`. Non-zero exit → `tracing::warn!` + emit
   `[post-hook] apt-get purge exit <code>`; return `Ok(())`.
9. **Probe `/boot` after.** Emit
   `[post-hook] /boot free after: <KiB>`.
10. **Done.** Emit `[post-hook] kernel cleanup completed`.

In addition to the `output_tx` lines above, every step emits a
`tracing::info!` event with the structured fields enumerated
authoritatively in the "Known framework limitation" subsection
below. The tracing trail is the **primary v1 audit surface** because
the batch-path `output_tx` is currently discarded (see "Known
framework limitation"); on the single-item path the `output_tx` lines
also land in `update_history.output`.

### Framework changes

**`UpdateLifecycleContext.batch_id: Option<Uuid>`**
(`crates/plugins/infrastructure/core/src/traits.rs`)

```rust
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct UpdateLifecycleContext {
    pub package_identifier: String,
    pub to_version: String,
    pub from_version: Option<String>,
    pub release_info: Option<ReleaseInfo>,
    pub update_succeeded: Option<bool>,
    pub batch_id: Option<Uuid>,           // NEW
}
```

Both `for_pre_hook` and `for_post_hook` constructors gain a trailing
`batch_id: Option<Uuid>` parameter. Call sites:

- Batch path: `crates/shared/agent-core/src/client.rs:577` (pre) and
  `:598` (post) — populate with `Some(payload.batch_id)`.
- Single-item path: `crates/shared/agent-core/src/update.rs:115` (pre)
  and `:284` (post) — populate with `None` (no batch context).
- Test fixtures in `update.rs` (lines ~1331, 1340, 1348, 1355): update
  call sites to pass `None` for the new parameter.

**`LifecycleHook::detect_host_compatibility`**
(`crates/plugins/infrastructure/core/src/roles.rs`)

```rust
#[async_trait]
pub trait LifecycleHook: PluginMeta {
    /// Preflight check. Default: Compatible. Override to skip on
    /// incompatible hosts.
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Ok(HostCompatibility::Compatible)
    }

    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<PreUpdateHookResult>;

    async fn execute_post_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<()>;
}
```

The new method is additive on the existing trait at `roles.rs:210-225`
(which is already `: PluginMeta`-bounded); the spec does not redefine
the trait. Existing `SystemdHookPlugin` and `ShellHookPlugin` retain
the default implementation. New plugin overrides it.

**Known framework limitation: batch-path output is unreachable** ⚠️
(`crates/shared/agent-core/src/client.rs:521`,
`crates/shared/agent-core/src/update.rs:740-745,800-805`)

The batch dispatch path constructs an output channel whose **receiver
is discarded** at the call site:

```rust
// client.rs:521
let (output_tx, _output_rx) =
    tokio::sync::mpsc::channel::<uptrakit_command::UpdateOutputLine>(100);
```

The `output_tx` is then passed into `execute_batch_update`, which
forwards it through plugin internals — but every line written to it
disappears into the dropped `_output_rx`. Downstream from the plugin,
`run_batch_pre_hook_plugins` and `run_batch_post_hook_plugins`
additionally create **their own** local `mpsc::channel` plus a drain
task (`update.rs:740-745` and `:800-805`) that swallows every hook
output line — a second nested discard.

The single-item path, in contrast, pairs `output_tx` with an
`output_rx` that the spawned task reads (`client.rs:351,398`,
`client.rs:205,241`) and streams back to the Controller as
`UpdateOutput` wire messages persisted into `update_history.output`.

Batch-path observability today is **return-value-based only**:
`BatchUpdateResult.output` is a `String` aggregated by the plugin
itself and surfaced into each item's `BatchUpdateItemResult.output`
(see `client.rs:630`). Hooks have no slot in that return value because
hooks fire once per batch (not once per item), and no per-batch
"trailer" field exists on the result payload.

**v1 decision: degraded observability via structured tracing.**

Fixing the discarded `_output_rx` is out of scope for this spec; it
requires either (i) adding a `batch_output: String` field to
`BatchUpdateResultPayload` (wire-level change with `wire_safe_enum!`
discipline downstream) or (ii) refactoring `client.rs:521` to read
the receiver and route lines into the existing per-item result
trailers. Both warrant their own spec.

For v1, `hook_kernel_cleanup_apt` emits its decision trace via two
channels:

1. **`output_tx` (best-effort).** All `[post-hook] ...` lines are
   still emitted to the `UpdateOutputSender` the hook is handed.
   On the single-item path (operator-driven `apt-get install
   linux-image-X` against a tracked concrete kernel Software Item),
   these surface in `update_history.output`. On the batch path
   they're discarded today and will surface automatically once the
   future framework fix rewires `_output_rx`. No code change in this
   plugin is needed for that future improvement to take effect.

2. **`tracing::info!` with structured fields (primary v1 audit
   trail).** Every decision-step of the cleanup pipeline emits a
   structured tracing event keyed on `batch_id`, with fields:
   `plugin_type = "hook_kernel_cleanup_apt"`,
   `host_id` (from runtime context if exposed; otherwise omitted),
   `batch_id` (from `ctx.batch_id`),
   `running_kernel`,
   `installed_kernels` (comma-joined),
   `latest_installed`,
   `held_kernels`,
   `keep_set`,
   `purge_set`,
   `dry_run`,
   `abort_reason` (when set),
   `apt_purge_exit_code` (when invoked),
   `boot_free_before_kib`,
   `boot_free_after_kib`.

   The journal entries can be filtered via
   `journalctl -u uptrakit-agent --output=json |
   jq 'select(.plugin_type == "hook_kernel_cleanup_apt")'` for full
   audit reconstruction. Spec-level note for the v2 framework work:
   if/when `BatchUpdateResultPayload.batch_output` lands, the same
   data should be mirrored into `output_tx` and the tracing approach
   becomes a redundant belt-and-braces.

**Agent-side hook dispatcher dedup**
(`crates/shared/agent-core/src/update.rs:770-815`)

`run_batch_post_hook_plugins` (and the matching pre-hook variant)
gain a function-local `HashSet<(PluginTypeId, Uuid)>` initialised at
the top of the loop over `plugins`. When `ctx.batch_id == Some(uuid)`,
the dispatcher checks the pair and, on second encounter, emits
`[post-hook] skipped: already ran for batch <uuid>` to the existing
(currently-drained) output channel **and** logs a
`tracing::info!(plugin_type, batch_id, "post-hook dedup skip")`
event. When `ctx.batch_id == None`, dedup is a no-op. The set lives
only for the duration of that function call, which is exactly one
batch dispatch by construction. No cross-batch caching, no shared
state, no lock primitive needed.

Single-item paths (`run_pre_hook_plugins` / `run_post_hook_plugins`
at `update.rs:115` and `:284`) iterate one hook at a time per Software
Item; they cannot duplicate a hook within their own scope and do not
need dedup.

Both pre- and post-hook dispatchers also invoke
`detect_host_compatibility()` first; on `Incompatible(reason)`, they
emit `[pre-hook] skipped: <reason>` / `[post-hook] skipped: <reason>`
to `output_tx` (best-effort) and log a structured `tracing::info!`
event with `plugin_type` and `reason` fields. Both pre- and post-hooks
are skipped atomically for that plugin.

### Data flow

```text
Operator attaches hook_kernel_cleanup_apt
  to (host=H1, software_item=linux-image-amd64)
                           |
                           v
apt-get upgrade pulls linux-image-amd64 -> 6.1.78-1
  ExecuteBatchUpdatePayload.batch_id = Uuid::now_v7()
                           |
                           v
Agent executes batch update via apt plugin
                           |
                           v
Post-update phase iterates batch items
  For each item with assigned PostUpdateHook plugins:
    For each (plugin, ctx):
      ctx.batch_id = Some(payload.batch_id)
      if (plugin_type_id, batch_id) seen: skip with dedup output
      else if !detect_host_compatibility(): skip with reason output
      else execute_post_hook(ctx, output_tx)
                           |
                           v
KernelCleanupAptHookPlugin::execute_post_hook
  → uname -r, dpkg-query --show --showformat=..., apt-mark showhold
  → compute_keep_and_purge_sets
  → apt-get purge -y <explicit list>
  → tracing::info! events (primary audit) + output_tx best-effort
  → batch path: tracing only (output_tx → discarded _output_rx)
  → single-item path: output_tx → update_history.output
                           |
                           v
Discovery next cycle picks up that old linux-image-* are gone
  → host_software_items rows reconciled by existing path
```

### Sudoers

The new sudoers entry is generated by the existing
`required_sudo_commands` mechanism. The hook plugin appears in
`/etc/sudoers.d/uptrakit-<host>` only when assigned to at least one
Software Item on that Host (existing per-assignment sudoers generation
applies; no change to that pipeline). The exact line is:

```text
uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get purge --yes linux-image-* linux-modules-* linux-headers-*
```

The sudoers glob `linux-modules-*` matches both `linux-modules-<KVER>`
and `linux-modules-extra-<KVER>` (sudoers `*` is greedy and matches
any suffix, including the `extra-` infix); the explicit runtime argv
nonetheless lists both families because `linux-modules-extra` is a
`Recommends:` of the image package on Debian/Ubuntu, not a `Depends:`,
so `apt-get purge linux-image-<KVER>` alone does not cascade to
remove it.

Implementation flags `with_args_suffix` semantics for verification
during plan-phase: if `*` matches only single-token args, the suffix
must be rewritten as either three separate `SudoCommandEntry` rows
(`purge --yes linux-image-*`, `purge --yes linux-modules-*`,
`purge --yes linux-headers-*`) or a single broader form. Final shape
locks in the plan, not the spec.

## Error handling

- **Plugin error type.** `KernelCleanupAptError` in `error.rs`,
  `thiserror::Error` + `rootcause::Report` per snapshot rules
  (`/docs/development/error-handling.md`). Variants:
  `CommandFailed { command: String, exit_code: i32 }`,
  `OutputParse { source: String, detail: String }`,
  `Configuration(String)`. `Result<T> = std::result::Result<T, Report<KernelCleanupAptError>>`.
- **Bidirectional conversion.** `impl_report_conversion!` between
  `KernelCleanupAptError` and `PluginError` for the cross-boundary path
  required by `LifecycleHook::execute_post_hook` return type.
- **Non-fatal contract.** Every code path inside `execute_post_hook`
  returns `Ok(())`. Every failure is logged via `tracing::warn!` with
  structured fields (`plugin_type`, `batch_id`, `package_identifier`,
  `error`) and emitted to `output_tx` with a `[post-hook] ...` prefix
  (best-effort — see "Known framework limitation" above). The
  framework's per-update record stays `succeeded=true` even on cleanup
  failure, matching the documented `PostUpdateHook` semantics.
- **Config validation.** `KernelCleanupAptConfig` implements
  `PluginConfig` (not the `uptrakit-web-api-types::Validate` trait,
  which is reserved for HTTP request shapes). The
  `PluginConfig::validate()` invocation in the plugin-config
  mutation pipeline rejects `keep_n < 2` with a
  `PluginConfigValidationError::invalid_field(...)` surfaced as `400`.
  Plugin code can rely on the invariant by the time `new()` runs.

## Testing

Per snapshot rule "Keep parsing and comparison in pure functions for
testability", the test surface concentrates in `decisions.rs`.

### Unit tests (`decisions.rs`)

Table-driven, covering:

- `parse_dpkg_kernel_list`: typical `dpkg-query` output with three
  concrete kernels; mixed installed/deinstall/config-files statuses;
  empty input; output with extraneous packages
  (`linux-image-amd64`, `linux-image-virtual`,
  `linux-image-generic-hwe-22.04`) that must be filtered out;
  malformed lines (graceful drop, not panic); **signed-only KVER**
  yields `KernelEntry { installed_variants: {Signed, Modules,
  Headers} }`; **unsigned-only KVER** yields
  `installed_variants: {Unsigned, Modules, Headers}`; **both signed
  and unsigned for the same KVER** canonicalise to a single entry
  with `{Signed, Unsigned, Modules, Headers}`; companion-only rows
  (`linux-modules-extra-<KVER>` with no matching image package) do
  NOT create a `KernelEntry` and are dropped.
- `parse_apt_mark_holds`: empty input, single hold, multiple holds,
  non-kernel holds (ignored).
- `compute_keep_and_purge_sets`:
  - running == latest, `keep_n=2`, three installed → keep latest +
    previous, purge oldest.
  - running != latest, `keep_n=2`, three installed → keep running +
    latest, purge middle.
  - running has no matching entry → `abort_reason = Some(_)`.
  - `keep_n=2`, two installed → empty purge set (no-op).
  - `keep_n=2`, one installed → empty purge set, no abort (running
    matches the sole entry).
  - `keep_n=3`, four installed → purge oldest only.
  - Held kernels: hold on the oldest kernel exempts it from purge
    even when `keep_n` would otherwise drop it; held set does not
    consume `keep_n` slots.
  - `keep_n=2`, running and latest are both held: held set covers,
    no further keeps needed.
  - Running KVER is installed as `Unsigned`-only (signed variant
    absent): abort invariant does NOT fire; KVER is kept.
  - Purge set's KVER has `installed_variants = {Unsigned, Modules}`:
    purge argv contains exactly `linux-image-unsigned-<KVER>` and
    `linux-modules-<KVER>`, no signed/headers/extra (verifies step-6
    cross-check on real per-variant data).

### Plugin-level tests (`plugin.rs`)

Mirror the existing `hook_systemd` test shape
(`crates/plugins/hooks/systemd/src/plugin.rs:149-200`): construct the
plugin via `Arc::new(uptrakit_command::LocalCommandExecutor)` and
assert descriptor metadata. Coverage:

- `plugin_type_id()` returns `"hook_kernel_cleanup_apt"`.
- Descriptor capabilities include `UpdateLifecycle` and `ConfigTest`.
- Descriptor has `LifecycleHook` role; no `Discoverer`,
  `VersionDetector`, `UpdateExecutor`, or other Software-family roles.
- Descriptor has one sudo entry; command is `apt-get`; args suffix
  starts with `purge --yes`.
- `detect_host_compatibility()` invoked on a machine without
  `apt-get` (CI's Mac / non-Debian Linux) returns
  `HostCompatibility::Incompatible(_)`.
- `KernelCleanupAptConfig::validate()` (via `PluginConfig`) rejects
  `keep_n = 0` and `keep_n = 1` with field name `keep_n`.

Direct execution of `execute_post_hook` against a real
`LocalCommandExecutor` is intentionally excluded: it would shell out
to `apt-get purge` on the test host. Coverage of the cleanup pipeline
itself lives entirely in `decisions.rs` (pure functions) plus a
manual end-to-end Debian VM run flagged in the implementation plan.
This matches the precedent set by `hook_systemd` and `hook_shell`,
which similarly omit live-execution tests of their hook bodies.

### Framework tests

- `UpdateLifecycleContext::for_post_hook` / `for_pre_hook` regression
  test asserting the new `batch_id` field is populated correctly
  from the new constructor parameter.
- Agent-side dedup test in `crates/shared/agent-core/src/update.rs`
  (the batch post-hook dispatcher): a fabricated batch of two items
  each assigned the same hook plugin fires the post-hook exactly once
  when `batch_id` is `Some`, twice when `batch_id` is `None`.
- Agent-side preflight test: hook reporting `Incompatible("…")`
  emits the expected skip output and never invokes the pre/post hook
  bodies.

### Snapshot conformance

- No `start_paused = true` (no tokio time APIs in this plugin or its
  tests).
- No SQLite transactions (no DB writes).
- No `parking_lot::Mutex` in the plugin (stateless). The agent-side
  dedup `HashSet` lives in a stack-local for the batch dispatch
  function; no shared async state.
- All public fallible APIs in `decisions.rs` get a `# Errors`
  rustdoc section.

## Quality gates

Standard backend Rust gates from
`docs/development/quality-gates.md`:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
python3 ci/check_plugin_semantic_boundary.py
markdownlint --config .markdownlint.json '**/*.md'
sentrux check .
```

No frontend changes in v1 (the existing plugin-assignment UI already
renders new hook plugin types from the catalog).

The system-integration mandatory Docker run is not required
(`docker build -f docker/Dockerfile.test ...`) — the feature does not
touch enrollment, wire, or service lifecycle. The reverse-proxy
mandatory run is not required.

## Documentation deliverables

Each item is non-optional unless flagged.

| Doc | Action | Owner |
| :--- | :--- | :--- |
| `docs/end-user/plugins/hook_kernel_cleanup_apt.md` (new) | Operator-facing plugin reference. Must include: (a) **supported meta-package matrix** — explicit table listing every meta-package the plugin's parser recognises (`linux-image-amd64`, `linux-image-arm64`, `linux-image-686`, `linux-image-generic`, `linux-image-virtual`, `linux-image-aws`, `linux-image-generic-hwe-XX.YY`, `linux-image-unsigned-generic`, etc.) and an explicit "not supported in v1" callout for Proxmox `pve-kernel-*`, vendor-OEM `linux-oem-*`, Raspberry Pi `linux-image-raspi*`, ZFS-on-root `linux-image-zfs`, and manually-compiled kernels installed via `dpkg -i`; (b) config fields (`keep_n`, `dry_run`, `min_boot_free_kib`) including the `min_boot_free_kib = 0` escape hatch for bind-mounted / atypical `/boot`; (c) sample output (success + dry-run + abort variants for each abort reason: running not installed, `/boot` tight, `/boot` probe ambiguous, no purges); (d) sudoers entry; (e) **HWE rollover playbook** — when Ubuntu rolls the host from `linux-image-generic` to `linux-image-generic-hwe-XX.YY`, the operator MUST reattach the hook to the new meta-package Software Item; (f) **`apt-mark hold` interaction** — held kernels are exempt and do not count toward `keep_n`; (g) dkms / custom-driver caveats; (h) **discovery race caveat** — verify the meta-package appears in host inventory before the first kernel update fires; (i) one-line link to `docs/end-user/operations/kernel-housekeeping.md` for cross-distro audit / observability content. | spec |
| `docs/end-user/operations/kernel-housekeeping.md` (new) | Cross-cutting runbook. Sections: (1) Debian/Ubuntu via `hook_kernel_cleanup_apt` (links to the plugin doc); (2) Fedora/RHEL/openSUSE via `installonly_limit=2` in `/etc/dnf/dnf.conf` with rationale + worked example; (3) **batch-path observability limitation** — explanation of the `client.rs:521` framework gap and that batch-path hook output is captured via structured `tracing::info!` / `tracing::warn!` events only; (4) **journalctl audit recipe** (`journalctl -u uptrakit-agent --output=json \| jq 'select(.plugin_type == "hook_kernel_cleanup_apt")'`) with sample queries (last 24 h decisions, all purges for a specific batch_id, all aborts and reason counts); (5) **regulated-environment guidance** — recommend journald forwarding to a durable sink (rsyslog, vector, fluent-bit, systemd-journal-upload) before enabling in SOC 2 / HIPAA hosts; alternatively, defer enablement until the batch-path output capture framework spec lands. State the v1 trade-off explicitly. | spec |
| `docs/development/update-hooks.md` (update) | Add `batch_id` field to `UpdateLifecycleContext` table; add `detect_host_compatibility()` row to `LifecycleHook` table; document agent-side dedup semantics and skip-on-Incompatible output format; document the existing batch-path `_output_rx` discard at `client.rs:521` as a known framework gap with a follow-up-spec marker; add `hook_kernel_cleanup_apt` to plugin-type table. | spec |
| `docs/development/plugin-guidelines.md` (update) | One-line note pointing hook authors at the preflight idiom (and its default-impl exemption). | spec |
| `docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md` (new) | Capture the assignment-shape decision and the rejected alternatives (host-scoped hooks, virtual Software Item, sweeper Enhancement). | spec |
| `crates/plugins/hooks/kernel-cleanup-apt/README.md` (new) | Plugin lifecycle per `plugin-guidelines.md` requirements: detection, version comparison, update execution, required privileges, failure modes, required configuration. | spec |
| `CONTEXT.md` (no change) | The plugin operates entirely within existing Domain Language. No new term. | n/a |

## Snapshot deviations

None. All Binding Rules from `.superpowers/standards-snapshot.md`
that intersect this feature are satisfied:

- `rootcause::Report` + `report!()/bail!()` macros used.
- No `unwrap()`/`expect()`/`panic!()` in production code.
- `Result<T>` alias defined per crate boundary.
- `impl_report_conversion!` for cross-boundary conversion.
- All public structs that survive the crate boundary are
  `#[non_exhaustive]` (only `UpdateLifecycleContext` is public and
  is already annotated upstream).
- `Validate` trait is reserved for `uptrakit-web-api-types` HTTP
  request types; plugin configs use the project-native
  `PluginConfig::validate() -> Result<(), PluginConfigValidationError>`
  trait (precedent: `SystemdHookConfig`, `ShellHookConfig`).
  `KernelCleanupAptConfig` follows that pattern.
- `parking_lot::Mutex` is not used (no shared async state in the
  plugin).
- No SQLite transactions (no DB writes).
- `#[expect(lint_name, reason = "...")]` for any lint suppression.
- `#[cfg_attr(all(test, not(feature = "sea-orm")), derive(strum::EnumIter))]`
  is not needed (no public enums).
- Pure parsing/decision functions live in their own module.

## Risks and mitigations

| Risk | Mitigation |
| :--- | :--- |
| Operator attaches the hook to a concrete `linux-image-<KVER>` Software Item instead of the meta-package; hook never fires on subsequent kernel bumps because that Software Item is superseded, not updated. | Operator doc page leads with meta-package attachment as the only supported pattern; troubleshooting section explicitly covers this mistake. |
| `dpkg-query` output format differs between Debian / Ubuntu releases. | Use `--showformat='${Package}\t${Status}\n'` (stable across `dpkg` versions); parser tolerates extra fields and unknown statuses (drops them, never panics). |
| `apt-mark showhold` fails on a misconfigured Host. | Hold set defaults to empty; cleanup proceeds. Operators who rely on holds receive a `tracing::warn!` and would notice purge of a held kernel via the output stream. Documented in operator doc page. |
| Running kernel's package was already manually purged (operator action) before the hook fires. | Abort condition with explicit `[post-hook] kernel cleanup aborted: running kernel <X> has no matching installed linux-image package`. No purge issued. |
| New kernel image is in `/boot` but initramfs is corrupted; purging old kernel would leave host unbootable on next reboot. | Out of scope for v1 — uptrakit does not run initramfs validity probes. Documented as a known gap; operators are advised to verify a fresh boot before trusting cleanup. |
| `/boot` is tight; `apt-get purge linux-image-<KVER>` triggers `update-initramfs -d -k` cascade that can fail mid-transaction and leave dpkg in an inconsistent / unbootable state. | Pipeline step 5 gates on `config.min_boot_free_kib` (default 51200 KiB / 50 MiB); aborts with explicit reason if `/boot` is below threshold. Tight `/boot` is the most common foot-gun; the abort surfaces it before any irreversible action. |
| Secure Boot hosts install `linux-image-unsigned-<KVER>` alongside / instead of `linux-image-<KVER>`; the abort-if-running-not-installed invariant could yield false aborts or, worse, miss the running kernel and queue it for purge. | `parse_dpkg_kernel_list` canonicalises both `linux-image-<KVER>` and `linux-image-unsigned-<KVER>` to the same KVER key (one `KernelEntry` per KVER); the abort fires only if neither variant is installed. Purge argv lists both `linux-image-<KVER>` and `linux-image-unsigned-<KVER>` when present. |
| Non-flavored / vendor / hand-compiled kernels (`pve-kernel-*`, `linux-oem-*`, `linux-image-raspi*`, manually-installed `.deb`) are not children of any tracked meta-package; the hook becomes a silent no-op forever on those hosts. | Operator doc page lists the supported meta-package matrix and an explicit "not supported in v1" section for Proxmox / OEM / Raspberry Pi / manual kernels. v2 may extend the recognised meta-package set. |
| Ubuntu HWE rollover from `linux-image-generic` to `linux-image-generic-hwe-XX.YY` swaps the meta-package; the hook attached to the old meta becomes inert. | Operator doc page contains the explicit HWE rollover playbook: re-attach to the new meta-package Software Item after the rollover; old meta-package can then be deleted from tracking. |
| Tracing audit lives only on the host's journal; default `SystemMaxUse` rotation can erase the purge-decision audit within days on small VMs. Compliance risk for regulated environments. | Operator runbook (regulated-environment guidance subsection) requires journald forwarding to a durable sink (rsyslog, vector, fluent-bit, journald-upload) before enabling the plugin in SOC 2 / HIPAA hosts, or explicit deferral until the batch-path output capture framework spec ships. The deferred framework spec is tracked in the Out-of-scope section. |
| Per-batch dedup `HashSet` lifetime regressions if the batch dispatch is refactored. | Integration test asserts single-fire-per-batch holds across the dispatch function boundary; sentrux architectural check catches accidental cross-batch state introduction. |
| Sudoers wildcard semantics on `apt-get purge --yes linux-image-* linux-modules-* linux-headers-*` may not match multi-positional argv in some sudoers builds. | Plan-phase verification step; if needed, split into three sudoers entries. Caught before merge. |
| Batch-path `update_history.output` does not capture hook output (existing framework gap at `client.rs:521` discarded `_output_rx`). On the primary `apt-get upgrade` trigger path, the Operator cannot inspect the cleanup decision in the Dashboard's per-update output panel. | Structured `tracing::info!` events from the plugin provide a full audit trail in the agent journal (`journalctl -u uptrakit-agent --output=json \| jq 'select(.plugin_type == "hook_kernel_cleanup_apt")'`). Documented in operator runbook. A follow-up framework spec must redesign batch-path output capture; this plugin's tracing approach becomes redundant once that lands. |
| Tracing-only audit on batch path is operationally weaker than DB-persisted output. | v1 trade-off accepted. The action is auditable (journal + subsequent state inspection); the framework fix is tracked as a deferred follow-up but does not block this feature. |

## Open questions (none blocking)

None. All design forks resolved during grilling.

## Implementation order (for the plan, not part of the spec)

The implementation plan should sequence:

1. Framework additions:
   - `UpdateLifecycleContext.batch_id: Option<Uuid>` field +
     constructor parameter; update all `for_pre_hook`/`for_post_hook`
     call sites (`client.rs:577,598`, `update.rs:115,284`, test
     fixtures around `update.rs:1331-1355`).
   - `LifecycleHook::detect_host_compatibility` default-impl method
     added in `roles.rs:210-225`.
   - Agent-side preflight (call `detect_host_compatibility` before
     hook execution in all four dispatch sites at
     `update.rs:115,284,555,678,743,803` — adjust line refs as
     refactoring shifts them) + per-batch dedup `HashSet` inside the
     two batch dispatchers (`run_batch_pre_hook_plugins`,
     `run_batch_post_hook_plugins`).
   - **Batch-path output capture is explicitly out of scope.** The
     existing `_output_rx` discard at `client.rs:521` and the local
     drain task at `update.rs:740-745,800-805` are left untouched;
     batch-path hook output observability is delivered via
     `tracing::info!` structured events in the plugin itself. A
     follow-up spec must redesign batch-path output capture so hooks
     and `execute_batch_update` output reach `update_history.output`;
     once that lands, this plugin's `tracing` events become redundant
     and can be downgraded to `debug!`.
   - Verify `hook_systemd` and `hook_shell` remain bit-for-bit
     unchanged behaviourally (their batch-path output was already
     dropped and remains dropped; their single-item-path output is
     unaffected by the new framework parameters).
2. New plugin crate: `decisions.rs` and unit tests first; `plugin.rs`
   with mocked-executor integration tests; registry registration.
3. Sudoers verification (`with_args_suffix` multi-positional check).
4. Documentation deliverables, including ADR 0010.
5. Manual end-to-end test on a Debian VM with three installed
   kernels.
