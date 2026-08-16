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

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Compiled-in const source table.** `PhsSource { owner, repo, branch, scripts_root }` with four entries: `community-scripts/ProxmoxVE`, `community-scripts/ProxmoxVED`, `tteck/Proxmox` (all `scripts_root = ""`), `worried-networking/uptrakit` (`scripts_root = "scripts/pvehs"`). Adding a source is a release, not config.                                                                                                                                                                                                                                                                                                                                                                                                                    |
| D2  | Each source accepts **both** URL forms (`raw.githubusercontent.com/...` and `github.com/.../raw/...`), normalised to the source's own raw form — same dual-form behaviour the single source has today.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| D3  | **Install URL derived per source**: swap the `ct/<slug>.sh` tail for `install/<slug>-install.sh` under the source's `scripts_root`, killing both the hardcoded prefix and the repo-root sibling assumption.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| D4  | **Identity stays the bare slug; no migration.** The source never reaches persistence; per-host divergence is absorbed by `host_software_items`/`host_software_item_plugins`. Note the tenant-wide layer: `find_or_create_software_item` matches `(plugin_type, package_identifier)` across hosts, so the same slug discovered from two sources on two hosts collapses onto one `software_items` row — safe because a shared slug across these four repos means the same application lineage by construction; only cosmetic item-level fields (name, icon) follow first-discoverer.                                                                                                                                                                |
| D5  | **Fetch-failure semantics split by cause** _(amended in review)_: a **definitive HTTP 404** on a CT or install script skips that slug with a `warn!` and continues; any ambiguous failure (network error, 5xx, 429, timeout) still aborts the whole run (`plugin.rs:472-479`, "avoid partial snapshot" — that rationale protects against ambiguity, which a 404 is not). Requires `fetch_text` to surface status instead of collapsing to `Option` (`plugin.rs:133-140`). Without this split, tteck (archived, frozen) and ProxmoxVED (staging repo — scripts are deleted after promotion) produce permanent 404s that abort discovery forever, killing tracking for every other slug on the host.                                                |
| D6  | **tteck/Proxmox is best-effort** _(path corrected in review)_: tteck-era scripts predate `check_for_gh_release`/`GH_REPO=`, so `analyze_phs_script` classifies them as apt-fallback and `discover_software` takes the final else-branch that fetches the install script (`plugin.rs:617-624`) — **not** the GitHub absent-version-file debug path. A missing `install/<slug>-install.sh` sibling in the archived repo is a permanent 404, handled by D5's skip-on-404. No `/opt/<APP>_version.txt` fallback.                                                                                                                                                                                                                                      |
| D7  | **Continuation-aware call parsing, scoped to the release-call collectors only**: join backslash-newline continuations in a preprocessing pass fed **exclusively** to gh-release and codeberg-release call collection; every other extractor (`extract_app_name`, `extract_gh_repo_var`, `extract_npm_package`, `extract_apt_package`) keeps the original unjoined content — they are strictly line-prefix-oriented and the join would silently break their matches across the community-scripts corpus. Rewriting our scripts to single-line calls was rejected — it leaves the parser fragile for any third-party script using continuations.                                                                                                    |
| D8  | **`bootstrap-host` subcommand defined in `uptrakit-agent-runtime`** as a shared clap `Subcommand` enum + runner. `uptrakit-agent` exposes it directly (`uptrakit-agent bootstrap-host`); the controller proxies it under a nested `agent` namespace (`uptrakit-controller-standalone agent bootstrap-host`), O2 nesting chosen.                                                                                                                                                                                                                                                                                                                                                                                                                   |
| D9  | **Sudoers/helper provisioning logic moves to `uptrakit-agent-core`** (the designated agent/agent-ssh sharing crate), staying on the `&dyn RemoteExecutor` seam; `agent-ssh-runtime` re-imports it. A local split-pipe `RemoteExecutor` implementation lands in `uptrakit-command`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| D10 | **Bootstrap semantics**: requires root (typed error otherwise, no self-elevation); `--user` flag defaulting to `uptrakit`; entries compat-filtered via `compatible_sudo_commands_for_host` (same as the SSH path); **validate-before-activate write** — the current `write_sudoers_file` tees onto the live file and validates after (`sudoers.rs:277-309`), so the move hardens it to tmp-file → `chmod 0440` → `visudo -cf` tmp → `mv` (remove tmp on failure), for the SSH path and the local path alike (the deleted installer heredoc was the only atomic writer; this restores that guarantee in code); idempotent re-run (unconditional but validated rewrite converging on identical content).                                            |
| D11 | **Installer stops knowing internals**: `uptrakit-install.sh` replaces its hand-maintained sudoers heredoc (`:57-88`) with a call to `uptrakit-controller-standalone agent bootstrap-host --user uptrakit`, inserted after the `/usr/bin/update` override — which itself stays after `customize()`, since `customize()` rewrites `/usr/bin/update` and would revert an earlier override. The call is **non-fatal** (`\|\| msg_error` + manual remediation command) — it sits past the install's point of no return under `build.func`'s errexit trap. `ct/uptrakit.sh`'s `update_script()` also invokes it non-fatally on both its up-to-date and post-upgrade paths, so existing installs self-heal on the next `update` run _(added in review)_. |
| D12 | **Single spec, two parts, no ADR.** Part B follows the established ADR-0005 thin-bin/runtime-crate pattern; moving code into the designated sharing crate is convention-following. Implementation may split into two plans.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| D13 | Spec carries a **live post-implement verification checklist** against `ssh root@uptrakit` as acceptance criteria.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

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
#[derive(Debug, PartialEq, Eq)]
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
`scripts/pvehs/install/uptrakit-install.sh` path (D3). **Empty-`scripts_root` join footgun**: three
of the four sources have `scripts_root = ""`, and a naive `{branch}/{scripts_root}/ct/…` format
produces a doubled slash (`main//ct/…`) that 404s on `raw.githubusercontent.com` — the URL
composition must emit the separator only when `scripts_root` is non-empty, and the per-source parse
tests below cover all three empty-root sources against exactly this.

`PhsScript` (`discovery.rs:161-168`) gains `source: &'static PhsSource`. `PhsSource` derives
`PartialEq, Eq` because `PhsScript` already derives both and the new reference field must compare.

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

Fetch-failure handling changes per D5: `fetch_text` (`plugin.rs:133-140`) currently collapses
404/5xx/timeout into one `Option` — it (or a wrapper) must surface the HTTP status so the caller
can distinguish. A definitive 404 on a CT or install script logs `warn!` and **skips that slug**;
any other failure keeps the abort ("avoid partial snapshot"). tteck items take the apt-fallback
else-branch (`plugin.rs:617-624`), where a missing install sibling in the archived repo is a
permanent 404 → skip, not run-abort (D6). ProxmoxVED's churn (staging repo, scripts deleted after
promotion) is bounded by the same skip.

Observability: when `/usr/bin/update` exists but parsing yields zero scripts, log at `warn!`
(currently `debug!`, `discovery.rs:769` / `plugin.rs:465`) including the first unmatched URL-like
token — this is the silence that let defect 1 run unnoticed for months; future allowlist drift
must be visible.

#### 4. Continuation-aware call collection (`discovery.rs`)

Preprocess the script content **once in the caller** — `analyze_phs_script()` joins lines ending
in `\` (tolerating trailing `\r`) with a single space and hands the joined string **only** to
`collect_gh_release_calls()` (`discovery.rs:546`) and its Codeberg twin (`discovery.rs:613`
onward). Every other extractor (`extract_app_name`, `extract_gh_repo_var`, `extract_npm_package`,
`extract_apt_package`) keeps the **original unjoined content** — they are line-prefix-oriented
(`strip_prefix("GH_REPO=")`, `apt`/`apt-get` line starts), and feeding them joined text would
silently lose detection for any community script whose relevant line follows a continuation.
`parse_phs_scripts()` input (`/usr/bin/update`) is unaffected. This makes the multiline `fetch_and_deploy_gh_release`
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
currently sits — three `bail!(Error::SshCommand(…))` sites (`sudoers.rs:84`, `:286`, `:304`), all
of which must convert to the new module's typed error. Items become `pub`; the inline test module moves with the
code.

One deliberate behaviour change rides the move (everything else is behaviour-identical):
`write_sudoers_file` becomes **validate-before-activate** per D10 — write `{path}.tmp`, `chmod
0440`, `visudo -cf` the tmp file, `mv` into place, `rm -f` the tmp on any failure. Today it tees
directly onto the live drop-in and validates afterwards (`sudoers.rs:277-309`), leaving an invalid
file active on validation failure — which can disable `sudo` host-wide; the installer heredoc
being deleted (`uptrakit-install.sh:58-86`) is currently the only writer that does this correctly.
`visudo` is invoked via `resolve_command_path` with a `/usr/sbin` fallback rather than bare — the
local `sh -c` environment may not have sbin dirs on `PATH`.

#### 7. Local executor adapter (`uptrakit-command`)

New `LocalRemoteExecutor` implementing `RemoteExecutor`, living in `uptrakit-command` alongside
`LocalCommandExecutor`. It must **not** route through `LocalCommandExecutor`'s merged-pipe
`CommandOutput` path: the moved sudoers code reads `.stderr` at four sites
(`sudoers.rs:87`, `:244`, `:289`, `:307` — including the `visudo -cf` failure message), so a
combined-output/empty-stderr mapping would blank exactly those diagnostics. Instead it spawns the command string via `sh -c` (tokio process) with **separately
piped stdout and stderr**, mapping losslessly to `RemoteCommandResult` — the same split the SSH
adapter provides (`agent-ssh-runtime/src/remote_exec.rs:29-38`). Shell mode preserves the
pipe/`tee`/`visudo` idioms the sudoers code relies on.

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

Acknowledged nuances:

- **Cross-crate clap coupling (accepted tradeoff, D8).** Three crates now share one `Subcommand`
  enum (`agent-runtime` defines; `agent` and `controller-runtime` embed), and `agent-runtime`
  takes on a `clap` dependency it did not have. This is a new shape in the workspace — the cost is
  a CLI-framework dependency in a runtime crate; the benefit is that neither binary re-declares or
  drifts from the bootstrap CLI surface. Owner-decided; plans should not re-litigate.
- **Probe executor is constructed with `is_root: true`.** Do not lean on
  `make_local_executor()`'s `SudoContext::default()` (`is_root: false`) plus the observation that
  today's `detect_host_compatibility()` probes never request privilege — that is a future-plugin
  invariant nobody enforces. `bootstrap-host` has already proven root via `detect_is_root` before
  probing, so build the probe executor with `SudoContext { is_root: true, .. }` and the question of
  emitted `sudo` prefixes never arises.
- **`privileged` flag on the moved sudoers functions.** Several moved functions (e.g.
  `write_sudoers_file`, `install_helper_script`) take a `privileged: bool` that controls whether a
  `sudo` prefix is emitted on their `tee`/`chmod`/`visudo` invocations. The local bootstrap flow
  passes the same value the SSH path uses when `detect_is_root` returns true — the process is
  guaranteed root (D10), so no `sudo` prefix is ever emitted locally.
- **`ensure_docker_group_membership` is a root-equivalent grant with delayed effect.** Adding the
  `uptrakit` user to the `docker` group is docker-socket access, i.e. root-equivalent — same as the
  SSH sync path today, not a new exposure, but worth naming since `bootstrap-host` now performs it
  on the controller's own host. The membership only takes effect for the running service after a
  restart (the installer calls `bootstrap-host` after `systemctl enable --now uptrakit`); until the
  next service restart the docker plugin simply stays incompatible, which is benign.

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

  dispatched alongside `DbMigrate` (`lib.rs:176`) before `boot::run_server`. The **dispatch arm is
  gated too**: the match/`if let` handling `ControllerCommand::Agent` must sit under its own
  `#[cfg(feature = "embedded-agent")]` block, or the build without the feature fails on a missing
  variant (same pattern as the `embedded-agent`-gated `mod` at `lib.rs:1`). Both gates are additive
  (`#[cfg(feature)]` only — no negation), satisfying the feature-flag rule. The dispatch arm
  initialises tracing exactly as the `DbMigrate` arm does (`lib.rs:176-183`) before running —
  bootstrap output (helper installs, `visudo` verdicts) must not run under an unconfigured
  dispatcher. Invocation:
  `uptrakit-controller-standalone agent bootstrap-host --user uptrakit` (also available on plain
  `uptrakit-controller` builds with `embedded-agent`).

#### 10. Installer change (`scripts/pvehs/install/uptrakit-install.sh`)

Delete the sudoers heredoc block (`:57-88`). Nothing else relocates: the `/usr/bin/update`
override (`:160-166`) **stays exactly where it is — after `customize()` (`:158`)**, because
upstream `customize()` writes its own `/usr/bin/update` pointing at `community-scripts/ProxmoxVE`
and the override exists to win that race; moving the override earlier would let `customize()`
silently revert it and reproduce defect 1. The new bootstrap call is inserted **after** the
override block (between `:166` and `cleanup_lxc`), **non-fatally** (D11):

```sh
/usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --user uptrakit \
  || msg_error "Host bootstrap failed — run '/usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --user uptrakit' as root to retry"
```

Non-fatal because this point is past the install's point of no return (service enabled and
running, registration token already consumed) and `build.func`'s errexit trap would otherwise turn
a bootstrap hiccup into a half-torn-down container; the remediation command makes the failure
recoverable by hand.

Ordering matters twice: `customize()` → override → bootstrap. The compat filter probes for
`/usr/bin/update`, so PHS sudo entries (including the helper) are only emitted once the _final_
override content exists. The sudoers grant now lands later in the install than the old heredoc
did (after service start instead of before); that is fine — the service does not need sudo to
start, only plugin-driven discovery/version commands use it, and those run periodically. The generated sudoers file name
(`/etc/sudoers.d/uptrakit-uptrakit`) matches what the heredoc wrote, so upgrades converge on the
same file. User creation, directories, and the systemd unit stay installer-owned — `bootstrap-host`
owns exactly the sudoers file + helper scripts.

**Existing-install self-heal via `ct/uptrakit.sh`** _(added in review)_: `update_script()` gains
the same non-fatal `bootstrap-host` invocation on **both** exit paths — the up-to-date early
return and the post-upgrade tail. Without this, hosts installed before this change (including the
live host, whose helper is absent today) never gain the helper until a manual command; with it,
the next `update` run repairs them. Idempotency (D10) makes the unconditional call safe.

#### 11. Cross-spec coordination

`docs/superpowers/specs/2026-08-16-pve-bootstrap-refactor-design.md` (pending plan) also rewrites
parts of `operations/bootstrap.rs` and touches `operations/sudoers.rs`. No semantic overlap — this
spec relocates executor-agnostic helpers; that spec changes PVE provisioning flows — but whichever
implementation lands second rebases over the other's file moves. Two specifics to coordinate: the
`write_sudoers_file` validate-before-activate hardening (D10) changes a function that spec's flows
call, and both plans must reference each other so neither reverts the other's shape. Flag both
plans with a mutual reference.

## Security notes

- **Fetch surface stays closed.** Discovery fetches only URLs matching the four const sources; a
  URL outside the allowlist parses to nothing. No new user-controlled URL flows are introduced, so
  no new `SsrfSafeResolver` obligations arise; the existing plugin HTTP client path is unchanged.
- **Sudoers writes keep their guardrails**: `visudo -cf` validation before activation, 0440 mode,
  atomic move, per-command NOPASSWD entries only — never blanket `ALL`
  (`docs/security/sudoers-management.md`). The module move is behaviour-identical **except** the
  deliberate D10 hardening: validate-before-activate replaces the current write-then-validate,
  strengthening (never weakening) the guardrail.
- **Two-writer ownership limitation (documented, not guarded).** On SSH-managed PVE hosts the sync
  path merges infra entries (`pct`/`qm` via `merge_infra_sudo_commands`,
  `operations/bootstrap.rs:1268-1298`) into the same drop-in; a `bootstrap-host` run there
  regenerates from the plugin registry only and would drop those infra lines until the next SSH
  sync rewrites them. Ownership rule: SSH sync owns the file on SSH-managed hosts; `bootstrap-host`
  is for standalone/embedded-agent hosts. Stated in `docs/security/sudoers-management.md` rather
  than enforced — the failure mode is a temporarily narrower grant set, never a wider one.
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
- Cross-source dedup: same slug via two sources in one update file → single `PhsScript`, first
  occurrence's source wins (parse-level determinism; in practice one update file references one
  source, so this pins `SOURCES`-iteration order rather than a live scenario).
- Fetch-failure split (D5): 404 on the install script → slug skipped with `warn!`, remaining slugs
  still processed; 5xx/network error → whole run aborts as today.
- Continuation-aware analysis: multiline `fetch_and_deploy_gh_release \` content (mirroring
  `ct/uptrakit.sh`) → correct `(key, owner, repo)` + `version_file_basename`; same for the
  Codeberg collector; failure path: continuation with malformed args still yields none.
- Extractor isolation (D7): a script where `APP=`, `GH_REPO=`, and an `apt` install line each
  directly follow a backslash-continuation line analyses identically before and after the join —
  proving the joined text reaches only the two release-call collectors.
- Zero-parse observability: update file present, no allowlisted URL → `warn!` emitted containing
  the first unmatched URL-like token.
- End-to-end fixture: an update-file line for the uptrakit source + CT/install script fixtures →
  discovery target with GitHub source `worried-networking/uptrakit` and
  `version_file_basename = Some("uptrakit-controller-standalone")`.

Part B:

- Moved sudoers tests keep passing in `uptrakit-agent-core` (scripted `RemoteExecutor` double —
  reuse the shared `uptrakit-command` test-support double if the PVE bootstrap spec has landed it
  by then; otherwise the module's existing private double moves along and consolidation joins that
  spec's deferred item).
- `LocalRemoteExecutor`: shell-mode execution keeps stdout and stderr separate and maps
  zero/non-zero exit codes correctly (success + failing command writing to stderr).
- Atomic write (D10): `visudo -cf` failure on the tmp file → tmp removed, live drop-in untouched
  (scripted executor asserts no `mv` ran); success path asserts tmp → chmod → visudo → mv order.
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
5. Second `bootstrap-host` run succeeds and converges: it rewrites (validated, atomic) but the
   resulting sudoers file and helper content are byte-identical to the first run's.
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
  removed, `agent bootstrap-host` call, remediation command) and the updater self-heal: existing
  installs repair helper/sudoers on their next `update` run, no manual step needed.
- `docs/end-user/plugin-configs.md` — PHS auto-discovery notes (`:120-124`) if the source list is
  user-visible there (check at plan time).
- `scripts/pvehs/install/uptrakit-install.sh` — the D11 change itself (code deliverable, listed for
  completeness).
- `scripts/pvehs/ct/uptrakit.sh` — `update_script()` self-heal calls (code deliverable, listed for
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
