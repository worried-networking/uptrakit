use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, DiscoveredSoftware, DiscoveryTarget, HostCompatibility,
    HostRequirements, HostRuntime, PluginCapability, PluginConfig, PluginConfigValidationError,
    PluginError, PluginFamily, PluginHttpClientConfig, PluginRole, RedirectMode, Result, SsrfMode,
    build_plugin_http_client, declare_plugin, execute_and_capture, plugin_ids,
};
use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::UvConfig;
use crate::error::UvError;

/// PEP 503/508 project-name charset: ASCII alphanumeric plus `.`, `_`, `-`;
/// must start (and per PEP 508 end, enforced loosely here) alphanumeric.
const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 128,
    first_char_valid: |c| c.is_ascii_alphanumeric(),
    char_valid: |c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-',
    reject_double_dot: true,
};

/// Validate a uv tool package identifier (PEP 503/508 project-name charset).
///
/// uv prints PEP 503-normalized names (`ruamel.yaml.cmd` → `ruamel-yaml-cmd`),
/// but the raw charset still admits `.` and `_` so operator-entered
/// identifiers in either form validate. `..` and path separators are rejected
/// (the identifier is later used as a path segment under the uv tools dir).
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    IDENTIFIER_RULES
        .validate(value)
        .map_err(PluginConfigValidationError::InvalidIdentifier)
}

/// Parse `uv tool list` output into a tool name → version map.
///
/// Stdout format (verified on uv 0.11.29):
///
/// ```text
/// ruff v0.6.8
/// - ruff
/// ```
///
/// The input is the **merged stdout+stderr stream** (`CommandOutput.output`
/// concatenates both), so acceptance is hard-anchored. A line is accepted only
/// when, split at the first `" v"`:
/// - the name part starts at column 0, is non-empty, and passes the
///   PEP 503/508 identifier charset (leading whitespace, `- ` bullets,
///   `warning:`/`note:` prefixes all fail this);
/// - the version part starts with an ASCII digit (PEP 440 normalized forms
///   cannot start otherwise) and contains no whitespace — any trailing
///   content rejects the line.
///
/// This function never errors: non-matching lines (entrypoint bullets,
/// `No tools installed` on stderr, uv warnings/notices) are skipped without
/// error, and the degenerate all-noise case yields an empty map,
/// indistinguishable at this layer from "no tools installed". Telling
/// genuine format drift apart from real emptiness is the caller's job —
/// `discover_software`'s format-drift guard treats output whose non-blank
/// lines are all uv diagnostic lines (see `is_uv_diagnostic_line`) or the
/// `No tools installed` sentinel as the empty case, and bails on anything
/// else.
pub fn parse_uv_tool_list(output: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in output.lines() {
        let Some((name, version)) = line.split_once(" v") else {
            continue;
        };
        if validate_identifier(name).is_err() {
            continue;
        }
        if !version.starts_with(|c: char| c.is_ascii_digit())
            || version.contains(char::is_whitespace)
        {
            continue;
        }
        result.insert(name.to_string(), version.to_string());
    }
    result
}

/// uv diagnostic line prefixes seen on stderr (warnings, errors, notes).
///
/// `CommandOutput.output` merges stdout and stderr (see its doc comment on
/// `CommandOutput`), so any uv diagnostic printed to stderr — e.g.
/// `warning: Ignoring malformed tool 'ruff': receipt is corrupted` for a
/// corrupted tool receipt — lands in the same string `discover_software`
/// parses. A line matching one of these prefixes is stderr noise, not
/// evidence that the `uv tool list` format itself changed.
const UV_DIAGNOSTIC_PREFIXES: &[&str] = &["warning:", "error:", "note:"];

/// True when `line`, after stripping leading whitespace, starts with a known
/// uv diagnostic prefix.
fn is_uv_diagnostic_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    UV_DIAGNOSTIC_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// True when `output` carries no evidence of genuine `uv tool list` format
/// drift.
///
/// The naive test — "parsed to zero tools but the raw output is non-empty"
/// — is wrong: `output` is stdout *and* stderr concatenated, so a single uv
/// diagnostic on stderr (malformed tool receipt, deprecation notice, …)
/// makes it non-empty even when stdout genuinely listed nothing. That must
/// not be treated as drift. Real drift means at least one non-blank line
/// looks like it *isn't* stderr noise: `output` is blank, contains the
/// `No tools installed` sentinel, or every non-blank line is a uv
/// diagnostic line ([`is_uv_diagnostic_line`]).
fn output_has_no_drift_evidence(output: &str) -> bool {
    output.contains("No tools installed")
        || output
            .lines()
            .all(|line| line.trim().is_empty() || is_uv_diagnostic_line(line))
}

/// Plugin for tracking Python CLI tools installed via `uv tool install`.
///
/// - **Discovery**: `uv tool list` — one featured software item per tool.
/// - **Version detection**: `uv tool list` — single call, looked up in the map.
/// - **Release fetching**: PyPI Simple API (`https://pypi.org/simple` by
///   default, overridable via `index_url`) — controller-side, bounded
///   parallel HTTP lookups via `buffer_unordered(10)`.
/// - Update execution (Plan 3) follows.
///
/// **Scope:** the agent user's tools only (`uv tool list` sees one user's
/// tools dir). Tools installed by other users are invisible — consistent with
/// the unprivileged-agent invariant. The agent user's `PATH` must include
/// uv's bin dir (typically `~/.local/bin`).
pub struct UvPlugin {
    pub(crate) config: UvConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
    pub(crate) client: reqwest::Client,
}

impl UvPlugin {
    /// Create a new uv plugin with the given configuration and host runtime.
    pub fn new(
        config: UvConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        config.validate().map_err(|e| e.to_string())?;

        // Permissive SSRF resolver for custom (potentially private/LAN)
        // indexes; strict resolver for the pypi.org default.
        let ssrf_mode = if config.index_url.is_some() {
            SsrfMode::Permissive
        } else {
            SsrfMode::Strict
        };

        let client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-package-manager-uv/",
                env!("CARGO_PKG_VERSION")
            ),
            ssrf_mode,
            redirect: RedirectMode::Limited { hops: 10 },
            ..Default::default()
        })?;

        Ok(Self {
            config,
            executor,
            client,
        })
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier,
        )
    }
}

// ── Plugin descriptor ─────────────────────────────────────────────────────
// Plan 3 adds: UpdateExecutor role + ConfigTestKind::UpdateCommandValidation.

declare_plugin!(UvPlugin, UvConfig, "package-manager.uv", {
    display_name: "uv Tools",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection],
    type_settings: true,
    roles: [Discoverer, VersionDetector, ReleaseFetcher],
    extra_capabilities: [PluginCapability::ControllerSideFetchReleases],
});

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for UvPlugin {
    /// Discover tools installed via `uv tool install` for the agent user.
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering uv-installed tools");

        let output = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("uv", ["tool".to_string(), "list".to_string()]),
            "uv tool list",
        )
        .await?;

        let installed = parse_uv_tool_list(&output);

        if installed.is_empty() && !output_has_no_drift_evidence(&output) {
            // Format-drift guard: output that parses to zero tools *and*
            // contains at least one non-blank line that isn't stderr noise
            // (see `output_has_no_drift_evidence`) means the hard-anchored
            // parser no longer matches the `uv tool list` format (e.g. a
            // future uv release). Degrading to `Ok(vec![])` here would
            // assert "this host has no uv tools" and deactivate every
            // previously discovered uv item fleet-wide with nothing
            // distinguishing it from real removal — fail loud instead so
            // the parse failure surfaces as `Err`.
            //
            // Traded-off case: a host where *every* installed uv tool has a
            // malformed/missing receipt prints only diagnostic lines and
            // exits 0 — the tools are genuinely installed but none get
            // listed. The merged stdout+stderr stream gives this guard no
            // way to tell that apart from "listed nothing because there is
            // nothing", so it reports zero tools and those items deactivate
            // after the two-miss hysteresis (ADR-0027) rather than erroring
            // every cycle. That is the accepted trade: a permanent hard
            // failure on an otherwise-healthy host would be worse than a
            // deactivation that a subsequent successful discovery reverses.
            tracing::warn!(
                output_len = output.len(),
                "uv tool list output parsed to zero tools; possible uv output format change"
            );
            // Typed at the crate boundary, converted to the infra-core
            // `PluginError` the `Discoverer` trait returns.
            return Err(report!(UvError::OutputFormatDrift(output.len())))
                .context_to::<PluginError>();
        }

        let tools: Vec<DiscoveredSoftware> = installed
            .into_iter()
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_UV.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "uv".to_string(),
                    // Plan 3 adds ExecuteUpdate.
                    roles: vec![PluginRole::DetectVersion, PluginRole::FetchReleases],
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }];
                DiscoveredSoftware {
                    package_identifier: name.clone(),
                    name,
                    installed_version: version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: true,
                    installed_display_version: None,
                }
            })
            .collect();

        tracing::debug!(count = tools.len(), "uv tool discovery complete");
        Ok(tools)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        // Non-zero `which` exit has a meaningful non-error interpretation
        // (uv absent), so call execute_quiet directly — the documented
        // carve-out from the execute_and_capture mandate.
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["uv".to_string()]))
            .await
        {
            Ok(out) if out.exit_code == 0 => Ok(HostCompatibility::Compatible),
            _ => Ok(HostCompatibility::Incompatible(
                "uv not found in PATH".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::{
        FixedOutputExecutor, test_runtime_with_executor,
    };
    use uptrakit_plugin_infrastructure_core::{
        Discoverer, HostCompatibility, PluginError, PluginRole, plugin_ids,
    };

    use crate::config::UvConfig;

    fn make_plugin(stdout: &str) -> UvPlugin {
        UvPlugin::new(
            UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::success(stdout)),
        )
        .expect("construct plugin")
    }

    fn make_failing_plugin(exit_code: i32) -> UvPlugin {
        UvPlugin::new(
            UvConfig::default(),
            test_runtime_with_executor(FixedOutputExecutor::failure(exit_code)),
        )
        .expect("construct plugin")
    }

    // ── descriptor ────────────────────────────────────────────────────────

    #[test]
    fn uv_plugin_no_sudo() {
        // All `uv tool` operations are per-user; the descriptor declares no
        // sudo commands (mirrors cargo's descriptor test).
        assert!(DESCRIPTOR.sudo.is_none());
    }

    /// `discover_software` emits `DiscoveryTarget.plugin_type` via the
    /// `plugin_ids::PACKAGE_MANAGER_UV` constant, while `declare_plugin!`
    /// carries the string literal `"package-manager.uv"` independently. A
    /// typo in either would compile and pass tests without this guard — the
    /// registry test that normally catches such drift
    /// (`descriptors_subset_of_known_ids`) does not cover uv, since registry
    /// activation is deferred to a later plan.
    #[test]
    fn descriptor_type_id_matches_plugin_ids_constant() {
        assert_eq!(DESCRIPTOR.type_id, plugin_ids::PACKAGE_MANAGER_UV.as_str());
    }

    #[test]
    fn uv_plugin_capabilities() {
        for cap in [
            PluginCapability::DiscoverLocalSoftware,
            PluginCapability::DetectHostCompatibility,
            PluginCapability::VersionDetection,
            PluginCapability::ConfigTest,
            PluginCapability::ReleaseFetching,
            PluginCapability::ControllerSideFetchReleases,
        ] {
            assert!(DESCRIPTOR.capabilities.contains(&cap), "missing {cap:?}");
        }
        // Plan 3 adds UpdateExecution (=> 7).
        assert_eq!(DESCRIPTOR.capabilities.len(), 6);
    }

    // ── discover_software ─────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_featured_targets() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n";
        let discovered = make_plugin(output).discover_software().await.unwrap();
        assert_eq!(discovered.len(), 2);
        for item in &discovered {
            assert!(item.featured, "uv tools are individual featured items");
            assert_eq!(item.targets.len(), 1);
            let target = item.targets.first().expect("one target");
            assert_eq!(target.plugin_type, plugin_ids::PACKAGE_MANAGER_UV.clone());
            assert_eq!(target.plugin_config, serde_json::json!({}));
            assert_eq!(target.plugin_config_name, "uv");
            // Plan 3 adds ExecuteUpdate.
            assert_eq!(
                target.roles,
                vec![PluginRole::DetectVersion, PluginRole::FetchReleases]
            );
        }
        let ruff = discovered
            .iter()
            .find(|i| i.package_identifier == "ruff")
            .unwrap();
        assert_eq!(ruff.installed_version, "0.6.8");
    }

    #[tokio::test]
    async fn discover_software_empty_list_is_ok() {
        // Exit 0 + `No tools installed` on stderr (merged stream).
        let discovered = make_plugin("No tools installed\n")
            .discover_software()
            .await
            .unwrap();
        assert!(discovered.is_empty());
    }

    /// Non-empty output that is neither `No tools installed` nor parseable
    /// (future uv format drift) must fail loud: degrading to `Ok(vec![])`
    /// would assert "no uv tools on this host" and deactivate every
    /// previously discovered uv item fleet-wide, indistinguishable from real
    /// removal.
    #[tokio::test]
    async fn discover_software_unparseable_output_is_err() {
        let Err(err) = make_plugin("something entirely different\n")
            .discover_software()
            .await
        else {
            panic!("expected discovery to fail on unparseable output");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }

    /// The realistic drift scenario: a future uv release drops the `v`
    /// prefix (`ruff 0.6.8` instead of `ruff v0.6.8`). This must still bail
    /// — it is real format drift, not stderr noise, so the F1 rewrite of the
    /// drift guard must not swallow it.
    #[tokio::test]
    async fn discover_software_realistic_drift_missing_v_prefix_is_err() {
        let Err(err) = make_plugin("ruff 0.6.8\nblack 24.4.2\n")
            .discover_software()
            .await
        else {
            panic!("expected discovery to fail on v-prefix-dropped drift");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }

    /// A uv diagnostic on stderr (merged into `output`) with zero tools
    /// parsed must NOT trip the drift guard: `output` is stdout+stderr
    /// concatenated (see `CommandOutput::output`'s doc comment), so a
    /// corrupted-receipt warning on stderr with nothing on stdout is a
    /// legitimate "no tools" state, not evidence the list format changed.
    /// Before the F1 fix this returned `Err` on every discovery cycle for
    /// any host with such a warning, permanently hiding uv discovery there.
    #[tokio::test]
    async fn discover_software_stderr_diagnostic_noise_with_no_tools_is_ok() {
        let discovered =
            make_plugin("warning: Ignoring malformed tool 'ruff': receipt is corrupted\n")
                .discover_software()
                .await
                .unwrap();
        assert!(discovered.is_empty());
    }

    /// The mixed case that isolates `.all` from `.any` in
    /// `output_has_no_drift_evidence`: one uv diagnostic line plus one
    /// non-diagnostic, unparseable line (drift, reproduced against real uv
    /// 0.12.5 as a malformed-receipt warning alongside a v-prefix-dropped
    /// listing line). `.all(diagnostic-or-blank)` is false here (the second
    /// line is neither), so the guard correctly sees drift evidence and
    /// bails — this is the single input that distinguishes the current
    /// design from a broken `.any` variant, which would see the diagnostic
    /// line alone and wrongly call it noise-only.
    #[tokio::test]
    async fn discover_software_mixed_diagnostic_and_drift_lines_is_err() {
        let output = "warning: Ignoring malformed tool `ruff`\nblack 24.4.2\n";
        let Err(err) = make_plugin(output).discover_software().await else {
            panic!("expected discovery to fail on mixed diagnostic+drift output");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }

    /// `uv tool list` failure surfaces `PluginError::PluginInternal`:
    /// real executors return `Err(CommandError::CommandFailed)` on non-zero
    /// exit and `execute_and_capture` folds any executor `Err` into
    /// `PluginInternal`. `FixedOutputExecutor::failure` matches that contract
    /// on `execute_quiet`. Never assert `PluginError::CommandFailed` here —
    /// that arm is production-unreachable.
    #[tokio::test]
    async fn discover_software_command_failure_is_plugin_internal() {
        let Err(err) = make_failing_plugin(1).discover_software().await else {
            panic!("expected discovery to fail");
        };
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }

    // ── detect_host_compatibility ─────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_uv_found() {
        let result = make_plugin("").detect_host_compatibility().await.unwrap();
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_uv_missing() {
        let result = make_failing_plugin(1)
            .detect_host_compatibility()
            .await
            .unwrap();
        assert!(matches!(result, HostCompatibility::Incompatible(_)));
    }

    #[test]
    fn validate_identifier_valid_names() {
        validate_identifier("ruff").unwrap();
        validate_identifier("ruamel-yaml-cmd").unwrap();
        validate_identifier("ruamel.yaml.cmd").unwrap();
        validate_identifier("typing_extensions").unwrap();
        validate_identifier("2to3").unwrap(); // digits are valid PEP 508 first chars
        validate_identifier("a").unwrap();
        validate_identifier(&"a".repeat(128)).unwrap();
    }

    #[test]
    fn validate_identifier_invalid_names() {
        validate_identifier("").unwrap_err();
        validate_identifier(&"a".repeat(129)).unwrap_err();
        validate_identifier("-ruff").unwrap_err();
        validate_identifier(".ruff").unwrap_err();
        validate_identifier("a..b").unwrap_err();
        validate_identifier("owner/pkg").unwrap_err();
        validate_identifier("pkg name").unwrap_err();
        validate_identifier("pkg==1.0").unwrap_err();
    }

    // ── parse_uv_tool_list ────────────────────────────────────────────────

    #[test]
    fn parse_uv_tool_list_basic() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n- blackd\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
        assert_eq!(map.get("black"), Some(&"24.4.2".to_string()));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_uv_tool_list_skips_entrypoint_bullets_and_indented_lines() {
        let map = parse_uv_tool_list("- ruff\n    ruff v0.6.8\n");
        assert!(map.is_empty());
    }

    /// A fixture that actually reaches the parser's guards (it contains
    /// `" v"`, so `split_once` accepts it) but fails the version-digit
    /// guard: `"Not vinstalled"` splits into name `"Not"` (a valid
    /// identifier) and version `"installed"` (does not start with a digit).
    /// Unlike the old `"No tools installed"` fixture this replaces — which
    /// contains no `" v"` and so is rejected by `split_once` before either
    /// guard runs, pinning nothing — this catches removal of the
    /// version-digit check: without it, `map` would gain a phantom
    /// `"Not" -> "installed"` entry. Sentinel-path coverage (`No tools
    /// installed` never produces an entry, and the empty-vs-drift split it
    /// enables) lives at the `discover_software` level, where it is
    /// load-bearing: see `discover_software_empty_list_is_ok` and
    /// `discover_software_unparseable_output_is_err`.
    #[test]
    fn parse_uv_tool_list_rejects_name_with_non_digit_version() {
        let map = parse_uv_tool_list("Not vinstalled\n");
        assert!(map.is_empty());
    }

    /// The realistic drift scenario: a future uv release drops the `v`
    /// prefix (`ruff 0.6.8`, no `v`). No `" v"` substring exists in the
    /// line, so `split_once` rejects it and the map is empty — pinning
    /// `split_once(" v")` specifically: mutating the pattern to
    /// `split_once(' ')` would instead parse `name = "ruff"`,
    /// `version = "0.6.8"` and populate the map.
    #[test]
    fn parse_uv_tool_list_missing_v_prefix_yields_empty() {
        assert!(parse_uv_tool_list("ruff 0.6.8\n").is_empty());
    }

    /// Merged-stream fixture: a stderr warning line is rejected by the
    /// identifier guard alone (the name part `warning: foo` contains a
    /// colon, which fails the PEP 503/508 charset, even though the version
    /// part `2.0` is otherwise clean), isolating that guard from the
    /// version-digit guard.
    #[test]
    fn parse_uv_tool_list_interleaved_warning_line() {
        let output = "warning: foo v2.0\nruff v0.6.8\n- ruff\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
    }

    /// PEP 440 normalized forms are not semver; the parser must accept them
    /// verbatim as the map value (no semver-shaped validation is performed).
    /// Includes an epoch segment (`1!2.0`) — passes today, pinned so it
    /// stays passing.
    #[test]
    fn parse_uv_tool_list_accepts_pep440_version_forms() {
        let output = "ruff v1.0.0rc1\nblack v2.0.0b3\nmypy v1.0.post1\npoetry v1.0.dev0\nhttpie v1.0+local\ntox v1!2.0\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"1.0.0rc1".to_string()));
        assert_eq!(map.get("black"), Some(&"2.0.0b3".to_string()));
        assert_eq!(map.get("mypy"), Some(&"1.0.post1".to_string()));
        assert_eq!(map.get("poetry"), Some(&"1.0.dev0".to_string()));
        assert_eq!(map.get("httpie"), Some(&"1.0+local".to_string()));
        assert_eq!(map.get("tox"), Some(&"1!2.0".to_string()));
        assert_eq!(map.len(), 6);
    }

    /// `str::lines` strips a trailing `\r` from CRLF-terminated input, so
    /// Windows-style line endings must yield clean version values.
    #[test]
    fn parse_uv_tool_list_handles_crlf_line_endings() {
        let output = "ruff v0.6.8\r\nblack v24.8.0\r\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
        assert_eq!(map.get("black"), Some(&"24.8.0".to_string()));
        assert_eq!(map.len(), 2);
    }

    /// Two lines for the same tool name: `HashMap::insert` overwrites the
    /// prior value, so the later line's version wins deliberately.
    #[test]
    fn parse_uv_tool_list_duplicate_name_last_wins() {
        let output = "ruff v0.6.8\nruff v0.6.9\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.9".to_string()));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_uv_tool_list_rejects_trailing_content_and_non_digit_versions() {
        assert!(parse_uv_tool_list("ruff v0.6.8 (extra)\n").is_empty());
        assert!(parse_uv_tool_list("ruff vlatest\n").is_empty());
        assert!(parse_uv_tool_list("ruff v\n").is_empty());
    }

    // Note: no dedicated "empty input" parser test. `parse_uv_tool_list("")`
    // and whitespace-only input trivially yield an empty map regardless of
    // guard implementation (no line ever reaches `split_once`), so such a
    // test pins nothing per this round's mutation criterion — see
    // `discover_software_empty_list_is_ok` for the load-bearing empty-input
    // coverage (through the sentinel, at the layer where it matters).
}
