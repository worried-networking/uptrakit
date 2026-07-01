use xtask::openapi_client_check::client;

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
