/// A single line of output from a command execution.
pub struct UpdateOutputLine {
    /// The text content of the output line.
    pub text: String,
    /// Which output stream this line came from.
    pub stream: UpdateOutputStream,
}

/// Output stream type for command execution.
///
/// Only includes streams that commands produce directly. Agent-level
/// streams (PreHook, PostHook, System) remain in the agent crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOutputStream {
    Stdout,
    Stderr,
}

/// Shell type for command execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Sh,
    PowerShell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_output_stream_variants() {
        assert_ne!(UpdateOutputStream::Stdout, UpdateOutputStream::Stderr);
    }

    #[test]
    fn shell_type_variants() {
        assert_ne!(ShellType::Bash, ShellType::Sh);
        assert_ne!(ShellType::Bash, ShellType::PowerShell);
        assert_ne!(ShellType::Sh, ShellType::PowerShell);
    }
}
