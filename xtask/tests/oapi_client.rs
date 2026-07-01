use xtask::openapi_client_check::client;

#[test]
fn extracts_and_normalizes_path_templates() {
    let src = r#"
        pub(crate) mod hosts {
            pub(crate) const BASE: &str = "/api/v1/hosts";
            pub(crate) fn by_id(id: &Uuid) -> String { format!("/api/v1/hosts/{id}") }
        }
        pub(crate) mod services {
            pub(crate) fn merge(target_id: &Uuid) -> String {
                format!("/api/v1/services/{target_id}/merge")
            }
        }
    "#;
    let mut t = client::path_templates_in_source(src);
    t.sort();
    assert_eq!(t, vec![
        "/api/v1/hosts".to_string(),
        "/api/v1/hosts/{}".to_string(),
        "/api/v1/services/{}/merge".to_string(),
    ]);
}

#[test]
fn collects_pub_async_methods_on_uptrakit_client_only() {
    let src = r#"
        impl UptrakitClient {
            pub async fn list_hosts(&self) {}
            pub async fn get_host(&self, id: &Uuid) {}
            async fn internal_helper(&self) {}
            pub fn not_async(&self) {}
        }
        impl SomethingElse { pub async fn unrelated(&self) {} }
        impl SomeTrait for UptrakitClient { async fn trait_method(&self) {} }
    "#;
    let mut m = client::methods_in_source(src);
    m.sort();
    assert_eq!(m, vec!["get_host".to_string(), "list_hosts".to_string()]);
}
