# PVEHS Multi-Source Discovery + Local Host Bootstrap — Design

**Date:** 2026-08-16
**Status:** Design (pending plan)

All `file:line` references are locator hints against `main` @ `64e67cbb0`; verify before editing.

## Problem

The PVEHS discovery plugin (`crates/plugins/discovery/proxmox-helper-scripts/`) only recognises CT
scripts hosted in `community-scripts/ProxmoxVE`, and the live Uptrakit deployment (LXC at
`ssh root@uptrakit`, installed by `scripts/pvehs/install/uptrakit-install.sh`) is invisible to it.
Three independent defects block end-to-end discovery and support of that host:

1. **Hardcoded single-source URL prefixes.** `PHS_CT_URL_PREFIX` / `PHS_CT_URL_PREFIX_ALT`
   (`discovery.rs:13-22`) match only `community-scripts/ProxmoxVE` CT URLs, so
   `parse_phs_scripts()` (`discovery.rs:732-770`) parses zero slugs from the live host's
   `/usr/bin/update`, which points at
   `raw.githubusercontent.com/worried-networking/uptrakit/main/scripts/pvehs/ct/uptrakit.sh`.
   Observed symptom: `parsed PHS scripts from update file count=0` every 6h cycle.
   `PHS_INSTALL_URL_PREFIX` (`discovery.rs:26`, consumed at `plugin.rs:621`) additionally assumes
   `ct/` and `install/` sit at repo root; Uptrakit nests them under `scripts/pvehs/`.
2. **Line-scoped gh-release call parsing.** `collect_gh_release_calls()` iterates lines
   (`discovery.rs:548-559`) and extracts both quoted args from the same line
   (`parse_gh_call_args`, `discovery.rs:565-579`). Both Uptrakit scripts spread
   `fetch_and_deploy_gh_release \` across continuation lines (`scripts/pvehs/ct/uptrakit.sh:54-58`,
   `scripts/pvehs/install/uptrakit-install.sh:30-36`), so even with defect 1 fixed the analysis
   finds no GitHub source and the item would surface without a release source or
   `version_file_basename`.
3. **The version-detection sudo helper is never provisioned on embedded-agent hosts.** The only
   writer of `/usr/local/bin/uptrakit-phs-version` is the SSH agent's bootstrap/sync path
   (`install_helper_script`, `agent-ssh-runtime/src/operations/sudoers.rs:67`; call sites
   `operations/bootstrap.rs:1177`, `operations/sync.rs:471`, `operations/bootstrap_proxmox.rs:599`).
   The installer hand-copies the sudoers grant (`uptrakit-install.sh:57-88`, helper line `:68`) but
   never writes the helper file, and the embedded agent has no provisioning path at all
   (`SudoContext::default()` assumption, `crates/shared/command/src/sudo.rs:70-76`). Live host
   state: grant present, helper absent — version detection would fail with "no such file" forever,
   regardless of discovery fixes.

Not a defect: `/root/.uptrakit-controller-standalone` as the version file. Upstream
`fetch_and_deploy_gh_release` writes `/root/.<app_key>`; `derive_version_file_basename(key, slug)`
(`discovery.rs:446`) already maps key ≠ slug (documented `paperless` case) and **must stay
untouched** — `uptrakit` slug + `uptrakit-controller-standalone` key resolves through the same path.

## Decisions (settled with owner, 2026-08-16)

| #   | Decision                                                                                                                                                                                                                                                                                                                        |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Compiled-in const source table.** `PhsSource { owner, repo, branch, scripts_root }` with four entries: `community-scripts/ProxmoxVE`, `community-scripts/ProxmoxVED`, `tteck/Proxmox` (all `scripts_root = ""`), `worried-networking/uptrakit` (`scripts_root = "scripts/pvehs"`). Adding a source is a release, not config.  |
| D2  | Each source accepts **both** URL forms (`raw.githubusercontent.com/...` and `github.com/.../raw/...`), normalised to the source's own raw form — same dual-form behaviour the single source has today.                                                                                                                          |
| D3  | **Install URL derived per source**: swap the `ct/<slug>.sh` tail for `install/<slug>-install.sh` under the source's `scripts_root`, killing both the hardcoded prefix and the repo-root sibling assumption.                                                                                                                     |
| D4  | **Identity stays the bare slug; no migration.** The source is a discovery-time attribute only — updates run the host-local `/usr/bin/update` and versions are read from local `/root/.<key>`, so persisted rows never reference the source. One host references one source in practice.                                         |
| D5  | **Fetch-failure semantics unchanged**: any CT-script fetch failure still aborts the whole discovery run (`plugin.rs:472-479`, "aborting discovery to avoid partial snapshot"), for every source.                                                                                                                                |
| D6  | **tteck/Proxmox is best-effort.** Archived repo, same layout; tteck-era scripts predate `check_for_gh_release` and used `/opt/<APP>_version.txt`, so GitHub-managed tteck items may be skipped by the existing absent-version-file debug-log path. No `/opt/` fallback.                                                         |
| D7  | **Continuation-aware call parsing**: join backslash-newline continuations in a preprocessing pass before line iteration, for gh-release and codeberg-release call collection alike. Rewriting our scripts to single-line calls was rejected — it leaves the parser fragile for any third-party script using continuations.      |
| D8  | **`bootstrap-host` subcommand defined in `uptrakit-agent-runtime`** as a shared clap `Subcommand` enum + runner. `uptrakit-agent` exposes it directly (`uptrakit-agent bootstrap-host`); the controller proxies it under a nested `agent` namespace (`uptrakit-controller-standalone agent bootstrap-host`), O2 nesting chosen. |
| D9  | **Sudoers/helper provisioning logic moves to `uptrakit-agent-core`** (the designated agent/agent-ssh sharing crate), staying on the `&dyn RemoteExecutor` seam; `agent-ssh-runtime` re-imports it. A local `RemoteExecutor` adapter over `LocalCommandExecutor` lands in `uptrakit-command`.                                    |
| D10 | **Bootstrap semantics**: requires root (typed error otherwise, no self-elevation); `--user` flag defaulting to `uptrakit`; entries compat-filtered via `compatible_sudo_commands_for_host` (same as the SSH path); atomic write + `visudo -cf` validation; idempotent re-run.                                                   |
| D11 | **Installer stops knowing internals**: `uptrakit-install.sh` replaces its hand-maintained sudoers heredoc (`:57-88`) with a call to `uptrakit-controller-standalone agent bootstrap-host --user uptrakit`, ordered after the `/usr/bin/update` override is written (compat filter requires it).                                 |
| D12 | **Single spec, two parts, no ADR.** Part B follows the established ADR-0005 thin-bin/runtime-crate pattern; moving code into the designated sharing crate is convention-following. Implementation may split into two plans.                                                                                                     |
| D13 | Spec carries a **live post-implement verification checklist** against `ssh root@uptrakit` as acceptance criteria.                                                                                                                                                                                                               |

Alternatives rejected during grilling: generalised URL pattern matching (arbitrary-fetch/SSRF
surface — the plugin fetches whatever it parses out of a root-owned file; a closed const allowlist
keeps the fetch surface enumerable); config-stored source list (plugin-config plumbing + UI surface
for a list that changes at release cadence anyway); installer writing the helper file itself
(installer would keep hand-copying internals — the drift that produced defect 3); build-time
generation of installer sudoers text (heavy; the CLI achieves registry-driven generation without a
build step); embedded-agent startup self-provisioning (agent runs unprivileged — cannot write
`/etc/sudoers.d` at runtime); clap top-level flatten (O1) and dynamic builder registration (O3) for
the proxy mechanism.

## Verified facts

Repository layouts (checked via `gh`, 2026-08-16):

- `community-scripts/ProxmoxVED`: root `ct/<slug>.sh` + `install/<slug>-install.sh`, branch
  `main` — mirrors ProxmoxVE.
- `tteck/Proxmox`: `archived: true`, `private: false`, same root `ct/` + `install/` layout, raw
  fetches return HTTP 200.
- `worried-networking/uptrakit`: `private: false`; raw CT URL under `scripts/pvehs/ct/` returns
  HTTP 200 — the "unpublished source" concern from the investigation handoff is moot.

Live deployment (`ssh root@uptrakit`, 2026-08-16): LXC container; single
`uptrakit-controller-standalone` binary (controller with `embedded-agent` + `embedded-ssh-agent`
features — `crates/core/controller-standalone/Cargo.toml`, release matrix
`docs/development/releases.md:44`); `/usr/bin/update` present (single `curl | bash` line to our
repo); `/root/.uptrakit-controller-standalone` contains `uptrakit-controller-standalone-v0.0.6`;
`/etc/sudoers.d/uptrakit-uptrakit` grants `/usr/local/bin/uptrakit-phs-version`, file absent from
disk; discovery-allowlist tables empty (all plugin types permitted).

Workspace seams (all existing, no new dependencies):

- `uptrakit-agent-runtime` is already shared by the agent binary (`crates/core/agent/Cargo.toml:26`)
  and the controller's embedded agent (`controller-runtime/src/agent/mod.rs:27`, feature
  `embedded-agent`).
- The controller CLI already has a subcommand seam: `ControllerCommand::DbMigrate`
  (`controller-runtime/src/cli.rs:6-18`, dispatched in `lib.rs:176` before server boot). A variant
  added there lands in both `uptrakit-controller` and `uptrakit-controller-standalone` — the two
  bins share `uptrakit_controller_runtime::run()` and differ only in enabled features.
- `compatible_sudo_commands_for_host(Arc<dyn CommandExecutor>)`
  (`plugins/infrastructure/registry/src/registry.rs:162`) is already executor-generic and works
  verbatim with `make_local_executor()` (`agent-runtime/src/lib.rs:81`). The registry is already in
  the agent's dependency graph via `uptrakit-agent-core` (`shared/agent-core/Cargo.toml`).
- `RemoteExecutor` is a one-method trait (`shared/command/src/remote_executor.rs:28-33`);
  `agent-ssh-runtime` already depends on `uptrakit-agent-core`
  (`agent-ssh-runtime/Cargo.toml:42`) — the module move creates no new edges and no cycle.

## Design

### Part A — multi-source discovery

#### 1. `PhsSource` const table (`discovery.rs`)

Replace the three URL-prefix consts (`discovery.rs:13-26`) with:

```rust
/// A known Proxmox-helper-scripts hosting repository.
#[derive(Debug)]
pub struct PhsSource {
    pub owner: &'static str,
    pub repo: &'static str,
    pub branch: &'static str,
    /// Path prefix inside the repo above `ct/` and `install/` ("" for repo root).
    pub scripts_root: &'static str,
}

pub const SOURCES: &[PhsSource] = &[
    PhsSource { owner: "community-scripts", repo: "ProxmoxVE", branch: "main", scripts_root: "" },
    PhsSource { owner: "community-scripts", repo: "ProxmoxVED", branch: "main", scripts_root: "" },
    PhsSource { owner: "tteck", repo: "Proxmox", branch: "main", scripts_root: "" },
    PhsSource {
        owner: "worried-networking",
        repo: "uptrakit",
        branch: "main",
        scripts_root: "scripts/pvehs",
    },
];
```

Methods on `PhsSource` (exact set to taste at plan time): `ct_prefix()` (raw form),
`ct_prefix_alt()` (`github.com/…/raw/…` form), `ct_url(slug)`, `install_url(slug)` — all composing
`scripts_root` between branch and `ct/`/`install/`. `install_url` renders
`…/{scripts_root}/install/{slug}-install.sh`, which for the uptrakit source yields the nested
`scripts/pvehs/install/uptrakit-install.sh` path (D3).

`PhsScript` (`discovery.rs:161-168`) gains `source: &'static PhsSource`.

#### 2. Parsing and normalisation (`parse_phs_scripts`)

`parse_phs_scripts()` (`discovery.rs:732-770`) iterates `SOURCES × [ct_prefix, ct_prefix_alt]`
per line instead of the fixed two-element `PREFIXES` array. On match: extract slug before `.sh`,
validate with the existing `is_valid_slug`, normalise `script_url` to the **matched source's** raw
form, and record the source on the emitted `PhsScript`. Dedup stays slug-keyed (first occurrence
wins — deterministic by line order); with D4 the same slug from two sources is one item either way.

Ordering note: the uptrakit source shares the `raw.githubusercontent.com/worried-networking/uptrakit/main/`
prefix shape with the others but a longer path; prefixes are source-complete strings (including
`scripts_root` and `ct/`), so no source's prefix is a prefix of another's — match order is
irrelevant.

#### 3. Install-URL derivation (`plugin.rs`)

`plugin.rs:621` becomes:

```rust
let install_url = script.source.install_url(&script.slug);
```

No other change to the fallback flow: CT fetch failure still aborts the run (D5), absent version
file on a GitHub-managed item still logs at debug and skips (`plugin.rs:487-525`) — the path tteck
items are expected to take (D6).

#### 4. Continuation-aware call collection (`discovery.rs`)

Before line iteration in `collect_gh_release_calls()` (`discovery.rs:546`) and its Codeberg twin
(`discovery.rs:613` onward), preprocess the content once: join lines ending in `\` (tolerating
trailing `\r`) with a single space. Applied to the analysis content only — `parse_phs_scripts()`
input (`/usr/bin/update`) is unaffected. This makes the multiline `fetch_and_deploy_gh_release`
calls in both Uptrakit scripts parse to
`("uptrakit-controller-standalone", "worried-networking", "uptrakit")`, and
`derive_version_file_basename` (`discovery.rs:446`, untouched) then returns
`Some("uptrakit-controller-standalone")` since the key is a valid slug differing from `uptrakit`.

#### 5. What Part A does **not** change

`derive_version_file_basename`, the sudo command declarations
(`plugin.rs:377-396`), `ProxmoxHelperScriptsConfig` (stays the empty `{}` struct), the
`declare_plugin!` block, wire types, REST surface, DB schema. SSRF posture: the fetch surface is a
closed four-repo allowlist on `raw.githubusercontent.com`; the plugin's HTTP client acquisition is
unchanged.

### Part B — `bootstrap-host` local provisioning

#### 6. Shared sudoers module (`uptrakit-agent-core`)

Move the executor-agnostic items of `agent-ssh-runtime/src/operations/sudoers.rs` into a new
`uptrakit-agent-core` module (e.g. `crates/shared/agent-core/src/sudoers.rs`): `detect_is_root`
(`:18`), `detect_sudo_available` (`:28`), `install_helper_script` (`:67`), `resolve_command_path`
(`:99`), `generate_sudoers_content`, `escape_sudoers_arg_token` (`:159`),
`render_sudoers_command_spec` (`:175`), `ensure_docker_group_membership` (`:224`),
`write_sudoers_file` (`:261`), plus the `ResolvedSudoCommand` (`:39`) and `SudoersContent` (`:51`)
types. All keep their `&dyn RemoteExecutor` signatures (D9). The module gets its own typed error
enum + `Result` alias per the error-handling standard; `agent-ssh-runtime` re-imports the functions
and converts errors via `impl_report_conversion!` where its `Error::SshCommand` coupling
(`sudoers.rs:84`, `:284`) currently sits. Items become `pub`; the inline test module moves with the
code.

#### 7. Local executor adapter (`uptrakit-command`)

New `LocalRemoteExecutor` implementing `RemoteExecutor` by running the command string through
`LocalCommandExecutor` (`shared/command/src/executor.rs:186-232`) with `CommandMode::Shell`
(`shared/command/src/types.rs:47-53`), preserving the pipe/`tee`/`visudo` idioms the sudoers code
relies on. Mapping `CommandOutput` (combined output + exit code) to `RemoteCommandResult`
(stdout/stderr split) follows the SSH adapter pattern (`agent-ssh-runtime/src/remote_exec.rs:29-38`)
with combined output carried as stdout and empty stderr; the sudoers code only inspects exit codes
and combined text, so the lossy split is acceptable and documented on the type.

#### 8. Shared subcommand enum (`uptrakit-agent-runtime`)

```rust
#[derive(clap::Subcommand)]
pub enum AgentRuntimeCommand {
    /// Provision sudoers entries and plugin helper scripts for this host.
    BootstrapHost(BootstrapHostArgs),
}

#[derive(clap::Args)]
pub struct BootstrapHostArgs {
    /// Unprivileged user the sudoers entries are written for.
    #[arg(long, default_value = "uptrakit")]
    pub user: String,
}
```

plus `pub async fn run_command(cmd: AgentRuntimeCommand) -> Result<()>` executing the bootstrap
flow: require root via `detect_is_root` against a `LocalRemoteExecutor` (typed error if not, D10);
collect entries with `compatible_sudo_commands_for_host(make_local_executor())`; install helper
scripts; resolve command paths; generate + atomically write
`/etc/sudoers.d/uptrakit-{user}` with `visudo -cf` validation; ensure docker group membership when
the docker plugin is compatible — i.e. the same sequence as the SSH sync action, against the local
adapter. `uptrakit-agent-runtime` gains a direct `uptrakit-plugin-infrastructure-registry`
dependency line (workspace-inherited; already linked transitively via `uptrakit-agent-core`) and a
`clap` dependency.

#### 9. CLI wiring (both binaries)

- **`uptrakit-agent`**: `cli.rs` (`crates/core/agent/src/cli.rs:4-11`) gains
  `#[command(subcommand)] command: Option<AgentRuntimeCommand>`; `main.rs` branches to
  `run_command` before constructing the daemon lifecycle. `--url` is already optional
  (`shared/service-sdk/src/cli.rs:28`), so the non-connecting subcommand does not fight required
  args. Invocation: `uptrakit-agent bootstrap-host --user uptrakit`.
- **controller-runtime**: `ControllerCommand` (`cli.rs:6-18`) gains a feature-gated variant (O2
  nesting, D8):

  ```rust
  #[cfg(feature = "embedded-agent")]
  Agent {
      #[command(subcommand)]
      command: uptrakit_agent_runtime::AgentRuntimeCommand,
  },
  ```

  dispatched alongside `DbMigrate` (`lib.rs:176`) before `boot::run_server`. The gate is additive
  (`#[cfg(feature)]` only — no negation), satisfying the feature-flag rule. Invocation:
  `uptrakit-controller-standalone agent bootstrap-host --user uptrakit` (also available on plain
  `uptrakit-controller` builds with `embedded-agent`).

#### 10. Installer change (`scripts/pvehs/install/uptrakit-install.sh`)

Delete the sudoers heredoc block (`:57-88`). After the binary is installed (`:37`) **and** the
`/usr/bin/update` override is written (`:160-166` today — the block moves ahead of the bootstrap
call), run:

```sh
/usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --user uptrakit
```

Ordering matters: the compat filter probes for `/usr/bin/update`, so PHS sudo entries (including
the helper) are only emitted once the override exists. The generated sudoers file name
(`/etc/sudoers.d/uptrakit-uptrakit`) matches what the heredoc wrote, so upgrades converge on the
same file. User creation, directories, and the systemd unit stay installer-owned — `bootstrap-host`
owns exactly the sudoers file + helper scripts.

#### 11. Cross-spec coordination

`docs/superpowers/specs/2026-08-16-pve-bootstrap-refactor-design.md` (pending plan) also rewrites
parts of `operations/bootstrap.rs` and touches `operations/sudoers.rs`. No semantic overlap — this
spec relocates executor-agnostic helpers; that spec changes PVE provisioning flows — but whichever
implementation lands second rebases over the other's file moves. Flag both plans with a mutual
reference.

## Security notes

- **Fetch surface stays closed.** Discovery fetches only URLs matching the four const sources; a
  URL outside the allowlist parses to nothing. No new user-controlled URL flows are introduced, so
  no new `SsrfSafeResolver` obligations arise; the existing plugin HTTP client path is unchanged.
- **Sudoers writes keep their guardrails**: `visudo -cf` validation before activation, 0440 mode,
  atomic move, per-command NOPASSWD entries only — never blanket `ALL`
  (`docs/security/sudoers-management.md`). The module move must not weaken any of this; moved code
  is behaviour-identical.
- **`bootstrap-host` runs as root by requirement** and writes exactly two kinds of files: helper
  scripts at plugin-declared `install_path`s (0755) and the sudoers drop-in (0440). It reads no
  secrets and logs no credentials.
- The hand-maintained heredoc (defect 3's origin) is deleted rather than patched — future plugin
  sudo requirements propagate to standalone installs without installer edits.
- `docs/hackme/09-discovery-result-poisoning.md` threat analysis must be revisited: the slug
  validation + canonical-URL rebuild defence (`:45-50`) now spans four sources; the analysis should
  state that a poisoned `/usr/bin/update` still cannot direct fetches outside the allowlist.

## Testing

Per-rule: success **and** failure paths; no Tokio time APIs are involved, so no `start_paused`
obligations; tests live in the existing inline modules and move with their code.

Part A (`discovery.rs` / `plugin.rs` test modules, `discovery.rs:1160` onward today):

- Per-source parse: for each of the four sources × both URL forms → expected slug, normalised
  `script_url` (source's raw form), and source identity. Existing asserts pinning the
  community-scripts normalisation are updated, not deleted.
- Nested install-URL derivation: uptrakit source →
  `…/worried-networking/uptrakit/main/scripts/pvehs/install/uptrakit-install.sh`.
- Rejection: a syntactically similar URL for a non-allowlisted repo yields no script.
- Cross-source dedup: same slug via two sources → single item, first occurrence's source.
- Continuation-aware analysis: multiline `fetch_and_deploy_gh_release \` content (mirroring
  `ct/uptrakit.sh`) → correct `(key, owner, repo)` + `version_file_basename`; same for the
  Codeberg collector; failure path: continuation with malformed args still yields none.
- End-to-end fixture: an update-file line for the uptrakit source + CT/install script fixtures →
  discovery target with GitHub source `worried-networking/uptrakit` and
  `version_file_basename = Some("uptrakit-controller-standalone")`.

Part B:

- Moved sudoers tests keep passing in `uptrakit-agent-core` (scripted `RemoteExecutor` double —
  reuse the shared `uptrakit-command` test-support double if the PVE bootstrap spec has landed it
  by then; otherwise the module's existing private double moves along and consolidation joins that
  spec's deferred item).
- `LocalRemoteExecutor`: shell-mode execution maps combined output + zero/non-zero exit codes
  correctly (success + failing command).
- Bootstrap flow against a scripted executor: helper install + sudoers write sequence; `visudo`
  failure aborts without activating the file; non-root start fails with the typed error.
- CLI: `uptrakit-agent bootstrap-host --user foo` parses; controller `agent bootstrap-host` parses
  under `embedded-agent` (feature-gated test); bare `uptrakit-agent` still enters daemon mode.

## Live verification checklist (acceptance, `ssh root@uptrakit`)

Run after release/upgrade to a build containing both parts:

1. `uptrakit-controller-standalone agent bootstrap-host --user uptrakit` as root succeeds;
   `/usr/local/bin/uptrakit-phs-version` exists (0755) and `/etc/sudoers.d/uptrakit-uptrakit`
   passes `visudo -cf`.
2. Trigger discovery (or wait for the 6h cycle); journal shows
   `parsed PHS scripts from update file count=1` and a discovery result for slug `uptrakit`.
3. The software item carries a GitHub release source for `worried-networking/uptrakit` and a
   GenericShell "PHS Shell" assignment; `sudo /usr/local/bin/uptrakit-phs-version
uptrakit-controller-standalone` returns `uptrakit-controller-standalone-v0.0.6`.
4. Installed-vs-upstream version comparison is sane against the `uptrakit-controller-standalone-v*`
   tag scheme (release-source normalisation is existing behaviour — verify, do not change).
5. Second `bootstrap-host` run is a no-op-equivalent (idempotency).
6. `nuc1` remains skipped (no `/usr/bin/update`) — no regression on non-PHS hosts.

## Documentation deliverables

Grep-derived set (all existing files; no wire/REST/OpenAPI surface changes, so no
`regen-api`/`regen-asyncapi` runs):

- `docs/development/autodiscovery-internals.md` — PHS section (`:84-133`) describes a single
  `raw.githubusercontent.com` fetch; document the source table and per-source install derivation.
- `docs/end-user/autodiscovery.md` — PHS section (`:91-126`): list supported script sources and the
  tteck best-effort caveat (D6).
- `docs/security/sudoers-management.md` — bootstrap-installs claim (`:111-113`) covers only the SSH
  path; add the `bootstrap-host` local provisioning path and command reference.
- `docs/hackme/09-discovery-result-poisoning.md` — canonical-URL defence (`:45-50`) restated for
  the multi-source allowlist.
- `docs/end-user/deployment/proxmox-helper-scripts.md` — installer behaviour change (heredoc
  removed, `agent bootstrap-host` call, remediation command for existing installs).
- `docs/end-user/plugin-configs.md` — PHS auto-discovery notes (`:120-124`) if the source list is
  user-visible there (check at plan time).
- `scripts/pvehs/install/uptrakit-install.sh` — the D11 change itself (code deliverable, listed for
  completeness).

Explicitly no update: `AGENTS.md` (no new invariant, no quick-start command change — the plugin and
CLI additions are internal to already-indexed crates), `CONTEXT.md` (no new controlled-vocabulary
term; "source" stays plugin-internal), ADRs (D12).

## Out of scope / deferred

- `/opt/<APP>_version.txt` version-file fallback for tteck-era GitHub-managed items (D6).
- Config/UI-managed source list.
- Deprovisioning of helper scripts and sudoers entries on uninstall.
- Embedded-agent automatic self-provisioning at startup (rejected — runs unprivileged).
- Consolidating pre-existing scripted-executor test doubles (belongs to the PVE bootstrap refactor
  spec's deferred list).
- Branch pinning/versioning strategy for script sources beyond `main`.
- Any change to `derive_version_file_basename` or the PHS sudo command declarations.

## Snapshot conformance

Checked against `.superpowers/standards-snapshot.md` and the common-mistakes ledger: typed errors +
`Result` alias for the new agent-core module and runner (error-handling standard); no `unwrap`/
`panic!`/`unsafe`; additive-only feature gate on the controller variant; no new dependencies beyond
workspace-registered `clap`/registry lines (`workspace = true`); tests cover success + failure with
no time APIs; line references are locator hints; doc deliverables grep-derived, not hand-listed; no
raw SQL, no DB change, no wire change; sudoers guardrails preserved verbatim.
