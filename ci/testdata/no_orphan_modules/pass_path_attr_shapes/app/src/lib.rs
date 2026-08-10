// Single-line form: attribute and `mod` on the same line.
#[path = "actual_x.rs"] mod x;

// Interleaved-attribute form: another attribute sits between `#[path]`
// and the `mod` line.
#[path = "actual_y.rs"]
#[cfg(unix)]
mod y;

// An attribute carrying an unbalanced bracket inside a string literal, and a
// newline between `mod` and its name — both accepted by rustc, and both seen
// by EVENT_RE, so PATH_ATTR_RE must see them too.
#[path = "actual_z.rs"]
#[doc = "a]b"]
mod
z;
