use xtask::openapi_client_check::ledgers;

#[test]
fn list_all_companion_detected_only_with_sibling() {
    let methods = vec!["list_hosts".to_string(), "list_all_hosts".to_string()];
    assert!(ledgers::is_list_all_companion("list_all_hosts", &methods));
    assert!(!ledgers::is_list_all_companion("list_all_orphans", &methods));
    assert!(!ledgers::is_list_all_companion("list_hosts", &methods));
}

#[test]
fn ledgers_have_no_double_booking() {
    ledgers::validate_no_double_booking().expect("ledgers must not double-book a name");
}
