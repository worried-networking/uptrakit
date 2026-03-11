use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, BatchUpdateItem,
    BatchUpdateResult, DiscoveredSoftware, DiscoveryTarget, HostCompatibility, OutputStreamType,
    Plugin, PluginCapability, PluginError, PluginRole, PluginType, PreUpdateHookResult,
    ReleaseInfo, Result, SudoCommandEntry, UpdateCategory, UpdateHookContext, UpdateOutputLine,
    UpstreamRelease, Version,
};

use crate::config::{AptConfig, AptDiscoveryFilter};

/// Fixed path for the temporary APT preferences file used during batch updates.
///
/// This path is hardcoded on both the write side (`execute_batch_update`) and
/// the sudoers declaration side (`required_sudo_commands`) so that the sudoers
/// rule can be maximally restrictive: the rule locks in exactly this path and
/// no other. Changing this value requires updating both uses simultaneously.
pub(crate) const APT_BATCH_PREF_FILE: &str = "/tmp/uptrakit-apt-batch.pref";

/// Validate a Debian APT package identifier.
///
/// Enforces Debian package naming rules from the Debian Policy Manual:
/// - Between 2 and 64 characters long.
/// - Must start with a lowercase letter or digit (`[a-z0-9]`).
/// - May only contain lowercase letters, digits, `+`, `-`, and `.`.
/// - Must not contain `..` (path traversal protection).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value.len() < 2 {
        return Err("package_identifier must be at least 2 characters long".to_string());
    }
    if value.len() > 64 {
        return Err("package_identifier must not exceed 64 characters".to_string());
    }

    // Must start with [a-z0-9]. The empty check above guarantees chars().next()
    // returns Some, so the unwrap_or path is unreachable in practice.
    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "package_identifier must start with a lowercase letter or digit, found '{first}'"
        ));
    }

    // All characters must be in [a-z0-9+\-.].
    for ch in value.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && !matches!(ch, '+' | '-' | '.') {
            return Err(format!(
                "package_identifier contains invalid character: '{ch}'"
            ));
        }
    }

    // No path traversal via '..'.
    if value.contains("..") {
        return Err("package_identifier must not contain '..'".to_string());
    }

    Ok(())
}

/// Validate a Debian APT version string before it is interpolated into install commands.
///
/// Allows Debian version characters (`[a-zA-Z0-9.+~:-]`). Rejects:
/// - Empty strings
/// - Strings starting with `-` (could be interpreted as a command-line flag by apt-get)
/// - Strings exceeding 256 characters
pub fn validate_version(version: &str) -> std::result::Result<(), String> {
    if version.is_empty() {
        return Err("version must not be empty".to_string());
    }
    if version.len() > 256 {
        return Err("version must not exceed 256 characters".to_string());
    }
    if version.starts_with('-') {
        return Err("version must not start with '-' (would be interpreted as a flag)".to_string());
    }
    for ch in version.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '+' | '~' | ':' | '-') {
            return Err(format!("version contains invalid character: '{ch}'"));
        }
    }
    Ok(())
}

/// Plugin for APT (Debian/Ubuntu package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for Debian packages managed by `apt-get`.
///
/// The `package_identifier` in `SoftwareItem` is the Debian package name
/// (e.g., `nginx`, `python3`, `apt-utils`).
pub struct AptPlugin {
    config: AptConfig,
    executor: Arc<dyn CommandExecutor>,
}

/// Parsed result from a single `apt-cache madison` line.
struct MadisonEntry {
    version: String,
    source: String,
}

impl AptPlugin {
    /// Compile-time capabilities for the APT plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::RefreshPackageIndex,
        PluginCapability::DetectHostCompatibility,
        PluginCapability::PostUpdateHook,
    ];

    /// Create a new APT plugin with the given configuration.
    pub async fn new(config: AptConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Parse `dpkg-query --show --showformat=${Package}\t${Version}\n` output.
    ///
    /// Each line is a tab-separated `package\tversion` pair. Lines with an
    /// empty version are skipped.
    fn parse_dpkg_output(output: &str) -> Vec<(String, String)> {
        output
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(2, '\t');
                let name = parts.next()?.trim();
                let version = parts.next()?.trim();
                if name.is_empty() || version.is_empty() {
                    None
                } else {
                    Some((name.to_string(), version.to_string()))
                }
            })
            .collect()
    }

    /// Parse `apt-cache madison <package>` output.
    ///
    /// Each line has the format:
    /// `   <package> | <version> | <source>`
    ///
    /// Returns the version and source from the first valid line
    /// (highest-priority candidate), or `None` if the output is empty or
    /// contains no parseable lines.
    fn parse_madison_output(output: &str) -> Option<MadisonEntry> {
        output.lines().find_map(|line| {
            let mut parts = line.splitn(3, '|');
            let _package = parts.next()?;
            let version = parts.next()?.trim();
            if version.is_empty() {
                return None;
            }
            let source = parts
                .next()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            Some(MadisonEntry {
                version: version.to_string(),
                source,
            })
        })
    }

    /// Detect whether a madison source string indicates a security repository.
    ///
    /// APT security updates typically come from URLs containing "security"
    /// (e.g. `http://security.ubuntu.com/ubuntu noble-security/main`).
    fn is_security_source(source: &str) -> bool {
        source.to_ascii_lowercase().contains("security")
    }

    /// Parse `apt-cache madison pkg1 pkg2 ...` output for a batch query.
    ///
    /// Lines from a multi-package madison query are interleaved:
    /// ```text
    ///    nginx | 1.24.0 | http://archive.ubuntu.com/ubuntu noble/main amd64 Packages
    ///    curl  | 7.88.1 | http://deb.debian.org/debian bookworm/main amd64 Packages
    ///    nginx | 1.18.0 | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages
    /// ```
    ///
    /// Groups lines by package name (first `|`-delimited field). For each
    /// package, only the *first* line is used (highest-priority candidate;
    /// madison output is already ordered by pin priority).
    fn parse_madison_output_batch(output: &str) -> HashMap<String, MadisonEntry> {
        let mut results: HashMap<String, MadisonEntry> = HashMap::new();
        for line in output.lines() {
            let mut parts = line.splitn(3, '|');
            let Some(pkg_name) = parts.next() else {
                continue;
            };
            let pkg_name = pkg_name.trim().to_string();
            if pkg_name.is_empty() {
                continue;
            }
            let Some(version) = parts.next() else {
                continue;
            };
            let version = version.trim();
            if version.is_empty() {
                continue;
            }
            let source = parts
                .next()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            // Only keep the first entry per package (highest priority).
            results.entry(pkg_name).or_insert_with(|| MadisonEntry {
                version: version.to_string(),
                source,
            });
        }
        results
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier).map_err(|e| report!(PluginError::Configuration(e)))
    }
}

#[async_trait]
impl Plugin for AptPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::PackageManagerApt
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["apt-get".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "apt-get not found".to_string(),
            )),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn post_update_hook(
        &self,
        _ctx: &UpdateHookContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()> {
        // `test -f` exits 0 when the file exists, non-zero when absent.
        // `execute_quiet` returns Err on any non-zero exit, so Ok(_) means
        // the reboot-required file is present.
        if self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "test",
                ["-f".to_string(), "/var/run/reboot-required".to_string()],
            ))
            .await
            .is_ok()
        {
            send_output(
                output_tx,
                "[post-hook] Reboot required to complete the update.",
                OutputStreamType::Stdout,
            )
            .await;
        }

        Ok(())
    }

    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("apt-get", "Package index refresh requires root privileges")
                // Restrict to `apt-get update` only (with optional flags).
                .with_args_suffix(Cow::Borrowed("update *"))
                .with_setenv(),
            SudoCommandEntry::new("apt-get", "Package installation requires root privileges")
                // Restrict to `apt-get install` only; covers single and batch installs.
                .with_args_suffix(Cow::Borrowed("install *"))
                .with_setenv(),
            SudoCommandEntry::new(
                "apt-get",
                "Batch package upgrade (pinned versions) requires root privileges",
            )
            // Lock in the exact -o Dir::Etc::Preferences= invocation that
            // execute_batch_update uses. The path is intentionally hardcoded on
            // both sides; see APT_BATCH_PREF_FILE. Using `apt-get upgrade` (not
            // `install`) preserves the apt manual/auto install mark — packages
            // auto-installed as dependencies keep their `auto` mark, allowing
            // `apt autoremove` to clean them up correctly.
            .with_args_suffix(Cow::Owned(format!(
                "-o Dir::Etc::Preferences={APT_BATCH_PREF_FILE} upgrade *"
            )))
            .with_setenv(),
        ]
    }

    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing APT package index");
        let cmd_output = self
            .executor
            .execute_quiet(
                &CommandSpec::exec("apt-get", ["update".to_string(), "-q".to_string()])
                    .with_env("DEBIAN_FRONTEND", "noninteractive")
                    .privileged(),
            )
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "apt-get update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("APT package index refreshed");
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering APT-managed software");

        // Step 1: Query all installed packages from dpkg.
        let dpkg_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dpkg-query",
                [
                    "--show".to_string(),
                    "--showformat=${Package}\\t${Version}\\n".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "dpkg-query failed: {e}"
                )))
            })?;

        if dpkg_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(dpkg_output.exit_code));
        }

        let all_packages = Self::parse_dpkg_output(&dpkg_output.output);

        // Step 2: For the Manual filter, build a set of manually-installed packages.
        let manual_set: Option<HashSet<String>> = match self.config.effective_filter() {
            AptDiscoveryFilter::Manual => {
                let mark_output = self
                    .executor
                    .execute_quiet(&CommandSpec::exec("apt-mark", ["showmanual".to_string()]))
                    .await
                    .map_err(|e| {
                        report!(PluginError::PluginInternal(format!(
                            "apt-mark showmanual failed: {e}"
                        )))
                    })?;

                if mark_output.exit_code != 0 {
                    bail!(PluginError::CommandFailed(mark_output.exit_code));
                }

                let set: HashSet<String> = mark_output
                    .output
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                Some(set)
            }
            AptDiscoveryFilter::All => None,
        };

        // Step 3: Filter by the manual set (if applicable) and build results.
        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .filter(|(name, _)| {
                manual_set
                    .as_ref()
                    .is_none_or(|set| set.contains(name.as_str()))
            })
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: PluginType::PackageManagerApt,
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "APT".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
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
                    featured: false,
                }
            })
            .collect();

        tracing::debug!(count = packages.len(), "APT software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting APT installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dpkg-query",
                [
                    "--show".to_string(),
                    "--showformat=${Version}\\n".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "dpkg-query failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                let version = cmd_output.output.trim().to_string();
                if version.is_empty() {
                    return Ok(None);
                }
                tracing::debug!(version = %version, "APT installed version detected");
                Ok(Some(Version::new(&version)))
            }
            // Exit code 1 means the package was not found.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in dpkg database"
                );
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching APT releases via apt-cache madison");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "apt-cache",
                ["madison".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "apt-cache madison failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let Some(entry) = Self::parse_madison_output(&cmd_output.output) else {
            // Package not found in any configured repository.
            return Ok(vec![]);
        };

        let category = if Self::is_security_source(&entry.source) {
            Some(UpdateCategory::Security)
        } else {
            None
        };

        tracing::debug!(
            version = %entry.version,
            ?category,
            source = %entry.source,
            "APT upstream version resolved"
        );
        Ok(vec![{
            let mut release =
                UpstreamRelease::new(Version::new(&entry.version), entry.version, false, "");
            release.category = category;
            release
        }])
    }

    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version).map_err(|e| report!(PluginError::Configuration(e)))?;

        let pkg_version = format!("{package_identifier}={to_version}");
        let args = vec![
            "install".to_string(),
            "--yes".to_string(),
            "--no-install-recommends".to_string(),
            pkg_version,
        ];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running apt-get install"
        );

        let display_args = std::iter::once("apt-get")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        send_output(
            output_tx,
            &format!("Running: {display_args}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        let cmd_output = self
            .executor
            .execute(
                &CommandSpec::exec("apt-get", args)
                    .with_env("DEBIAN_FRONTEND", "noninteractive")
                    .privileged(),
                output_tx,
            )
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "apt-get install failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Execute batch updates using APT preferences pin-priority mechanism.
    ///
    /// Writes a temporary preferences file that blocks all upgrades except the
    /// specifically targeted packages, then runs `apt-get upgrade --yes`.
    /// The temp file is written to a user-writable path (no sudo needed) and
    /// deleted after the upgrade completes.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all package identifiers and versions up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
            validate_version(&item.to_version)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        // Build apt_preferences content:
        // 1. Block all upgrades by default
        // 2. Allow only our targeted packages at pin-priority 990
        let mut prefs = String::from(
            "# uptrakit batch update preferences (temporary)\n\
             Package: *\n\
             Pin: release *\n\
             Pin-Priority: -1\n\n",
        );
        for item in items {
            prefs.push_str(&format!(
                "Package: {}\nPin: version {}\nPin-Priority: 990\n\n",
                item.package_identifier, item.to_version
            ));
        }

        // APT_BATCH_PREF_FILE is hardcoded here and in required_sudo_commands()
        // so the sudoers rule locks down this exact -o Dir::Etc::Preferences=
        // invocation. Never change this path without updating both.
        let pref_path = std::path::Path::new(APT_BATCH_PREF_FILE);
        std::fs::write(pref_path, &prefs).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to write apt preferences file: {e}"
            )))
        })?;

        let pref_path_str = pref_path.display().to_string();
        let dir_opt = format!("Dir::Etc::Preferences={pref_path_str}");

        let args = vec![
            "-o".to_string(),
            dir_opt,
            "upgrade".to_string(),
            "--yes".to_string(),
        ];

        let display_args = std::iter::once("apt-get")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages: {}\nRunning: {display_args}",
                items.len(),
                pkg_list.join(", ")
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running apt-get batch upgrade"
        );

        let cmd_result = self
            .executor
            .execute(
                &CommandSpec::exec("apt-get", args)
                    .with_env("DEBIAN_FRONTEND", "noninteractive")
                    .privileged(),
                output_tx,
            )
            .await;

        // Always clean up the temp file (no sudo needed).
        if let Err(e) = std::fs::remove_file(pref_path) {
            tracing::warn!(path = %pref_path_str, error = %e, "failed to remove apt preferences file");
        }

        let cmd_output =
            cmd_result.map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        let success = cmd_output.exit_code == 0;
        let results = items
            .iter()
            .map(|item| {
                BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
            })
            .collect();

        Ok(results)
    }

    /// Detect installed versions for multiple packages using a single `dpkg-query` call.
    ///
    /// Runs:
    /// ```text
    /// dpkg-query --show --showformat='${Package}\t${Version}\n' pkg1 pkg2 pkg3
    /// ```
    ///
    /// The exit code is intentionally ignored: `dpkg-query` exits non-zero when any
    /// requested package is unknown, but packages that *are* found still appear in
    /// stdout. Packages absent from stdout are treated as not installed (`None` with
    /// no error).
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec![
            "--show".to_string(),
            "--showformat=${Package}\\t${Version}\\n".to_string(),
        ];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting APT installed versions"
        );

        // Non-zero exit is expected when any package is unknown; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("dpkg-query", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                // dpkg-query completely failed (e.g., not found on PATH).
                let error_str = format!("dpkg-query failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        // Parse output into a map for O(1) lookup.
        let dpkg_map: HashMap<String, String> =
            Self::parse_dpkg_output(&stdout).into_iter().collect();

        let results = items
            .iter()
            .map(|item| {
                let installed_version = dpkg_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(count = items.len(), "APT batch version detection complete");
        Ok(results)
    }

    /// Fetch available releases for multiple packages using a single `apt-cache madison` call.
    ///
    /// Runs:
    /// ```text
    /// apt-cache madison pkg1 pkg2 pkg3
    /// ```
    ///
    /// Output lines are grouped by package name; only the first (highest-priority)
    /// entry per package is used. Packages absent from the output have empty releases.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["madison".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch fetching APT releases via apt-cache madison"
        );

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("apt-cache", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "apt-cache madison failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let parsed = Self::parse_madison_output_batch(&cmd_output.output);

        let results = items
            .iter()
            .map(|item| {
                let Some(entry) = parsed.get(&item.package_identifier) else {
                    // Package not found in any configured repository.
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                let category = if Self::is_security_source(&entry.source) {
                    Some(UpdateCategory::Security)
                } else {
                    None
                };

                let release = {
                    let mut r = UpstreamRelease::new(
                        Version::new(&entry.version),
                        entry.version.clone(),
                        false,
                        "",
                    );
                    r.category = category;
                    r
                };
                BatchFetchResult::found(item.package_identifier.clone(), vec![release])
            })
            .collect();

        tracing::debug!(count = items.len(), "APT batch fetch complete");
        Ok(results)
    }
}

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    AptPlugin,
    AptConfig,
    "package_manager_apt",
    {
        fn capabilities(&self) -> Vec<PluginCapability> {
            Self::CAPABILITIES.to_vec()
        }
        fn required_sudo_commands(
            &self,
        ) -> Vec<uptrakit_plugin_infrastructure_core::SudoCommandEntry> {
            Plugin::required_sudo_commands(self)
        }
    }
);

#[async_trait]
impl uptrakit_plugin_infrastructure_core::DiscoveryPlugin for AptPlugin {
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        Plugin::discover_software(self).await
    }

    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Plugin::detect_host_compatibility(self).await
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for AptPlugin {
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        Plugin::detect_installed_version(self, package_identifier).await
    }

    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        Plugin::batch_detect_installed_version(self, items).await
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for AptPlugin {
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Plugin::fetch_releases(self, package_identifier).await
    }

    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        Plugin::batch_fetch_releases(self, items).await
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexPlugin for AptPlugin {
    async fn refresh_package_index(&self) -> Result<()> {
        Plugin::refresh_package_index(self).await
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for AptPlugin {
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        Plugin::execute_update(
            self,
            package_identifier,
            to_version,
            release_info,
            output_tx,
        )
        .await
    }

    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        Plugin::execute_batch_update(self, items, output_tx).await
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateHooksPlugin for AptPlugin {
    async fn pre_update_hook(
        &self,
        ctx: &UpdateHookContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<PreUpdateHookResult> {
        Plugin::pre_update_hook(self, ctx, output_tx).await
    }

    async fn post_update_hook(
        &self,
        ctx: &UpdateHookContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()> {
        Plugin::post_update_hook(self, ctx, output_tx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{CommandOutput, LocalCommandExecutor};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    /// Mock executor that routes `execute_quiet` output by the command program name.
    ///
    /// Matches the program name from `CommandSpec::mode` (Exec variant only).
    /// Falls back to an empty-output success for Shell-mode or unrecognised programs.
    struct RoutedOutputExecutor {
        /// `(program_name, output_to_return)` entries checked in order.
        routes: Vec<(&'static str, String)>,
    }

    impl RoutedOutputExecutor {
        /// Create an executor from a list of `(program, output)` pairs.
        fn with_routes(routes: Vec<(&'static str, &'static str)>) -> Arc<dyn CommandExecutor> {
            Arc::new(Self {
                routes: routes
                    .into_iter()
                    .map(|(p, o)| (p, o.to_string()))
                    .collect(),
            })
        }

        fn output_for(&self, spec: &CommandSpec) -> String {
            use uptrakit_plugin_infrastructure_core::CommandMode;
            if let CommandMode::Exec { program, .. } = &spec.mode {
                for (name, out) in &self.routes {
                    if program == *name {
                        return out.clone();
                    }
                }
            }
            String::new()
        }
    }

    #[async_trait]
    impl CommandExecutor for RoutedOutputExecutor {
        async fn execute(
            &self,
            spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output_for(spec),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output_for(spec),
                exit_code: 0,
            })
        }
    }

    /// Mock executor that returns a configurable exit code for `execute_quiet`.
    struct FixedExitCodeExecutor {
        exit_code: i32,
    }

    impl FixedExitCodeExecutor {
        fn with_exit_code(exit_code: i32) -> Arc<dyn CommandExecutor> {
            Arc::new(Self { exit_code })
        }
    }

    #[async_trait]
    impl CommandExecutor for FixedExitCodeExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: String::new(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            if self.exit_code == 0 {
                Ok(CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            } else {
                use rootcause::prelude::*;
                bail!(uptrakit_command::CommandError::CommandFailed(
                    self.exit_code
                ))
            }
        }
    }

    // ── validate_identifier ──────────────────────────────────────────────

    #[test]
    fn validate_identifier_valid_simple() {
        assert!(validate_identifier("nginx").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dash() {
        assert!(validate_identifier("apt-utils").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_plus() {
        assert!(validate_identifier("g++").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dot() {
        assert!(validate_identifier("python3.11").is_ok());
    }

    #[test]
    fn validate_identifier_valid_starts_with_digit() {
        assert!(validate_identifier("2ping").is_ok());
    }

    #[test]
    fn validate_identifier_valid_min_length() {
        assert!(validate_identifier("bc").is_ok());
    }

    #[test]
    fn validate_identifier_valid_max_length() {
        let name = "a".repeat(64);
        assert!(validate_identifier(&name).is_ok());
    }

    #[test]
    fn validate_identifier_empty_fails() {
        let err = validate_identifier("").expect_err("should fail");
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_identifier_too_short_fails() {
        let err = validate_identifier("a").expect_err("should fail");
        assert!(err.contains("2 characters"));
    }

    #[test]
    fn validate_identifier_too_long_fails() {
        let name = "a".repeat(65);
        let err = validate_identifier(&name).expect_err("should fail");
        assert!(err.contains("64"));
    }

    #[test]
    fn validate_identifier_uppercase_fails() {
        assert!(validate_identifier("Nginx").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dash_fails() {
        assert!(validate_identifier("-foo").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dot_fails() {
        assert!(validate_identifier(".foo").is_err());
    }

    #[test]
    fn validate_identifier_path_traversal_fails() {
        assert!(validate_identifier("foo..bar").is_err());
    }

    #[test]
    fn validate_identifier_slash_fails() {
        assert!(validate_identifier("foo/bar").is_err());
    }

    #[test]
    fn validate_identifier_whitespace_fails() {
        assert!(validate_identifier("foo bar").is_err());
    }

    // ── parse_madison_output ────────────────────────────────────────────

    #[test]
    fn parse_madison_output_single_entry() {
        let output = "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0-2ubuntu7.3");
        assert!(entry.source.contains("archive.ubuntu.com"));
    }

    #[test]
    fn parse_madison_output_multiple_entries_returns_first() {
        let output = concat!(
            "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n",
            "   nginx | 1.18.0-6ubuntu14 | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages\n",
        );
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0-2ubuntu7.3");
    }

    #[test]
    fn parse_madison_output_malformed_line_skipped_gracefully() {
        let output = concat!("no pipe here\n", "   nginx | 1.24.0 | source\n",);
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0");
    }

    #[test]
    fn parse_madison_output_empty() {
        assert!(AptPlugin::parse_madison_output("").is_none());
    }

    #[test]
    fn parse_madison_output_security_source() {
        let output = "   openssl | 3.0.2-0ubuntu1.16 | http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "3.0.2-0ubuntu1.16");
        assert!(AptPlugin::is_security_source(&entry.source));
    }

    #[test]
    fn parse_madison_output_non_security_source() {
        let output =
            "   nginx | 1.24.0-2 | http://archive.ubuntu.com/ubuntu noble/main amd64 Packages\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert!(!AptPlugin::is_security_source(&entry.source));
    }

    #[test]
    fn is_security_source_detects_security_urls() {
        // Ubuntu security repo
        assert!(AptPlugin::is_security_source(
            "http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages"
        ));
        // Debian security repo
        assert!(AptPlugin::is_security_source(
            "http://security.debian.org/debian-security bookworm-security/main amd64 Packages"
        ));
        // Mixed case
        assert!(AptPlugin::is_security_source(
            "http://SECURITY.ubuntu.com/ubuntu noble-Security/main amd64 Packages"
        ));
    }

    #[test]
    fn is_security_source_rejects_non_security_urls() {
        assert!(!AptPlugin::is_security_source(
            "http://archive.ubuntu.com/ubuntu noble/main amd64 Packages"
        ));
        assert!(!AptPlugin::is_security_source(
            "http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages"
        ));
        assert!(!AptPlugin::is_security_source(""));
    }

    #[test]
    fn parse_madison_output_missing_source_field() {
        let output = "   nginx | 1.24.0\n";
        let entry = AptPlugin::parse_madison_output(output).unwrap();
        assert_eq!(entry.version, "1.24.0");
        assert!(entry.source.is_empty());
        assert!(!AptPlugin::is_security_source(&entry.source));
    }

    // ── parse_dpkg_output ───────────────────────────────────────────────

    #[test]
    fn parse_dpkg_output_normal() {
        let output = "nginx\t1.24.0-2ubuntu7.3\npython3\t3.11.0-5ubuntu2\n";
        let result = AptPlugin::parse_dpkg_output(output);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            ("nginx".to_string(), "1.24.0-2ubuntu7.3".to_string())
        );
        assert_eq!(
            result[1],
            ("python3".to_string(), "3.11.0-5ubuntu2".to_string())
        );
    }

    #[test]
    fn parse_dpkg_output_empty_version_skipped() {
        let output = "nginx\t\npython3\t3.11.0\n";
        let result = AptPlugin::parse_dpkg_output(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "python3");
        assert_eq!(result[0].1, "3.11.0");
    }

    #[test]
    fn parse_dpkg_output_empty_input() {
        let result = AptPlugin::parse_dpkg_output("");
        assert!(result.is_empty());
    }

    // ── required_sudo_commands ───────────────────────────────────────────

    #[tokio::test]
    async fn apt_plugin_required_sudo_commands() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let entries = plugin.required_sudo_commands();
        assert_eq!(entries.len(), 3);
        // All three entries are for apt-get.
        assert!(entries.iter().all(|e| e.command == "apt-get"));
        // All three require SETENV: (DEBIAN_FRONTEND=noninteractive).
        assert!(entries.iter().all(|e| e.needs_setenv));
        // Index refresh entry.
        assert_eq!(entries[0].args_suffix.as_deref(), Some("update *"));
        // Single-package install entry.
        assert_eq!(entries[1].args_suffix.as_deref(), Some("install *"));
        // Batch upgrade entry locks in the pref-file path.
        let batch_suffix = entries[2].args_suffix.as_deref().unwrap();
        assert!(
            batch_suffix.contains(APT_BATCH_PREF_FILE),
            "batch args_suffix must reference APT_BATCH_PREF_FILE"
        );
        assert!(batch_suffix.starts_with("-o Dir::Etc::Preferences="));
        assert!(batch_suffix.ends_with("upgrade *"));
    }

    // ── capabilities ────────────────────────────────────────────────────

    #[tokio::test]
    async fn apt_plugin_capabilities() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create plugin");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
        assert!(plugin.has_capability(PluginCapability::PostUpdateHook));
        assert_eq!(plugin.capabilities().len(), 4);
    }

    // ── empty identifier guards ──────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_releases_empty_identifier_fails() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let result = plugin.fetch_releases("").await;
        assert!(result.is_err());
    }

    // ── detect_host_compatibility ────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        let plugin = AptPlugin::new(
            AptConfig::default(),
            FixedExitCodeExecutor::with_exit_code(0),
        )
        .await
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        let plugin = AptPlugin::new(
            AptConfig::default(),
            FixedExitCodeExecutor::with_exit_code(1),
        )
        .await
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "apt-get not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    // ── post_update_hook ─────────────────────────────────────────────────

    #[tokio::test]
    async fn post_update_hook_emits_reboot_message_when_file_exists() {
        // exit_code 0 means `test -f /var/run/reboot-required` succeeded (file exists)
        let plugin = AptPlugin::new(
            AptConfig::default(),
            FixedExitCodeExecutor::with_exit_code(0),
        )
        .await
        .expect("create");
        let ctx = UpdateHookContext::new("nginx".to_string(), "1.24.0".to_string(), None);
        let (tx, mut rx) = mpsc::channel(10);
        plugin.post_update_hook(&ctx, &tx).await.expect("ok");
        drop(tx);

        let mut found_reboot_msg = false;
        while let Some(line) = rx.recv().await {
            if line.text.contains("Reboot required") {
                found_reboot_msg = true;
            }
        }
        assert!(found_reboot_msg, "expected reboot required message");
    }

    #[tokio::test]
    async fn post_update_hook_silent_when_file_missing() {
        // exit_code 1 means `test -f /var/run/reboot-required` failed (file absent)
        let plugin = AptPlugin::new(
            AptConfig::default(),
            FixedExitCodeExecutor::with_exit_code(1),
        )
        .await
        .expect("create");
        let ctx = UpdateHookContext::new("nginx".to_string(), "1.24.0".to_string(), None);
        let (tx, mut rx) = mpsc::channel(10);
        plugin.post_update_hook(&ctx, &tx).await.expect("ok");
        drop(tx);

        let mut found_any = false;
        while rx.recv().await.is_some() {
            found_any = true;
        }
        assert!(
            !found_any,
            "expected no output when reboot-required file is absent"
        );
    }

    // ── validate_version ────────────────────────────────────────────────

    #[test]
    fn validate_version_debian_standard() {
        assert!(validate_version("1.24.0-2ubuntu7.3").is_ok());
        assert!(validate_version("3.11.0-5ubuntu2").is_ok());
        assert!(validate_version("1:2.3.4-5").is_ok()); // epoch format
    }

    #[test]
    fn validate_version_with_tilde() {
        assert!(validate_version("1.0~beta1").is_ok());
    }

    #[test]
    fn validate_version_empty_fails() {
        let err = validate_version("").expect_err("should fail");
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_version_too_long_fails() {
        let long = "1".repeat(257);
        assert!(validate_version(&long).is_err());
    }

    #[test]
    fn validate_version_leading_dash_fails() {
        let err = validate_version("--allow-unauthenticated").expect_err("should fail");
        assert!(err.contains("flag"));
    }

    #[test]
    fn validate_version_space_fails() {
        assert!(validate_version("1.0 --allow-unauthenticated").is_err());
    }

    #[test]
    fn validate_version_equals_fails() {
        assert!(validate_version("1.0=extra").is_err());
    }

    #[test]
    fn validate_version_max_length_ok() {
        let v = "1".repeat(256);
        assert!(validate_version(&v).is_ok());
    }

    #[tokio::test]
    async fn post_update_hook_always_returns_ok() {
        // Even when the executor returns a non-zero exit code (file missing),
        // post_update_hook should return Ok(()) — it is non-fatal.
        let plugin = AptPlugin::new(
            AptConfig::default(),
            FixedExitCodeExecutor::with_exit_code(1),
        )
        .await
        .expect("create");
        let ctx = UpdateHookContext::new("pkg".to_string(), "1.0".to_string(), None);
        let (tx, _rx) = mpsc::channel(10);
        let result = plugin.post_update_hook(&ctx, &tx).await;
        assert!(result.is_ok());
    }

    // ── discover_software target emission ─────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets() {
        // Targets are always emitted regardless of filter.
        let executor = RoutedOutputExecutor::with_routes(vec![("dpkg-query", "nginx\t1.24.0\n")]);
        let plugin = AptPlugin::new(AptConfig::default(), executor)
            .await
            .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);

        let target = &discoveries[0].targets[0];
        assert_eq!(target.plugin_type, PluginType::PackageManagerApt);
        assert_eq!(target.plugin_config_name, "APT");
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_default_config_discovers_all_packages() {
        // Default config → effective filter All → all dpkg packages discovered.
        let executor = RoutedOutputExecutor::with_routes(vec![(
            "dpkg-query",
            "nginx\t1.24.0\npython3\t3.11.0\n",
        )]);
        let plugin = AptPlugin::new(AptConfig::default(), executor)
            .await
            .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 2, "all dpkg packages must be discovered");
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_all_filter() {
        // discovery_filter: All → targets always emitted.
        let executor = RoutedOutputExecutor::with_routes(vec![("dpkg-query", "nginx\t1.24.0\n")]);
        let plugin = AptPlugin::new(
            AptConfig {
                discovery_filter: AptDiscoveryFilter::All,
            },
            executor,
        )
        .await
        .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "explicit All filter must still emit targets"
        );
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_manual_filter() {
        // discovery_filter: Manual → apt-mark narrows packages; targets always emitted.
        let executor = RoutedOutputExecutor::with_routes(vec![
            ("dpkg-query", "nginx\t1.24.0\npython3\t3.11.0\n"),
            ("apt-mark", "nginx\n"), // only nginx is manually installed
        ]);
        let plugin = AptPlugin::new(
            AptConfig {
                discovery_filter: AptDiscoveryFilter::Manual,
            },
            executor,
        )
        .await
        .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].package_identifier, "nginx");
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "manual filter must still emit targets"
        );
    }

    // ── parse_madison_output_batch ───────────────────────────────────────

    #[test]
    fn parse_madison_output_batch_groups_by_package() {
        let output = concat!(
            "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages\n",
            "   curl  | 7.88.1-10+deb12u5 | http://deb.debian.org/debian bookworm/main amd64 Packages\n",
            "   nginx | 1.18.0-6ubuntu14  | http://archive.ubuntu.com/ubuntu focal/main amd64 Packages\n",
        );
        let result = AptPlugin::parse_madison_output_batch(output);
        assert_eq!(result.len(), 2);
        assert_eq!(result["nginx"].version, "1.24.0-2ubuntu7.3");
        assert_eq!(result["curl"].version, "7.88.1-10+deb12u5");
    }

    #[test]
    fn parse_madison_output_batch_only_first_entry_kept() {
        let output = concat!(
            "   nginx | 1.24.0 | source1\n",
            "   nginx | 1.18.0 | source2\n",
        );
        let result = AptPlugin::parse_madison_output_batch(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result["nginx"].version, "1.24.0");
    }

    #[test]
    fn parse_madison_output_batch_security_source_detected() {
        let output = "   openssl | 3.0.2-0ubuntu1.16 | http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages\n";
        let result = AptPlugin::parse_madison_output_batch(output);
        assert!(AptPlugin::is_security_source(&result["openssl"].source));
    }

    #[test]
    fn parse_madison_output_batch_empty_returns_empty() {
        let result = AptPlugin::parse_madison_output_batch("");
        assert!(result.is_empty());
    }

    // ── batch_detect_installed_version ───────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_version_found_packages() {
        let executor = RoutedOutputExecutor::with_routes(vec![(
            "dpkg-query",
            "nginx\t1.24.0-2ubuntu7.3\npython3\t3.11.0-5ubuntu2\n",
        )]);
        let plugin = AptPlugin::new(AptConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("python3".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 2);
        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(
            nginx.installed_version,
            Some(Version::new("1.24.0-2ubuntu7.3"))
        );
        assert!(nginx.error.is_none());

        let python3 = results
            .iter()
            .find(|r| r.package_identifier == "python3")
            .unwrap();
        assert_eq!(
            python3.installed_version,
            Some(Version::new("3.11.0-5ubuntu2"))
        );
        assert!(python3.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_package_not_in_output_is_not_installed() {
        // dpkg-query returns output for nginx only; curl is absent (not installed).
        let executor = RoutedOutputExecutor::with_routes(vec![("dpkg-query", "nginx\t1.24.0\n")]);
        let plugin = AptPlugin::new(AptConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("curl".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 2);
        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.installed_version.is_none());
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_detect_installed_version_empty_items_returns_empty() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin
            .batch_detect_installed_version(&[])
            .await
            .expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_invalid_identifier_fails() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create");
        let items = vec![BatchDetectItem::new("INVALID_UPPERCASE".to_string())];
        let result = plugin.batch_detect_installed_version(&items).await;
        assert!(result.is_err());
    }

    // ── batch_fetch_releases ─────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_releases_mixed_packages() {
        let executor = RoutedOutputExecutor::with_routes(vec![(
            "apt-cache",
            concat!(
                "   nginx | 1.24.0-2ubuntu7.3 | http://archive.ubuntu.com/ubuntu noble/main amd64 Packages\n",
                "   openssl | 3.0.2-0ubuntu1.16 | http://security.ubuntu.com/ubuntu noble-security/main amd64 Packages\n",
            ),
        )]);
        let plugin = AptPlugin::new(AptConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchFetchItem::new("nginx".to_string()),
            BatchFetchItem::new("openssl".to_string()),
            BatchFetchItem::new("curl".to_string()),
        ];
        let results = plugin.batch_fetch_releases(&items).await.expect("ok");

        assert_eq!(results.len(), 3);

        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(nginx.releases.len(), 1);
        assert_eq!(nginx.releases[0].tag, "1.24.0-2ubuntu7.3");
        assert!(nginx.releases[0].category.is_none());
        assert!(nginx.error.is_none());

        let openssl = results
            .iter()
            .find(|r| r.package_identifier == "openssl")
            .unwrap();
        assert_eq!(openssl.releases.len(), 1);
        assert_eq!(openssl.releases[0].category, Some(UpdateCategory::Security));
        assert!(openssl.error.is_none());

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.releases.is_empty(), "absent package has no releases");
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_fetch_releases_empty_items_returns_empty() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin.batch_fetch_releases(&[]).await.expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_fetch_releases_invalid_identifier_fails() {
        let plugin = AptPlugin::new(AptConfig::default(), test_executor())
            .await
            .expect("create");
        let items = vec![BatchFetchItem::new("INVALID".to_string())];
        let result = plugin.batch_fetch_releases(&items).await;
        assert!(result.is_err());
    }
}
