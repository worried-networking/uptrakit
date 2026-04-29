use rust_embed::RustEmbed;

/// SvelteKit build output embedded at compile time.
#[derive(RustEmbed)]
#[folder = "$OUT_DIR/embed"]
pub struct Assets;
