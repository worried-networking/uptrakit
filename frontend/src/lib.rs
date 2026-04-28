use rust_embed::RustEmbed;

/// SvelteKit build output embedded at compile time.
#[derive(RustEmbed)]
#[folder = "build"]
pub struct Assets;
