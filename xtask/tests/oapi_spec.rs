use xtask::openapi_client_check::spec;

#[test]
fn parses_operations_with_ids() {
    let json = r#"{
      "openapi": "3.1.0",
      "paths": {
        "/api/v1/hosts": { "get": { "operationId": "list_hosts" } },
        "/api/v1/hosts/{id}": {
          "get": { "operationId": "get_host" },
          "put": { "operationId": "update_host" }
        },
        "/ignored": { "parameters": [] }
      }
    }"#;
    let ops = spec::load(json).expect("parse");
    assert_eq!(ops.len(), 3);
    assert!(
        ops.iter().any(|o| o.operation_id == "list_hosts"
            && o.path == "/api/v1/hosts"
            && o.method == "get")
    );
    assert!(
        ops.iter()
            .any(|o| o.operation_id == "update_host" && o.method == "put")
    );
}
