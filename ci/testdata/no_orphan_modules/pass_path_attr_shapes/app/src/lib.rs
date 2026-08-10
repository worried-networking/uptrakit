// Single-line form: attribute and `mod` on the same line.
#[path = "actual_x.rs"] mod x;

// Interleaved-attribute form: another attribute sits between `#[path]`
// and the `mod` line.
#[path = "actual_y.rs"]
#[cfg(unix)]
mod y;
