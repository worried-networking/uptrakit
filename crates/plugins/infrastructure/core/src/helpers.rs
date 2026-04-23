//! Shared helper functions for package-manager plugin implementations.
//!
//! These free functions extract the repetitive boilerplate found across all
//! package-manager plugins: identifier validation, single-package update
//! execution, batch update execution (versioned and names-only variants), and
//! package-index refresh.  Using them keeps each plugin's trait implementations
//! under ten lines while preserving exact output and error behaviour.
//!
//! # Usage
//!
//! Import from the crate root:
//!
//! ```rust,ignore
//! use uptrakit_plugin_infrastructure_core::{
//!     require_package_identifier, CommandUpdateParams, execute_command_update,
//!     BatchVersionedParams, execute_batch_versioned_command,
//!     BatchNamesParams, execute_batch_names_command,
//!     refresh_package_index_command,
//! };
//! ```

use rootcause::prelude::*;

use crate::batch_detect::{BatchDetectItem, BatchDetectResult};
use crate::batch_update::{BatchUpdateItem, BatchUpdateResult};
use crate::command::{CommandExecutor, CommandSpec, send_output};
use crate::error::{PluginError, Result};
use crate::execute_and_capture;
use crate::plugin_config::PluginConfigValidationError;
use crate::{OutputStreamType, UpdateOutputSender};

/// Function-pointer type for package-identifier and version validation predicates.
///
/// Accepts a value and returns `Ok(())` if valid, or a typed validation error
/// if invalid.
/// Used by [`BatchVersionedParams`], [`BatchNamesParams`], and
/// [`require_package_identifier`].
pub type ValidatorResult = std::result::Result<(), PluginConfigValidationError>;

/// Function-pointer type for package-identifier and version validators.
pub type ValidatorFn = fn(&str) -> ValidatorResult;

/// Convert a typed config/identifier validation error into the legacy raw
/// message text used by `PluginError::Configuration`.
///
/// This preserves existing user-facing messages while removing the
/// stringly-typed validator callback contract.
pub fn validation_error_message(error: PluginConfigValidationError) -> String {
    match error {
        PluginConfigValidationError::InvalidField { field, message } => {
            format!("{field}: {message}")
        }
        PluginConfigValidationError::InvalidIdentifier(message)
        | PluginConfigValidationError::Contract(message) => message,
    }
}

// ── require_package_identifier ────────────────────────────────────────────────

/// Validate a package identifier using `predicate` and map any failure to
/// [`PluginError::Configuration`].
///
/// This is the canonical one-liner replacement for the repetitive
/// `validate_identifier(...).map_err(|e| {
///     report!(PluginError::Configuration(validation_error_message(e)))
/// })`
/// pattern that appears in every package-manager plugin.
///
/// # Example
///
/// ```rust,ignore
/// fn require_package_identifier(&self, id: &str) -> Result<()> {
///     require_package_identifier(id, validate_identifier)
/// }
/// ```
pub fn require_package_identifier(
    value: &str,
    mut predicate: impl FnMut(&str) -> ValidatorResult,
) -> Result<()> {
    predicate(value).map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))
}

// ── execute_batch_detect_read_command ────────────────────────────────────────

/// Run a batch-detect read command and return either stdout or per-item errors.
///
/// This helper is intentionally small and purpose-built for batch detect paths
/// where command invocation errors should be downgraded to per-item
/// [`BatchDetectResult::error`] values instead of failing the whole operation.
///
/// Behavior:
/// - `Ok(stdout)` on successful `execute_quiet` invocation.
/// - `Err(Vec<BatchDetectResult>)` when command invocation fails, mapping the
///   same error string to every requested item. With the standard executor
///   this includes non-zero exits from `execute_quiet`.
pub async fn execute_batch_detect_read_command(
    executor: &dyn CommandExecutor,
    cmd: CommandSpec,
    items: &[BatchDetectItem],
    context: &str,
) -> std::result::Result<String, Vec<BatchDetectResult>> {
    match executor.execute_quiet(&cmd).await {
        Ok(output) => Ok(output.output),
        Err(e) => {
            let error_str = format!("{context} failed: {e}");
            Err(items
                .iter()
                .map(|item| {
                    BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                })
                .collect())
        }
    }
}

// ── execute_command_update ────────────────────────────────────────────────────

/// Parameters for [`execute_command_update`].
pub struct CommandUpdateParams<'a> {
    /// Command executor to drive.
    pub executor: &'a dyn CommandExecutor,
    /// Binary name (e.g., `"apt-get"`, `"cargo"`).
    pub binary: &'a str,
    /// Arguments to pass to the binary.
    pub args: Vec<String>,
    /// Whether to mark the command as privileged (sudo).
    pub privileged: bool,
    /// Optional modifier applied to the [`CommandSpec`] before execution.
    ///
    /// Use this for per-plugin spec customisation such as setting environment
    /// variables (e.g., `DEBIAN_FRONTEND=noninteractive` for APT).
    pub spec_modifier: Option<Box<dyn FnOnce(CommandSpec) -> CommandSpec + Send + 'a>>,
    /// Custom exit-code success predicate.
    ///
    /// When `None`, only exit code `0` is treated as success.
    /// Pass `Some(|_| true)` to skip exit-code checking (e.g., `mas`, Homebrew).
    pub exit_code_success: Option<fn(i32) -> bool>,
    /// Custom error factory called when exit code indicates failure.
    ///
    /// Receives the `display_args` string (binary + args joined by spaces) and
    /// the exit code.  When `None`, produces
    /// `PluginError::InstallFailed("{binary} failed with exit code {code}")`.
    pub exit_code_error: Option<fn(&str, i32) -> PluginError>,
}

/// Execute a single-package update command via `executor.execute()`.
///
/// Sends `"Running: {binary} {args}"` to `output_tx` and returns a `String`
/// prefixed with the same line followed by the command's captured stdout.
///
/// The caller is responsible for validation and any context `send_output`
/// calls (e.g. `"Updating {pkg} to {ver}"`) *before* invoking this helper.
///
/// # Errors
///
/// - Returns `PluginError::InstallFailed` (or the custom error from
///   [`CommandUpdateParams::exit_code_error`]) when the process exits with a
///   non-success code.
/// - Returns `PluginError::InstallFailed` when the process could not be
///   spawned.
pub async fn execute_command_update(
    params: CommandUpdateParams<'_>,
    output_tx: &UpdateOutputSender,
) -> Result<String> {
    let display_args: String = std::iter::once(params.binary)
        .chain(params.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    send_output(
        output_tx,
        &format!("Running: {display_args}"),
        OutputStreamType::Stdout,
    )
    .await;
    let mut output = format!("Running: {display_args}\n");

    let mut spec = CommandSpec::exec(params.binary, params.args);
    if params.privileged {
        spec = spec.privileged();
    }
    if let Some(modifier) = params.spec_modifier {
        spec = modifier(spec);
    }

    let cmd_output = params
        .executor
        .execute(&spec, output_tx)
        .await
        .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

    let succeeded = params
        .exit_code_success
        .map_or(cmd_output.exit_code == 0, |f| f(cmd_output.exit_code));

    if !succeeded {
        let error = params.exit_code_error.map_or_else(
            || {
                PluginError::InstallFailed(format!(
                    "{} failed with exit code {}",
                    params.binary, cmd_output.exit_code
                ))
            },
            |f| f(&display_args, cmd_output.exit_code),
        );
        bail!(error);
    }

    output.push_str(&cmd_output.output);
    Ok(output)
}

// ── execute_batch_versioned_command ───────────────────────────────────────────

/// Parameters for [`execute_batch_versioned_command`].
///
/// Use this for package managers that embed the target version in each
/// argument (e.g. `pkg@ver`, `pkg-ver`, `pkg=ver`).
pub struct BatchVersionedParams<'a> {
    /// Command executor to drive.
    pub executor: &'a dyn CommandExecutor,
    /// Binary name (e.g., `"npm"`, `"dnf"`).
    pub binary: &'a str,
    /// Arguments placed *before* the per-package items (e.g., `["install", "-g"]`).
    pub prefix_args: Vec<String>,
    /// Whether to mark the command as privileged (sudo).
    pub privileged: bool,
    /// Format a single `(package_identifier, version)` pair into a command argument.
    ///
    /// Examples: `|id, ver| format!("{id}@{ver}")`, `|id, ver| format!("{id}-{ver}")`.
    pub format_item: fn(&str, &str) -> String,
    /// Validate each package identifier before execution.
    pub validate_identifier: ValidatorFn,
    /// Validate each package version before execution.
    pub validate_version: ValidatorFn,
}

/// Execute a versioned batch update using a single command invocation.
///
/// Validates all items, builds the command from `prefix_args` + formatted items,
/// executes once, and maps the result to one [`BatchUpdateResult`] per item
/// (all sharing the same success flag and output string).
///
/// Returns an empty `Vec` immediately when `items` is empty.
pub async fn execute_batch_versioned_command(
    params: BatchVersionedParams<'_>,
    items: &[BatchUpdateItem],
    output_tx: &UpdateOutputSender,
) -> Result<Vec<BatchUpdateResult>> {
    if items.is_empty() {
        return Ok(vec![]);
    }

    for item in items {
        (params.validate_identifier)(&item.package_identifier)
            .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        (params.validate_version)(&item.to_version)
            .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
    }

    let mut args = params.prefix_args;
    for item in items {
        args.push((params.format_item)(
            &item.package_identifier,
            &item.to_version,
        ));
    }

    let display_args: String = std::iter::once(params.binary)
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

    let mut spec = CommandSpec::exec(params.binary, args);
    if params.privileged {
        spec = spec.privileged();
    }

    let cmd_output = params
        .executor
        .execute(&spec, output_tx)
        .await
        .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

    output.push_str(&cmd_output.output);
    let success = cmd_output.exit_code == 0;

    Ok(items
        .iter()
        .map(|item| {
            BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
        })
        .collect())
}

// ── execute_batch_names_command ───────────────────────────────────────────────

/// Parameters for [`execute_batch_names_command`].
///
/// Use this for package managers that pass only package names to the update
/// command (e.g., `pacman`, `snap refresh`, `pkg install`, `brew upgrade`).
pub struct BatchNamesParams<'a> {
    /// Command executor to drive.
    pub executor: &'a dyn CommandExecutor,
    /// Binary name (e.g., `"pacman"`, `"snap"`).
    pub binary: &'a str,
    /// Arguments placed *before* the package names (e.g., `["-S", "--noconfirm"]`).
    pub prefix_args: Vec<String>,
    /// Whether to mark the command as privileged (sudo).
    pub privileged: bool,
    /// Arguments placed *after* the package names (e.g., `["--channel=stable"]`).
    pub suffix_args: Vec<String>,
    /// Validate each package identifier before execution.
    pub validate_identifier: ValidatorFn,
    /// Optional version validator.  When `Some`, each item's `to_version` is
    /// validated (pre-flight check only; the version is not passed to the command).
    pub validate_version: Option<ValidatorFn>,
}

/// Execute a names-only batch update using a single command invocation.
///
/// Validates all items (and optionally their versions), builds the command from
/// `prefix_args` + package names + `suffix_args`, executes once, and maps the
/// result to one [`BatchUpdateResult`] per item.
///
/// Returns an empty `Vec` immediately when `items` is empty.
pub async fn execute_batch_names_command(
    params: BatchNamesParams<'_>,
    items: &[BatchUpdateItem],
    output_tx: &UpdateOutputSender,
) -> Result<Vec<BatchUpdateResult>> {
    if items.is_empty() {
        return Ok(vec![]);
    }

    for item in items {
        (params.validate_identifier)(&item.package_identifier)
            .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        if let Some(validate_version) = params.validate_version {
            validate_version(&item.to_version)
                .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        }
    }

    let mut args = params.prefix_args;
    for item in items {
        args.push(item.package_identifier.clone());
    }
    args.extend(params.suffix_args);

    let display_args: String = std::iter::once(params.binary)
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

    let mut spec = CommandSpec::exec(params.binary, args);
    if params.privileged {
        spec = spec.privileged();
    }

    let cmd_output = params
        .executor
        .execute(&spec, output_tx)
        .await
        .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

    output.push_str(&cmd_output.output);
    let success = cmd_output.exit_code == 0;

    Ok(items
        .iter()
        .map(|item| {
            BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
        })
        .collect())
}

// ── refresh_package_index_command ─────────────────────────────────────────────

/// Refresh a package index by running `cmd` quietly via [`execute_and_capture`].
///
/// Logs `"refreshing {label}"` before and `"{label} refreshed"` after.
/// The `label` should be a human-readable name for the index, e.g.,
/// `"DNF package index"` or `"Pacman package database"`.
///
/// # Errors
///
/// Propagates any error returned by [`execute_and_capture`]:
/// - Process-level failures → `PluginError::PluginInternal`
/// - Non-zero exit code → `PluginError::CommandFailed`
pub async fn refresh_package_index_command(
    executor: &dyn CommandExecutor,
    cmd: CommandSpec,
    label: &str,
) -> Result<()> {
    tracing::info!("refreshing {label}");
    execute_and_capture(executor, cmd, label).await?;
    tracing::info!("{label} refreshed");
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "testing"))]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::testing::FixedOutputExecutor;

    fn make_tx() -> UpdateOutputSender {
        let (tx, _rx) = mpsc::channel(16);
        tx
    }

    // ── require_package_identifier ────────────────────────────────────────

    #[test]
    fn require_package_identifier_valid_predicate() {
        let result = require_package_identifier("nginx", |v| {
            if v.is_empty() {
                Err(PluginConfigValidationError::Contract("empty".to_string()))
            } else {
                Ok(())
            }
        });
        assert!(result.is_ok());
    }

    #[test]
    fn require_package_identifier_failing_predicate() {
        let result = require_package_identifier("", |v| {
            if v.is_empty() {
                Err(PluginConfigValidationError::Contract(
                    "package_identifier must not be empty".to_string(),
                ))
            } else {
                Ok(())
            }
        });
        let err = result.unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::Configuration(_)
        ));
    }

    #[test]
    fn require_package_identifier_predicate_passes_value() {
        let mut seen = String::new();
        let result = require_package_identifier("my-pkg", |v| {
            seen = v.to_string();
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(seen, "my-pkg");
    }

    #[test]
    fn validation_error_message_invalid_identifier_returns_inner_text() {
        let err = PluginConfigValidationError::InvalidIdentifier("bad id".to_string());
        assert_eq!(validation_error_message(err), "bad id");
    }

    #[test]
    fn validation_error_message_contract_returns_inner_text() {
        let err = PluginConfigValidationError::Contract("bad version".to_string());
        assert_eq!(validation_error_message(err), "bad version");
    }

    // ── execute_command_update ────────────────────────────────────────────

    #[tokio::test]
    async fn execute_command_update_success() {
        let executor = FixedOutputExecutor::success("installed ok");
        let tx = make_tx();
        let result = execute_command_update(
            CommandUpdateParams {
                executor: executor.as_ref(),
                binary: "pkg",
                args: vec!["install".to_string(), "-y".to_string(), "nginx".to_string()],
                privileged: false,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: None,
            },
            &tx,
        )
        .await;
        let out = result.unwrap();
        assert!(out.starts_with("Running: pkg install -y nginx\n"));
        assert!(out.contains("installed ok"));
    }

    #[tokio::test]
    async fn execute_command_update_nonzero_exit_default_error() {
        let executor = FixedOutputExecutor::failure(1);
        let tx = make_tx();
        let err = execute_command_update(
            CommandUpdateParams {
                executor: executor.as_ref(),
                binary: "pkg",
                args: vec!["install".to_string(), "nginx".to_string()],
                privileged: false,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: None,
            },
            &tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::InstallFailed(_)
        ));
    }

    #[tokio::test]
    async fn execute_command_update_custom_exit_code_error() {
        let executor = FixedOutputExecutor::failure(42);
        let tx = make_tx();
        let err = execute_command_update(
            CommandUpdateParams {
                executor: executor.as_ref(),
                binary: "apk",
                args: vec!["add".to_string(), "nginx=1.0".to_string()],
                privileged: false,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: Some(|_, code| PluginError::CommandFailed(code)),
            },
            &tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::CommandFailed(42)
        ));
    }

    #[tokio::test]
    async fn execute_command_update_always_success_predicate() {
        // exit_code_success: Some(|_| true) skips exit-code check (mas/Homebrew pattern)
        let executor = FixedOutputExecutor::failure(1);
        let tx = make_tx();
        let result = execute_command_update(
            CommandUpdateParams {
                executor: executor.as_ref(),
                binary: "mas",
                args: vec!["upgrade".to_string(), "12345".to_string()],
                privileged: false,
                spec_modifier: None,
                exit_code_success: Some(|_| true),
                exit_code_error: None,
            },
            &tx,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn execute_command_update_spec_modifier_applied() {
        // Verify spec_modifier is called by checking the output path succeeds.
        // The modifier just returns the spec unchanged; we verify it compiles
        // and executes without panic.
        let executor = FixedOutputExecutor::success("ok");
        let tx = make_tx();
        let result = execute_command_update(
            CommandUpdateParams {
                executor: executor.as_ref(),
                binary: "apt-get",
                args: vec!["install".to_string(), "--yes".to_string()],
                privileged: false,
                spec_modifier: Some(Box::new(|spec| {
                    spec.with_env("DEBIAN_FRONTEND", "noninteractive")
                })),
                exit_code_success: None,
                exit_code_error: None,
            },
            &tx,
        )
        .await;
        assert!(result.is_ok());
    }

    // ── execute_batch_versioned_command ───────────────────────────────────

    #[tokio::test]
    async fn execute_batch_versioned_command_empty_items() {
        let executor = FixedOutputExecutor::success("should not run");
        let tx = make_tx();
        let result = execute_batch_versioned_command(
            BatchVersionedParams {
                executor: executor.as_ref(),
                binary: "npm",
                prefix_args: vec!["install".to_string(), "-g".to_string()],
                privileged: false,
                format_item: |id, ver| format!("{id}@{ver}"),
                validate_identifier: |_| Ok(()),
                validate_version: |_| Ok(()),
            },
            &[],
            &tx,
        )
        .await
        .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn execute_batch_versioned_command_success_all_true() {
        let executor = FixedOutputExecutor::success("batch done");
        let tx = make_tx();
        let items = vec![
            BatchUpdateItem::new("pkg1".to_string(), "1.0".to_string(), None),
            BatchUpdateItem::new("pkg2".to_string(), "2.0".to_string(), None),
        ];
        let results = execute_batch_versioned_command(
            BatchVersionedParams {
                executor: executor.as_ref(),
                binary: "npm",
                prefix_args: vec!["install".to_string(), "-g".to_string()],
                privileged: false,
                format_item: |id, ver| format!("{id}@{ver}"),
                validate_identifier: |_| Ok(()),
                validate_version: |_| Ok(()),
            },
            &items,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.success));
        assert!(
            results[0]
                .output
                .contains("Running: npm install -g pkg1@1.0 pkg2@2.0\n")
        );
    }

    #[tokio::test]
    async fn execute_batch_versioned_command_failure_all_false() {
        let executor = FixedOutputExecutor::failure(1);
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "pkg1".to_string(),
            "1.0".to_string(),
            None,
        )];
        let results = execute_batch_versioned_command(
            BatchVersionedParams {
                executor: executor.as_ref(),
                binary: "dnf",
                prefix_args: vec!["install".to_string(), "-y".to_string()],
                privileged: false,
                format_item: |id, ver| format!("{id}-{ver}"),
                validate_identifier: |_| Ok(()),
                validate_version: |_| Ok(()),
            },
            &items,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
    }

    #[tokio::test]
    async fn execute_batch_versioned_command_invalid_identifier_fails() {
        let executor = FixedOutputExecutor::success("ok");
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "INVALID".to_string(),
            "1.0".to_string(),
            None,
        )];
        let err = execute_batch_versioned_command(
            BatchVersionedParams {
                executor: executor.as_ref(),
                binary: "apk",
                prefix_args: vec!["add".to_string()],
                privileged: false,
                format_item: |id, ver| format!("{id}={ver}"),
                validate_identifier: |v| {
                    if v.chars().any(|c| c.is_ascii_uppercase()) {
                        Err(PluginConfigValidationError::Contract(
                            "uppercase not allowed".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
                },
                validate_version: |_| Ok(()),
            },
            &items,
            &tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::Configuration(_)
        ));
    }

    #[tokio::test]
    async fn execute_batch_versioned_command_invalid_version_fails() {
        let executor = FixedOutputExecutor::success("ok");
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "pkg1".to_string(),
            "".to_string(),
            None,
        )];
        let err = execute_batch_versioned_command(
            BatchVersionedParams {
                executor: executor.as_ref(),
                binary: "apk",
                prefix_args: vec!["add".to_string()],
                privileged: false,
                format_item: |id, ver| format!("{id}={ver}"),
                validate_identifier: |_| Ok(()),
                validate_version: |v| {
                    if v.is_empty() {
                        Err(PluginConfigValidationError::Contract(
                            "version must not be empty".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
                },
            },
            &items,
            &tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::Configuration(_)
        ));
    }

    #[tokio::test]
    async fn execute_batch_versioned_command_format_item_output() {
        // Verify `format_item` is applied correctly for `=` separator (APK style).
        let executor = FixedOutputExecutor::success("done");
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "busybox".to_string(),
            "1.36.1-r5".to_string(),
            None,
        )];
        let results = execute_batch_versioned_command(
            BatchVersionedParams {
                executor: executor.as_ref(),
                binary: "apk",
                prefix_args: vec!["add".to_string()],
                privileged: false,
                format_item: |id, ver| format!("{id}={ver}"),
                validate_identifier: |_| Ok(()),
                validate_version: |_| Ok(()),
            },
            &items,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].output.contains("apk add busybox=1.36.1-r5"));
    }

    // ── execute_batch_names_command ───────────────────────────────────────

    #[tokio::test]
    async fn execute_batch_names_command_empty_items() {
        let executor = FixedOutputExecutor::success("should not run");
        let tx = make_tx();
        let result = execute_batch_names_command(
            BatchNamesParams {
                executor: executor.as_ref(),
                binary: "pacman",
                prefix_args: vec!["-S".to_string(), "--noconfirm".to_string()],
                privileged: false,
                suffix_args: vec![],
                validate_identifier: |_| Ok(()),
                validate_version: None,
            },
            &[],
            &tx,
        )
        .await
        .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn execute_batch_names_command_success() {
        let executor = FixedOutputExecutor::success("packages installed");
        let tx = make_tx();
        let items = vec![
            BatchUpdateItem::new("nginx".to_string(), "1.0".to_string(), None),
            BatchUpdateItem::new("curl".to_string(), "7.0".to_string(), None),
        ];
        let results = execute_batch_names_command(
            BatchNamesParams {
                executor: executor.as_ref(),
                binary: "pacman",
                prefix_args: vec!["-S".to_string(), "--noconfirm".to_string()],
                privileged: true,
                suffix_args: vec![],
                validate_identifier: |_| Ok(()),
                validate_version: None,
            },
            &items,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.success));
    }

    #[tokio::test]
    async fn execute_batch_names_command_suffix_args_ordering() {
        // suffix_args must appear AFTER package names.
        let executor = FixedOutputExecutor::success("ok");
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "firefox".to_string(),
            "130.0".to_string(),
            None,
        )];
        let results = execute_batch_names_command(
            BatchNamesParams {
                executor: executor.as_ref(),
                binary: "snap",
                prefix_args: vec!["refresh".to_string()],
                privileged: true,
                suffix_args: vec!["--channel=stable".to_string()],
                validate_identifier: |_| Ok(()),
                validate_version: None,
            },
            &items,
            &tx,
        )
        .await
        .unwrap();
        // Output must contain "snap refresh firefox --channel=stable"
        assert!(
            results[0]
                .output
                .contains("snap refresh firefox --channel=stable")
        );
    }

    #[tokio::test]
    async fn execute_batch_names_command_validate_version_some() {
        // When validate_version is Some, invalid versions must be rejected.
        let executor = FixedOutputExecutor::success("ok");
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "pkg".to_string(),
            "INVALID VERSION!".to_string(),
            None,
        )];
        let err = execute_batch_names_command(
            BatchNamesParams {
                executor: executor.as_ref(),
                binary: "pacman",
                prefix_args: vec!["-S".to_string(), "--noconfirm".to_string()],
                privileged: false,
                suffix_args: vec![],
                validate_identifier: |_| Ok(()),
                validate_version: Some(|v| {
                    if v.contains('!') {
                        Err(PluginConfigValidationError::Contract(
                            "invalid version".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
                }),
            },
            &items,
            &tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::Configuration(_)
        ));
    }

    #[tokio::test]
    async fn execute_batch_names_command_validate_version_none_skips_check() {
        // When validate_version is None, any version string is accepted.
        let executor = FixedOutputExecutor::success("ok");
        let tx = make_tx();
        let items = vec![BatchUpdateItem::new(
            "pkg".to_string(),
            "ANYTHING-GOES".to_string(),
            None,
        )];
        let result = execute_batch_names_command(
            BatchNamesParams {
                executor: executor.as_ref(),
                binary: "pkg",
                prefix_args: vec!["install".to_string(), "-y".to_string()],
                privileged: false,
                suffix_args: vec![],
                validate_identifier: |_| Ok(()),
                validate_version: None,
            },
            &items,
            &tx,
        )
        .await;
        assert!(result.is_ok());
    }

    // ── execute_batch_detect_read_command ─────────────────────────────────

    #[tokio::test]
    async fn execute_batch_detect_read_command_command_error_maps_all_items() {
        let executor = FixedOutputExecutor::failure(1);
        let items = vec![
            crate::BatchDetectItem::new("nginx".to_string()),
            crate::BatchDetectItem::new("curl".to_string()),
        ];

        let results = execute_batch_detect_read_command(
            executor.as_ref(),
            CommandSpec::exec("dpkg-query", ["--show".to_string()]),
            &items,
            "dpkg-query",
        )
        .await
        .expect_err("expected per-item errors on command failure");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].package_identifier, "nginx");
        assert_eq!(results[1].package_identifier, "curl");
        assert!(results[0].installed_version.is_none());
        assert!(results[1].installed_version.is_none());

        let error_0 = results[0].error.as_deref().expect("error");
        let error_1 = results[1].error.as_deref().expect("error");
        assert_eq!(error_0, error_1);
        assert!(error_0.contains("dpkg-query failed:"));
        assert!(error_0.contains("command exited with code 1"));
    }

    // ── refresh_package_index_command ─────────────────────────────────────

    #[tokio::test]
    async fn refresh_package_index_command_success() {
        let executor = FixedOutputExecutor::success("");
        let result = refresh_package_index_command(
            executor.as_ref(),
            CommandSpec::exec("apk", ["update".to_string()]),
            "APK package index",
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn refresh_package_index_command_nonzero_exit_propagates_plugin_internal() {
        // FixedOutputExecutor::failure → execute_quiet returns Err(CommandFailed)
        // execute_and_capture maps process errors to PluginError::PluginInternal
        let executor = FixedOutputExecutor::failure(1);
        let err = refresh_package_index_command(
            executor.as_ref(),
            CommandSpec::exec("apk", ["update".to_string()]),
            "APK package index",
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            PluginError::PluginInternal(_)
        ));
    }
}
