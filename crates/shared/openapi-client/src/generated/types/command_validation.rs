//! Dangerous command pattern detection for plugin configuration validation.
//!
//! Controller-side detection of potentially dangerous patterns in shell
//! commands stored in plugin configs.
//!
//! This module only detects patterns and returns matches. Policy decisions
//! (reject vs allow) and semantic audit emission are handled by web API route
//! handlers.
/// Simple substring patterns with human-readable descriptions.
///
/// Matching is case-insensitive on the command string.
const DANGEROUS_PATTERNS: &[(&str, &str)] = &[
    ("dd if=", "raw disk write"),
    ("mkfs.", "filesystem format"),
    (":(){ :|:& };:", "fork bomb"),
    ("/dev/tcp/", "bash network socket"),
    ("/dev/udp/", "bash network socket"),
    ("> /dev/sda", "raw disk overwrite"),
    ("mv /* /dev/null", "destructive move to null"),
];
/// Patterns that need end-of-string or wildcard boundary matching.
///
/// The pattern must appear at the end of the command or be followed by
/// whitespace or `*`.
const BOUNDARY_PATTERNS: &[(&str, &str)] = &[
    ("rm -rf /", "recursive delete from root"),
    ("chmod 777 /", "recursive world-writable root"),
];
/// Shell interpreters that, when piped to, indicate remote code execution.
const PIPE_TO_SHELL_TARGETS: &[&str] = &["bash", "sh", "zsh", "dash"];
/// Download commands whose output piped to a shell is dangerous.
const PIPE_FROM_DOWNLOADERS: &[&str] = &["curl", "wget"];
/// Command prefixes that can wrap a shell interpreter (e.g. `sudo bash`, `env sh`).
const PIPE_SHELL_PREFIXES: &[&str] = &["sudo", "env", "doas", "run0"];
/// Detect dangerous patterns in a command string.
///
/// Returns a list of `(pattern, description)` for each match found.
/// Matching is case-insensitive. An empty return means no patterns matched.
pub fn detect_dangerous_patterns(command: &str) -> Vec<(&'static str, &'static str)> {
    let lower = command.to_lowercase();
    let mut matches: Vec<(&str, &str)> = DANGEROUS_PATTERNS
        .iter()
        .filter(|(pattern, _)| lower.contains(*pattern))
        .copied()
        .collect();
    for &(pattern, desc) in BOUNDARY_PATTERNS {
        if let Some(pos) = lower.find(pattern) {
            let after = pos + pattern.len();
            if after >= lower.len() {
                matches.push((pattern, desc));
            } else {
                let next_char = lower.as_bytes()[after];
                if next_char == b'*' || next_char == b' ' || next_char == b'\n' {
                    matches.push((pattern, desc));
                }
            }
        }
    }
    if lower.contains('|') {
        let has_downloader = PIPE_FROM_DOWNLOADERS.iter().any(|dl| lower.contains(*dl));
        let pipe_segments: Vec<&str> = lower.split('|').collect();
        if has_downloader && pipe_segments.len() >= 2 {
            'segments: for segment in &pipe_segments[1..] {
                let trimmed = segment.trim();
                let effective_cmd = trimmed
                    .split_whitespace()
                    .find(|w| !w.starts_with('-') && !PIPE_SHELL_PREFIXES.contains(w))
                    .unwrap_or("");
                if PIPE_TO_SHELL_TARGETS.contains(&effective_cmd) {
                    matches.push(("curl|bash", "pipe remote script to shell"));
                    break 'segments;
                }
            }
        }
    }
    matches
}
