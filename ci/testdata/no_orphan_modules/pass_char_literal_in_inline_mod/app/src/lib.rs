mod inl {
    pub fn is_open_brace(c: char) -> bool {
        c == '{'
    }

    pub fn is_open_brace_byte(c: u8) -> bool {
        c == b'{'
    }

    // An escaped-quote literal immediately before a brace literal: scanning
    // for the closing quote must not stop on the escaped quote itself, or
    // the residual quote mis-consumes the separator and unblanks the brace.
    pub const QUOTE_THEN_BRACE: [char; 2] = ['\'','{'];
}

mod a;
