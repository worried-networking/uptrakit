use uptrakit_shared_types::OutputStreamType;

/// A single line of output from a command execution.
#[derive(Clone, Debug)]
pub struct UpdateOutputLine {
    /// The text content of the output line.
    pub text: String,
    /// Which output stream this line came from.
    pub stream: OutputStreamType,
}
