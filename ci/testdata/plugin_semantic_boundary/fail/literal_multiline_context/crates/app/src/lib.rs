pub fn multiline_literal_context_fixture() {
    let _payload = serde_json::json!({
        "plugin_type":
            "releases_github",
    });

    let _payload_with_blank_lines = serde_json::json!({
        "plugin_type":



            "releases_gitlab",
    });

    let _route = format!(
        "/api/plugin-types/{}/config",
        "generic_shell",
    );

    let _raw_payload = concat!(
        r#"{
  "plugin_type":
"#,
        "webhook",
        r#"
}"#
    );
}
