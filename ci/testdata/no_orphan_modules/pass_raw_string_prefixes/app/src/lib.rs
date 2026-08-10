// Prefixed raw strings. `br` (byte) and `cr` (C string) are raw exactly like
// `r`, so the sanitizer must consume them with the hash-delimited terminator
// rule — not with the escape-aware ordinary-string rule, which stops at the
// first bare `"` inside the literal and leaks the remainder (braces included)
// into inline-module depth tracking.
mod inl {
    pub const BYTE_RAW: &[u8] = br#"a"b{"#;
    pub const C_RAW: &std::ffi::CStr = cr#"c"d{"#;
    pub const BYTE_RAW_PLAIN: &[u8] = br"trailing backslash \";

    // Non-raw prefixed strings still take the escape-aware path.
    pub const BYTE_STR: &[u8] = b"{ escaped quote \" then brace {";
    pub const C_STR: &std::ffi::CStr = c"{ escaped quote \" then brace {";
}

mod child;
