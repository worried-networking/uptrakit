/// Render raw terminal bytes (VT100/ANSI) into plain text.
///
/// Feeds `raw` through a vt100 terminal emulator (80 rows × 220 cols) and
/// extracts the final screen state as plain text with no ANSI escape codes.
/// Trailing blank rows are trimmed. The emulator correctly collapses `\r`
/// overwrites and cursor-up/down progress-bar patterns before extraction.
///
/// # Examples
///
/// ```
/// use uptrakit_mcp::terminal::render_terminal_output;
///
/// let plain = render_terminal_output(b"hello\nworld\n");
/// assert!(plain.contains("hello"));
/// assert!(plain.contains("world"));
/// ```
pub fn render_terminal_output(raw: &[u8]) -> String {
    const ROWS: u16 = 80;
    const COLS: u16 = 220;

    if raw.is_empty() {
        return String::new();
    }

    let mut parser = vt100::Parser::new(ROWS, COLS, 0);
    parser.process(raw);
    let screen = parser.screen();

    let lines: Vec<String> = screen
        .rows(0, COLS)
        .map(|row| row.trim_end().to_owned())
        .collect();

    match lines.iter().rposition(|l| !l.is_empty()) {
        Some(idx) => lines[..=idx].join("\n"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passthrough() {
        let input = b"hello\nworld\n";
        let result = render_terminal_output(input);
        assert!(
            result.contains("hello"),
            "expected 'hello' in output, got: {result:?}"
        );
        assert!(
            result.contains("world"),
            "expected 'world' in output, got: {result:?}"
        );
    }

    #[test]
    fn carriage_return_collapses() {
        let input = b"loading\rdone\n";
        let result = render_terminal_output(input);
        assert!(result.contains("done"), "expected 'done', got: {result:?}");
        assert!(
            !result.contains("loading"),
            "expected 'loading' overwritten, got: {result:?}"
        );
    }

    #[test]
    fn ansi_sequences_stripped() {
        let input = b"\x1b[1mBold text\x1b[0m normal";
        let result = render_terminal_output(input);
        assert!(
            result.contains("Bold text"),
            "expected 'Bold text', got: {result:?}"
        );
        assert!(
            !result.contains("\x1b"),
            "expected no ANSI escapes, got: {result:?}"
        );
    }

    #[test]
    fn cursor_up_progress_bar_collapses() {
        // ESC[1A moves cursor up one line — progress bar rewrite pattern
        let input = b"0%\n\x1b[1A100%\n";
        let result = render_terminal_output(input);
        assert!(result.contains("100%"), "expected '100%', got: {result:?}");
    }

    #[test]
    fn multibyte_utf8_boundary_safe() {
        let input = "ハロー\n".as_bytes();
        let result = render_terminal_output(input);
        assert!(
            result.contains("ハロー"),
            "expected Japanese text, got: {result:?}"
        );
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(render_terminal_output(b""), "");
    }

    #[test]
    fn trailing_blank_rows_trimmed() {
        let input = b"output line\n";
        let result = render_terminal_output(input);
        assert!(
            !result.ends_with("\n\n"),
            "trailing blank rows not trimmed: {result:?}"
        );
    }
}
