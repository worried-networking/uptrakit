pub fn hardcoded_rust_literal() {
    let plugin_type = r#"releases_github"#;
    let raw_context = r#"channel_type = "webhook""#;
    let _ = (plugin_type, raw_context);
}
