//! Log-output-format resolution: `UPTRAKIT_LOG_FORMAT` parsing and
//! journald stdout detection (`JOURNAL_STREAM` + fstat identity match).

use std::str::FromStr;

/// Requested log output format, from `UPTRAKIT_LOG_FORMAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogFormat {
    Auto,
    Text,
    Journald,
}

/// Error for an unknown `UPTRAKIT_LOG_FORMAT` value.
#[derive(Debug, thiserror::Error)]
#[error("unknown log format {value:?}; expected auto, text, or journald")]
pub(crate) struct ParseLogFormatError {
    value: String,
}

impl FromStr for LogFormat {
    type Err = ParseLogFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "text" => Ok(Self::Text),
            "journald" => Ok(Self::Journald),
            _ => Err(ParseLogFormatError {
                value: s.to_string(),
            }),
        }
    }
}

/// The `dev:inode` pair systemd publishes in `JOURNAL_STREAM`
/// (systemd.exec(5)) when stdout/stderr is connected to the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JournalStream {
    dev: u64,
    ino: u64,
}

/// Error for a malformed `JOURNAL_STREAM` value.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ParseJournalStreamError {
    #[error("invalid JOURNAL_STREAM value {0:?}; expected <dev>:<inode>")]
    MissingColon(String),
    #[error("invalid JOURNAL_STREAM number in {value:?}")]
    InvalidNumber {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
}

impl FromStr for JournalStream {
    type Err = ParseJournalStreamError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (dev, ino) = s
            .split_once(':')
            .ok_or_else(|| ParseJournalStreamError::MissingColon(s.to_string()))?;
        let number = |source| ParseJournalStreamError::InvalidNumber {
            value: s.to_string(),
            source,
        };
        Ok(Self {
            dev: dev.parse().map_err(number)?,
            ino: ino.parse().map_err(number)?,
        })
    }
}

/// Pure core of the stdout-is-journal check.
///
/// `stdout_stat` is the `(st_dev, st_ino)` of fd 1, widened to `i128`
/// because the platform types differ (`u64` on Linux, `i32`/`u64` on
/// macOS) and `i128` admits them all losslessly without casts.
pub(crate) fn journal_stream_matches(
    env_value: Option<&str>,
    stdout_stat: Option<(i128, i128)>,
) -> bool {
    let (Some(env_value), Some((dev, ino))) = (env_value, stdout_stat) else {
        return false;
    };
    env_value
        .parse::<JournalStream>()
        .is_ok_and(|js| i128::from(js.dev) == dev && i128::from(js.ino) == ino)
}

/// Whether the process's stdout IS the systemd journal stream.
///
/// Deliberately narrower than "journald is reachable" (which
/// `tracing_journald::layer()` probes itself): journald mode must not
/// activate when an operator redirects stdout elsewhere on a systemd
/// host.
pub(crate) fn stdout_is_journal() -> bool {
    #[cfg(unix)]
    {
        let env = std::env::var("JOURNAL_STREAM").ok();
        let stat = rustix::fs::fstat(std::io::stdout())
            .ok()
            .map(|s| (i128::from(s.st_dev), i128::from(s.st_ino)));
        journal_stream_matches(env.as_deref(), stat)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Resolve the requested format from a `UPTRAKIT_LOG_FORMAT` value.
///
/// An unparseable value warns on stderr and degrades to [`LogFormat::Auto`] rather than
/// aborting startup — a typo in a unit file must not take the service down.
pub(crate) fn resolve_format(env_value: Option<&str>) -> LogFormat {
    match env_value {
        Some(value) => value.parse().unwrap_or_else(|e| {
            eprintln!("warning: ignoring UPTRAKIT_LOG_FORMAT: {e}");
            LogFormat::Auto
        }),
        None => LogFormat::Auto,
    }
}

/// Resolve the requested format from `UPTRAKIT_LOG_FORMAT` (default: auto).
pub(crate) fn env_log_format() -> LogFormat {
    resolve_format(std::env::var("UPTRAKIT_LOG_FORMAT").ok().as_deref())
}

/// Whether the journald layer should be installed for `format`.
pub(crate) fn use_journald(format: LogFormat, stdout_is_journal: bool) -> bool {
    match format {
        LogFormat::Auto => stdout_is_journal,
        LogFormat::Text => false,
        LogFormat::Journald => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_parses_known_values() {
        assert_eq!("auto".parse::<LogFormat>().unwrap(), LogFormat::Auto);
        assert_eq!("text".parse::<LogFormat>().unwrap(), LogFormat::Text);
        assert_eq!(
            "journald".parse::<LogFormat>().unwrap(),
            LogFormat::Journald
        );
        // Operator-friendly: case and surrounding whitespace tolerated.
        assert_eq!(
            " Journald ".parse::<LogFormat>().unwrap(),
            LogFormat::Journald
        );
    }

    #[test]
    fn log_format_rejects_unknown_value() {
        let err = "systemd".parse::<LogFormat>().unwrap_err();
        assert!(err.to_string().contains("systemd"));
    }

    #[test]
    fn journal_stream_parses_dev_inode() {
        assert_eq!(
            "10:352799757".parse::<JournalStream>().unwrap(),
            JournalStream {
                dev: 10,
                ino: 352_799_757
            }
        );
    }

    #[test]
    fn journal_stream_rejects_missing_colon() {
        "".parse::<JournalStream>().unwrap_err();
        "12".parse::<JournalStream>().unwrap_err();
    }

    #[test]
    fn journal_stream_trims_surrounding_whitespace() {
        assert_eq!(
            " 10:352799757 ".parse::<JournalStream>().unwrap(),
            "10:352799757".parse::<JournalStream>().unwrap()
        );
    }

    #[test]
    fn journal_stream_rejects_non_numeric() {
        "a:1".parse::<JournalStream>().unwrap_err();
        "1:b".parse::<JournalStream>().unwrap_err();
        "1:2:3".parse::<JournalStream>().unwrap_err();
        "-1:2".parse::<JournalStream>().unwrap_err();
    }

    #[test]
    fn journal_stream_matches_requires_env_and_stat() {
        assert!(!journal_stream_matches(None, Some((10, 7))));
        assert!(!journal_stream_matches(Some("10:7"), None));
        assert!(!journal_stream_matches(None, None));
    }

    #[test]
    fn journal_stream_matches_on_equality_only() {
        assert!(journal_stream_matches(Some("10:7"), Some((10, 7))));
        assert!(!journal_stream_matches(Some("10:7"), Some((11, 7))));
        assert!(!journal_stream_matches(Some("10:7"), Some((10, 8))));
        assert!(!journal_stream_matches(Some("garbage"), Some((10, 7))));
    }

    #[test]
    fn use_journald_matrix() {
        assert!(!use_journald(LogFormat::Auto, false));
        assert!(use_journald(LogFormat::Auto, true));
        assert!(!use_journald(LogFormat::Text, true));
        assert!(use_journald(LogFormat::Journald, false));
    }

    #[test]
    fn resolve_format_matrix() {
        assert_eq!(resolve_format(None), LogFormat::Auto);
        assert_eq!(resolve_format(Some("journald")), LogFormat::Journald);
        assert_eq!(resolve_format(Some("  TEXT  ")), LogFormat::Text);
        assert_eq!(resolve_format(Some("syslog")), LogFormat::Auto);
    }
}
