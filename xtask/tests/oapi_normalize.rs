use xtask::openapi_client_check::normalize::normalize_path;

#[test]
fn rewrites_named_placeholders_to_empty_braces() {
    assert_eq!(normalize_path("/api/v1/hosts"), "/api/v1/hosts");
    assert_eq!(normalize_path("/api/v1/hosts/{id}"), "/api/v1/hosts/{}");
    assert_eq!(
        normalize_path("/api/v1/software-items/{item_id}/hosts/{host_id}"),
        "/api/v1/software-items/{}/hosts/{}"
    );
    assert_eq!(
        normalize_path("/api/v1/services/{target_id}/merge"),
        "/api/v1/services/{}/merge"
    );
}
