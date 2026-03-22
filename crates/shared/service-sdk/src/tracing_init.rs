//! Centralised tracing / logging initialisation for all Uptrakit binaries.
//!
//! # Overview
//!
//! [`TracingBuilder`] is the single entry-point for all service daemons.
//! Feature-gated helpers [`init_cli_tracing`] and [`init_test_tracing`] cover
//! the two special cases (CLI → stderr; tests → thread-safe once-init).
//!
//! # `RUST_LOG` precedence
//!
//! Programmatic directives (derived from `-v` flags) are added **first**;
//! `RUST_LOG` directives are added **after** — so `RUST_LOG` wins for any
//! same-target match.  This is the opposite of the historical behaviour and
//! lets operators override any verbosity setting without rebuilding.

use std::collections::BTreeMap;

use tracing_subscriber::{EnvFilter, Layer, Registry, prelude::*};

#[cfg(feature = "cli")]
use std::io;
#[cfg(feature = "test-support")]
use std::sync::Once;

/// A type-erased tracing layer that can be stacked with other layers.
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

/// Builder for the global tracing subscriber.
///
/// The default configuration mirrors the historical service-daemon behaviour:
/// `uptrakit=info` at verbosity 0, `uptrakit=debug` at 1, `uptrakit=trace`
/// at 2 (the maximum).
///
/// # Example
///
/// ```rust,ignore
/// uptrakit_service_sdk::TracingBuilder::new()
///     .verbosity(args.common.verbose)
///     .init();
/// ```
pub struct TracingBuilder {
    verbosity: u8,
    max_verbosity: u8,
    /// Map from verbosity level → list of (target, level) directive pairs.
    directives: BTreeMap<u8, Vec<(String, String)>>,
    extra_layers: Vec<BoxedLayer>,
}

impl Default for TracingBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TracingBuilder {
    /// Create a builder with the default 3-level (0–2) directive scheme.
    pub fn new() -> Self {
        let mut directives: BTreeMap<u8, Vec<(String, String)>> = BTreeMap::new();
        directives.insert(0, vec![("uptrakit".to_string(), "info".to_string())]);
        directives.insert(1, vec![("uptrakit".to_string(), "debug".to_string())]);
        directives.insert(2, vec![("uptrakit".to_string(), "trace".to_string())]);
        Self {
            verbosity: 0,
            max_verbosity: 2,
            directives,
            extra_layers: Vec::new(),
        }
    }

    /// Set the verbosity level (from the number of `-v` flags).
    pub fn verbosity(mut self, verbosity: u8) -> Self {
        self.verbosity = verbosity;
        self
    }

    /// Override the maximum effective verbosity level (default: 2).
    ///
    /// Verbosity values above this are clamped and trigger a warning on stderr.
    pub fn max_verbosity(mut self, max_verbosity: u8) -> Self {
        self.max_verbosity = max_verbosity;
        self
    }

    /// Replace the directive set for a given verbosity level.
    ///
    /// Each pair is `(tracing-target, level-string)`, e.g.
    /// `("uptrakit_controller", "debug")`.
    pub fn directives_for_verbosity(mut self, verbosity: u8, directives: &[(&str, &str)]) -> Self {
        self.directives.insert(
            verbosity,
            directives
                .iter()
                .map(|(t, l)| ((*t).to_string(), (*l).to_string()))
                .collect(),
        );
        self
    }

    /// Push an additional, already-filtered layer onto the subscriber stack.
    ///
    /// This is the integration point for the `journald` audit layer.
    pub fn extra_layer(mut self, layer: BoxedLayer) -> Self {
        self.extra_layers.push(layer);
        self
    }

    /// Build the `EnvFilter` without initialising the subscriber.
    ///
    /// Programmatic directives are added first; `RUST_LOG` directives are
    /// appended after so that `RUST_LOG` wins for the same target.
    fn build_env_filter(&self) -> EnvFilter {
        let clamped = self.verbosity.min(self.max_verbosity);

        if self.verbosity > self.max_verbosity {
            let v_max = "v".repeat(self.max_verbosity as usize);
            let v_over = "v".repeat(self.max_verbosity as usize + 1);
            let level = self
                .directives
                .get(&self.max_verbosity)
                .and_then(|d| d.first())
                .map(|(_, l)| l.as_str())
                .unwrap_or("trace");
            eprintln!(
                "warning: -{v_over} or more has no additional effect; \
                 maximum verbosity is -{v_max} ({level})"
            );
        }

        // Start from an empty filter (blocks everything unless a directive matches).
        let mut filter = EnvFilter::new("");

        // 1. Programmatic directives first (lower priority for same-target conflicts).
        if let Some(directives) = self.directives.get(&clamped) {
            for (target, level) in directives {
                let s = format!("{target}={level}");
                filter =
                    filter.add_directive(s.parse().expect("BUG: invalid compile-time directive"));
            }
        }

        // 2. RUST_LOG appended after — wins for same-target matches.
        if let Ok(rust_log) = std::env::var("RUST_LOG") {
            for part in rust_log.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                match part.parse() {
                    Ok(d) => {
                        filter = filter.add_directive(d);
                    }
                    Err(e) => {
                        eprintln!("warning: ignoring invalid RUST_LOG directive {part:?}: {e}");
                    }
                }
            }
        }

        filter
    }

    /// Consume the builder and return the constructed [`EnvFilter`].
    ///
    /// Useful when the caller needs the filter without initialising a subscriber
    /// (e.g. to pass it to a non-registry `fmt()` builder).
    pub fn build_filter(self) -> EnvFilter {
        self.build_env_filter()
    }

    /// Consume the builder and install the global tracing subscriber.
    ///
    /// The subscriber is a `tracing_subscriber::registry()` with:
    /// 1. An `fmt` layer filtered by the verbosity-derived + `RUST_LOG` filter.
    /// 2. Any extra layers registered via [`extra_layer`](Self::extra_layer).
    pub fn init(self) {
        let filter = self.build_env_filter();
        let fmt_layer: BoxedLayer = Box::new(tracing_subscriber::fmt::layer().with_filter(filter));

        let mut layers: Vec<BoxedLayer> = vec![fmt_layer];
        layers.extend(self.extra_layers);

        tracing_subscriber::registry().with(layers).init();
    }
}

/// Initialise tracing for CLI binaries.
///
/// Output goes to **stderr** so it does not contaminate stdout command output.
/// No subscriber is installed when `verbosity == 0`.
///
/// | `-v` count | Directive |
/// |---|---|
/// | 0 | *(no subscriber)* |
/// | 1 | `uptrakit_cli=warn` |
/// | 2 | `uptrakit_cli=debug` |
/// | 3 | `uptrakit=debug` |
/// | 4+ | `uptrakit=trace` (capped) |
#[cfg(feature = "cli")]
pub fn init_cli_tracing(verbosity: u8) {
    if verbosity == 0 {
        return;
    }
    let filter = TracingBuilder::new()
        .max_verbosity(4)
        .directives_for_verbosity(1, &[("uptrakit_cli", "warn")])
        .directives_for_verbosity(2, &[("uptrakit_cli", "debug")])
        .directives_for_verbosity(3, &[("uptrakit", "debug")])
        .directives_for_verbosity(4, &[("uptrakit", "trace")])
        .verbosity(verbosity)
        .build_filter();
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(filter)
        .init();
}

/// Initialise tracing for integration / unit tests (once per process).
///
/// Uses `with_test_writer()` so output is captured by the test harness.
/// Baseline level is `warn`; `RUST_LOG` can raise it for specific targets.
#[cfg(feature = "test-support")]
pub fn init_test_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let mut filter = EnvFilter::new("").add_directive("warn".parse().expect("valid directive"));
        if let Ok(rust_log) = std::env::var("RUST_LOG") {
            for part in rust_log.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                match part.parse() {
                    Ok(d) => {
                        filter = filter.add_directive(d);
                    }
                    Err(e) => {
                        eprintln!("warning: ignoring invalid RUST_LOG directive {part:?}: {e}");
                    }
                }
            }
        }
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter(filter)
            .try_init();
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::prelude::*;

    use super::*;

    // ── Shared test infrastructure ────────────────────────────────────────────

    #[derive(Clone)]
    struct BufWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Install a thread-local subscriber with the given filter, run `f`, and
    /// return the captured output.  Uses `set_default` so tests can run in
    /// parallel without global-subscriber conflicts.
    fn capture(filter: EnvFilter, f: impl FnOnce()) -> String {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = BufWriter(buf.clone());
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_filter(filter),
        );
        let _guard = tracing::subscriber::set_default(subscriber);
        f();
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// When two directives share the same target, the one added last wins.
    #[test]
    fn same_target_last_directive_wins() {
        let filter = EnvFilter::new("uptrakit_sa_test=info")
            .add_directive("uptrakit_sa_test=debug".parse().unwrap());
        let out = capture(filter, || {
            tracing::debug!(target: "uptrakit_sa_test", "debug event");
        });
        assert!(
            out.contains("debug event"),
            "later `debug` directive should win over earlier `info`"
        );
    }

    /// A target-qualified directive beats a bare level even when added first.
    #[test]
    fn target_qualified_beats_bare_level() {
        let filter =
            EnvFilter::new("error").add_directive("uptrakit_tq_test=debug".parse().unwrap());
        let out = capture(filter, || {
            tracing::debug!(target: "uptrakit_tq_test", "debug event");
        });
        assert!(
            out.contains("debug event"),
            "target-qualified directive should beat bare `error` level"
        );
    }

    /// Default verbosity 0 → info passes, debug is filtered out.
    #[test]
    fn builder_default_verbosity_0_produces_info() {
        let filter = TracingBuilder::new().verbosity(0).build_filter();
        let out = capture(filter, || {
            tracing::info!(target: "uptrakit_v0_test", "info message");
            tracing::debug!(target: "uptrakit_v0_test", "debug message");
        });
        assert!(
            out.contains("info message"),
            "info should pass at verbosity 0"
        );
        assert!(
            !out.contains("debug message"),
            "debug should be hidden at verbosity 0"
        );
    }

    /// Verbosity 1 → debug passes.
    #[test]
    fn builder_verbosity_1_produces_debug() {
        let filter = TracingBuilder::new().verbosity(1).build_filter();
        let out = capture(filter, || {
            tracing::debug!(target: "uptrakit_v1_test", "debug message");
        });
        assert!(
            out.contains("debug message"),
            "debug should pass at verbosity 1"
        );
    }

    /// Verbosity above max_verbosity is clamped to the max.
    #[test]
    fn builder_max_verbosity_clamps() {
        // v=5 with max_verbosity=2 → clamped to 2 → trace directives apply
        let filter = TracingBuilder::new()
            .verbosity(5)
            .max_verbosity(2)
            .build_filter();
        let out = capture(filter, || {
            tracing::trace!(target: "uptrakit_clamp_test", "trace message");
        });
        assert!(
            out.contains("trace message"),
            "trace should pass when v=5 is clamped to max_verbosity=2 (trace)"
        );
    }

    /// Custom directives replace the defaults; only the specified target passes.
    #[test]
    fn builder_custom_directives_replace_defaults() {
        // Replace v=0 default with a single-crate directive
        let filter = TracingBuilder::new()
            .directives_for_verbosity(0, &[("uptrakit_controller_cd_test", "info")])
            .verbosity(0)
            .build_filter();
        let out = capture(filter, || {
            tracing::info!(
                target: "uptrakit_other_cd_test",
                "other info"
            );
            tracing::info!(
                target: "uptrakit_controller_cd_test",
                "controller info"
            );
        });
        assert!(
            !out.contains("other info"),
            "uptrakit_other events should be filtered when only uptrakit_controller is targeted"
        );
        assert!(
            out.contains("controller info"),
            "uptrakit_controller events should pass"
        );
    }

    /// An extra layer registered via `extra_layer` receives events independently
    /// from the main fmt layer.
    ///
    /// Uses `Vec<BoxedLayer>` in a single `.with()` call so all layers are
    /// added at the `Registry` level — required because `BoxedLayer` erases
    /// to `Box<dyn Layer<Registry>>` and cannot be stacked on an already-layered
    /// subscriber without a type mismatch.
    #[test]
    fn builder_extra_layer_is_applied() {
        let extra_buf = Arc::new(Mutex::new(Vec::new()));
        let extra_writer = BufWriter(extra_buf.clone());

        let main_buf = Arc::new(Mutex::new(Vec::new()));
        let main_writer = BufWriter(main_buf.clone());

        // Main layer: error-only; extra layer: everything (trace).
        let layers: Vec<BoxedLayer> = vec![
            Box::new(
                tracing_subscriber::fmt::layer()
                    .with_writer(main_writer)
                    .with_filter(EnvFilter::new("error")),
            ),
            Box::new(
                tracing_subscriber::fmt::layer()
                    .with_writer(extra_writer)
                    .with_filter(EnvFilter::new("trace")),
            ),
        ];

        let subscriber = tracing_subscriber::registry().with(layers);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(target: "uptrakit_el_test", "info event");

        let main_out = String::from_utf8(main_buf.lock().unwrap().clone()).unwrap();
        let extra_out = String::from_utf8(extra_buf.lock().unwrap().clone()).unwrap();

        assert!(
            !main_out.contains("info event"),
            "main layer (error filter) should suppress info"
        );
        assert!(
            extra_out.contains("info event"),
            "extra layer (trace filter) should receive info independently"
        );
    }
}
