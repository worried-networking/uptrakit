use uptrakit_service_sdk::test_support::MockTransport;

#[test]
fn mock_transport_importable() {
    let _transport = MockTransport::new();
}
