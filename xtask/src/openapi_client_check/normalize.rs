//! Canonicalize path templates so client (`{item_id}`) and spec (`{id}`) match.

/// Rewrite every `{name}` segment to a bare `{}`.
#[must_use]
pub fn normalize_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut in_brace = false;
    for c in p.chars() {
        match c {
            '{' => {
                in_brace = true;
                out.push_str("{}");
            }
            '}' => in_brace = false,
            _ if in_brace => {}
            _ => out.push(c),
        }
    }
    out
}
