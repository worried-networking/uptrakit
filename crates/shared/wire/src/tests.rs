use std::collections::{BTreeSet, HashSet};

use time::UtcDateTime;
use uuid::Uuid;

use crate::limits::{MAX_SHORT_STRING_LEN, WireValidate};

use super::*;

const TEST_UUID_1: Uuid = Uuid::from_bytes([
    0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00, 0x00,
]);
const TEST_UUID_2: Uuid = Uuid::from_bytes([
    0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00, 0x01,
]);
const TEST_UUID_3: Uuid = Uuid::from_bytes([
    0x66, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00, 0x01,
]);
// =========================================================================
// ServiceMessage tests
// =========================================================================

#[test]
fn ping_serialization_roundtrip() {
    let ping = ServiceMessage::Ping(PingPayload {
        service_ts: 1706400000000,
    });
    let json = serde_json::to_string(&ping).unwrap();
    assert_eq!(json, r#"{"type":"ping","service_ts":1706400000000}"#);

    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, ping);
}

fn agent_capabilities() -> BTreeSet<Capability> {
    [
        Capability::GracefulShutdown,
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
    ]
    .into_iter()
    .collect()
}

fn mqtt_capabilities() -> BTreeSet<Capability> {
    [Capability::GracefulShutdown, Capability::UpdateTracking]
        .into_iter()
        .collect()
}

fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    [
        Capability::GracefulShutdown,
        Capability::SoftwareDiscovery,
        Capability::SshRemote,
        Capability::UpdateHooks,
    ]
    .into_iter()
    .collect()
}

#[test]
fn register_agent_serialization_roundtrip() {
    let msg = ServiceMessage::Register(RegisterPayload::new(agent_capabilities()));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"register""#));
    assert!(json.contains(r#""software_discovery""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn register_mqtt_serialization_roundtrip() {
    let msg = ServiceMessage::Register(RegisterPayload::new(mqtt_capabilities()));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"register""#));
    assert!(json.contains(r#""update_tracking""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn register_empty_capabilities_omitted_from_json() {
    let msg = ServiceMessage::Register(RegisterPayload::new(BTreeSet::new()));
    let json = serde_json::to_string(&msg).unwrap();
    // Empty capabilities BTreeSet must be omitted (skip_serializing_if).
    assert!(!json.contains(r#""capabilities""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn register_roundtrip_accepts_missing_runtime_instance_id() {
    let json = r#"{"type":"register","capabilities":["software_discovery"]}"#;
    let msg: ServiceMessage = serde_json::from_str(json).expect("deserialize register");

    match msg {
        ServiceMessage::Register(payload) => {
            assert_eq!(payload.runtime_instance_id, None);
            assert!(
                payload
                    .capabilities
                    .contains(&Capability::SoftwareDiscovery)
            );
        }
        other => panic!("expected register, got {other:?}"),
    }
}

#[test]
fn register_roundtrip_preserves_runtime_instance_id() {
    let runtime_instance_id = TEST_UUID_1;
    let msg = ServiceMessage::Register(
        RegisterPayload::new(agent_capabilities()).with_runtime_instance_id(runtime_instance_id),
    );

    let json = serde_json::to_string(&msg).expect("serialize");
    let roundtrip: ServiceMessage = serde_json::from_str(&json).expect("deserialize");

    match roundtrip {
        ServiceMessage::Register(payload) => {
            assert_eq!(payload.runtime_instance_id, Some(runtime_instance_id));
        }
        other => panic!("expected register, got {other:?}"),
    }
}

#[test]
fn enroll_agent_serialization_roundtrip() {
    let msg = ServiceMessage::Enroll(EnrollPayload {
        hostname: "node-1".to_string(),
        friendly_name: "Node One".to_string(),
        enrollment_token: Some(SecretString::new("tok-123")),
        capabilities: agent_capabilities(),
        service_app_name: "uptrakit-agent".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
    assert!(json.contains(r#""capabilities""#));
    assert!(json.contains(r#""software_discovery""#));
}

#[test]
fn enroll_mqtt_serialization_roundtrip() {
    let msg = ServiceMessage::Enroll(EnrollPayload {
        hostname: "mqtt-service-1".to_string(),
        friendly_name: "MQTT Service Node 1".to_string(),
        enrollment_token: Some(SecretString::new("tok-456")),
        capabilities: mqtt_capabilities(),
        service_app_name: "uptrakit-mqtt".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"enroll"#));
    assert!(json.contains(r#""hostname":"mqtt-service-1"#));
    assert!(json.contains(r#""update_tracking""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn enroll_ssh_agent_serialization_roundtrip() {
    let msg = ServiceMessage::Enroll(EnrollPayload {
        hostname: "ssh-agent-1".to_string(),
        friendly_name: "SSH Agent Node 1".to_string(),
        enrollment_token: Some(SecretString::new("tok-789")),
        capabilities: ssh_agent_capabilities(),
        service_app_name: "uptrakit-agent-ssh".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"enroll"#));
    assert!(json.contains(r#""hostname":"ssh-agent-1"#));
    assert!(json.contains(r#""ssh_remote""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn enroll_without_token_serialization_roundtrip() {
    let msg = ServiceMessage::Enroll(EnrollPayload {
        hostname: "node-2".to_string(),
        friendly_name: "Node Two".to_string(),
        enrollment_token: None,
        capabilities: agent_capabilities(),
        service_app_name: "uptrakit-agent".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    // enrollment_token should be omitted when None
    assert!(!json.contains("enrollment_token"));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn request_certificate_serialization_roundtrip() {
    let msg = ServiceMessage::RequestCertificate(RequestCertificatePayload {
        csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
            .to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"request_certificate"#));
    assert!(json.contains(r#""csr_pem":"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn renew_certificate_serialization_roundtrip() {
    let msg = ServiceMessage::RenewCertificate(RenewCertificatePayload {
        csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\nrenew\n-----END CERTIFICATE REQUEST-----\n"
            .to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"renew_certificate"#));
    assert!(json.contains(r#""csr_pem":"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn report_hosts_serialization_roundtrip() {
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-42".to_string(),
            os_type: Some("linux".to_string()),
            os_version: Some("Ubuntu 24.04 LTS".to_string()),
            architecture: Some("x86_64".to_string()),
            hostname: None,
            ip_address: None,
            agent_host_id: None,
            features: None,
        }],
        agent_version: "0.0.1".to_string(),
        capabilities: BTreeSet::new(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"report_hosts"#));
    assert!(json.contains(r#""agent_version":"0.0.1"#));
    assert!(json.contains(r#""hosts":[{"machine_id":"machine-42""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn report_hosts_multiple_hosts() {
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: vec![
            HostInfo {
                machine_id: "host-a".to_string(),
                os_type: Some("linux".to_string()),
                os_version: None,
                architecture: None,
                hostname: None,
                ip_address: None,
                agent_host_id: None,
                features: None,
            },
            HostInfo {
                machine_id: "host-b".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("Debian 12".to_string()),
                architecture: Some("aarch64".to_string()),
                hostname: None,
                ip_address: None,
                agent_host_id: None,
                features: None,
            },
        ],
        agent_version: "0.0.1".to_string(),
        capabilities: BTreeSet::new(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""host-a"#));
    assert!(json.contains(r#""host-b"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn version_check_results_serialization_roundtrip() {
    let msg = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
        results: vec![
            VersionCheckResult {
                software_item_id: TEST_UUID_1,
                host_software_item_id: None,
                installed_version: Some("1.2.3".to_string()),
                installed_display_version: None,
                latest_version: None,
                error: None,
                update_category: UpdateCategory::Unknown,
            },
            VersionCheckResult {
                software_item_id: TEST_UUID_2,
                host_software_item_id: None,
                installed_version: None,
                installed_display_version: None,
                latest_version: None,
                error: Some("detection failed".to_string()),
                update_category: UpdateCategory::Unknown,
            },
        ],
    });
    let value = serde_json::to_value(&msg).unwrap();
    assert_eq!(
        value.get("type"),
        Some(&serde_json::Value::String(
            "version_check_results".to_string()
        ))
    );

    let results = value
        .get("results")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert_eq!(results.len(), 2);

    let first = results[0].as_object().unwrap();
    assert_eq!(
        first.get("installed_version"),
        Some(&serde_json::json!("1.2.3"))
    );
    assert!(!first.contains_key("error"));
    assert!(!first.contains_key("latest_version"));
    assert!(!first.contains_key("installed_display_version"));
    assert!(!first.contains_key("host_software_item_id"));

    let second = results[1].as_object().unwrap();
    assert_eq!(
        second.get("error"),
        Some(&serde_json::json!("detection failed"))
    );
    assert!(!second.contains_key("installed_version"));
    assert!(!second.contains_key("latest_version"));
    assert!(!second.contains_key("installed_display_version"));
    assert!(!second.contains_key("host_software_item_id"));

    let deserialized: ServiceMessage = serde_json::from_value(value).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn version_check_result_with_latest_version() {
    let msg = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
        results: vec![VersionCheckResult {
            software_item_id: TEST_UUID_1,
            host_software_item_id: None,
            installed_version: Some("1.24.4".to_string()),
            installed_display_version: None,
            latest_version: Some("1.24.5".to_string()),
            error: None,
            update_category: UpdateCategory::Unknown,
        }],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""latest_version":"1.24.5"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn version_check_result_backward_compat_no_latest_version() {
    // Messages from older agents that don't include latest_version
    // should still deserialize correctly.
    let json = serde_json::json!({
        "type": "version_check_results",
        "results": [{
            "software_item_id": TEST_UUID_1.to_string(),
            "installed_version": "1.0.0"
        }]
    });
    let msg: ServiceMessage = serde_json::from_value(json).unwrap();
    if let ServiceMessage::VersionCheckResults(payload) = msg {
        assert_eq!(
            payload.results[0].installed_version,
            Some("1.0.0".to_string())
        );
        assert_eq!(payload.results[0].latest_version, None);
    } else {
        panic!("expected VersionCheckResults");
    }
}

#[test]
fn version_check_result_backward_compat_no_error() {
    // Messages from older agents that don't include error
    // should still deserialize correctly.
    let json = serde_json::json!({
        "type": "version_check_results",
        "results": [{
            "software_item_id": TEST_UUID_1.to_string(),
            "installed_version": "1.0.0",
            "update_category": "unknown"
        }]
    });
    let msg: ServiceMessage = serde_json::from_value(json).unwrap();
    if let ServiceMessage::VersionCheckResults(payload) = msg {
        assert_eq!(payload.results.len(), 1);
        assert_eq!(payload.results[0].software_item_id, TEST_UUID_1);
        assert_eq!(
            payload.results[0].installed_version,
            Some("1.0.0".to_string())
        );
        assert_eq!(payload.results[0].update_category, UpdateCategory::Unknown);
        assert!(payload.results[0].error.is_none());
    } else {
        panic!("expected VersionCheckResults");
    }
}

#[test]
fn update_started_serialization_roundtrip() {
    let msg = ServiceMessage::UpdateStarted(UpdateStartedPayload {
        update_history_id: TEST_UUID_1,
        from_version: Some("1.0.0".to_string()),
        interactive: true,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"update_started"#));
    assert!(json.contains(r#""from_version":"1.0.0"#));
    assert!(json.contains(r#""interactive":true"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn update_started_omits_none_from_version() {
    let msg = ServiceMessage::UpdateStarted(UpdateStartedPayload {
        update_history_id: TEST_UUID_1,
        from_version: None,
        interactive: false,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("from_version"));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn update_started_interactive_defaults_false_on_missing_field() {
    // Old agents that do not send the `interactive` field must deserialize as false.
    let json = r#"{"type":"update_started","protocol_version":1,"seq":1,"update_history_id":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let msg: ServiceMessage = serde_json::from_str(json).unwrap();
    if let ServiceMessage::UpdateStarted(payload) = msg {
        assert!(!payload.interactive);
    } else {
        panic!("expected UpdateStarted");
    }
}

#[test]
fn update_output_serialization_roundtrip() {
    let msg = ServiceMessage::UpdateOutput(UpdateOutputPayload {
        update_history_id: TEST_UUID_1,
        output: "Downloading package...".to_string(),
        stream: OutputStreamType::Stdout,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"update_output"#));
    assert!(json.contains(r#""stream":"stdout"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn update_output_all_stream_types() {
    for (stream, expected) in [
        (OutputStreamType::Stdout, "stdout"),
        (OutputStreamType::Stderr, "stderr"),
        (OutputStreamType::PreHook, "pre_hook"),
        (OutputStreamType::PostHook, "post_hook"),
        (OutputStreamType::System, "system"),
    ] {
        let msg = ServiceMessage::UpdateOutput(UpdateOutputPayload {
            update_history_id: TEST_UUID_1,
            output: "test".to_string(),
            stream,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(&format!(r#""stream":"{expected}""#)));
    }
}

#[test]
fn update_output_default_stream() {
    let json = r#"{"type":"update_output","update_history_id":"550e8400-e29b-41d4-a716-446655440000","output":"test"}"#;
    let msg: ServiceMessage = serde_json::from_str(json).unwrap();
    if let ServiceMessage::UpdateOutput(payload) = msg {
        assert_eq!(payload.stream, OutputStreamType::Stdout);
    } else {
        panic!("Expected UpdateOutput");
    }
}

#[test]
fn update_result_completed_serialization_roundtrip() {
    let msg = ServiceMessage::UpdateResult(UpdateResultPayload {
        update_history_id: TEST_UUID_1,
        status: UpdateFinalStatus::Completed,
        from_version: Some("1.0.0".to_string()),
        to_version: Some("2.0.0".to_string()),
        output: "Update completed successfully".to_string(),
        error: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"update_result"#));
    assert!(json.contains(r#""status":"completed"#));
    assert!(!json.contains("error"));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn update_result_failed_serialization_roundtrip() {
    let msg = ServiceMessage::UpdateResult(UpdateResultPayload {
        update_history_id: TEST_UUID_1,
        status: UpdateFinalStatus::Failed,
        from_version: None,
        to_version: None,
        output: "Error output".to_string(),
        error: Some("Package not found".to_string()),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"update_result"#));
    assert!(json.contains(r#""status":"failed"#));
    assert!(json.contains(r#""error":"Package not found"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn disconnecting_shutdown_serialization_roundtrip() {
    let msg = ServiceMessage::Disconnecting(DisconnectingPayload::new(DisconnectReason::Shutdown));
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"type":"disconnecting","reason":"shutdown"}"#);
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn disconnecting_restart_serialization_roundtrip() {
    let msg = ServiceMessage::Disconnecting(DisconnectingPayload::new(DisconnectReason::Restart));
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"type":"disconnecting","reason":"restart"}"#);
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

// =========================================================================
// ControllerMessage tests
// =========================================================================

#[test]
fn pong_serialization_roundtrip() {
    let pong = ControllerMessage::Pong(PongPayload {
        service_ts: 1706400000000,
        controller_ts: 1706400000050,
    });
    let json = serde_json::to_string(&pong).unwrap();
    assert_eq!(
        json,
        r#"{"type":"pong","service_ts":1706400000000,"controller_ts":1706400000050}"#
    );

    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, pong);
}

#[test]
fn enrolled_serialization_roundtrip() {
    let msg = ControllerMessage::Enrolled(EnrolledPayload {
        service_id: TEST_UUID_1,
        enrollment_secret: SecretString::new("secret-abc"),
        status: EnrollmentStatus::Pending,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(
        json,
        r#"{"type":"enrolled","service_id":"550e8400-e29b-41d4-a716-446655440000","enrollment_secret":"secret-abc","status":"pending"}"#
    );
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn approved_serialization_roundtrip() {
    let msg = ControllerMessage::Approved(ApprovedPayload {
        service_id: TEST_UUID_1,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(
        json,
        r#"{"type":"approved","service_id":"550e8400-e29b-41d4-a716-446655440000"}"#
    );
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn rejected_serialization_roundtrip() {
    let msg = ControllerMessage::Rejected(RejectedPayload {
        service_id: TEST_UUID_1,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(
        json,
        r#"{"type":"rejected","service_id":"550e8400-e29b-41d4-a716-446655440000"}"#
    );
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn certificate_serialization_roundtrip() {
    let msg = ControllerMessage::Certificate(CertificatePayload {
        cert_pem: "-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----\n".to_string(),
        not_after: UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("key_pem"));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn error_serialization_roundtrip() {
    let msg = ControllerMessage::Error(ErrorPayload {
        code: ErrorCode::EnrollmentFailed,
        message: "The enrollment token is invalid".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(
        json,
        r#"{"type":"error","code":"enrollment_failed","message":"The enrollment token is invalid"}"#
    );
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_settings_serialization_roundtrip() {
    let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours: 6,
        ca_bundle_hash: "abc123".to_string(),
        capabilities: BTreeSet::new(),
        report_page_limits: ReportPageLimits::default(),
        shutdown_timeout: Some(std::time::Duration::from_secs(120)),
        ping_interval: std::time::Duration::from_secs(300),
        tenant_id: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(
        json,
        r#"{"type":"service_settings","renewal_window_hours":6,"ca_bundle_hash":"abc123","shutdown_timeout_seconds":120,"ping_interval":300}"#
    );
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_settings_without_shutdown_timeout() {
    let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours: 6,
        ca_bundle_hash: "abc123def".to_string(),
        capabilities: BTreeSet::new(),
        report_page_limits: ReportPageLimits::default(),
        shutdown_timeout: None,
        ping_interval: std::time::Duration::from_secs(15),
        tenant_id: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"service_settings"#));
    assert!(!json.contains("shutdown_timeout_seconds"));
    assert!(json.contains(r#""ping_interval":15"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_settings_serializes_non_default_report_page_limits() {
    let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours: 6,
        ca_bundle_hash: "abc123".to_string(),
        capabilities: BTreeSet::new(),
        report_page_limits: ReportPageLimits {
            report_hosts: 100,
            ..ReportPageLimits::default()
        },
        shutdown_timeout: None,
        ping_interval: std::time::Duration::from_secs(300),
        tenant_id: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""report_page_limits":{"report_hosts":100"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_settings_backward_compat_extra_fields() {
    // Future-proof: extra fields in JSON should be ignored
    let json = r#"{"type":"service_settings","renewal_window_hours":12,"ca_bundle_hash":"def456","shutdown_timeout_seconds":60,"ping_interval":300,"some_future_field":"value"}"#;
    let msg: ControllerMessage = serde_json::from_str(json).unwrap();
    assert_eq!(
        msg,
        ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 12,
            ca_bundle_hash: "def456".to_string(),
            capabilities: BTreeSet::new(),
            report_page_limits: ReportPageLimits::default(),
            shutdown_timeout: Some(std::time::Duration::from_secs(60)),
            ping_interval: std::time::Duration::from_secs(300),
            tenant_id: None,
        })
    );
}

#[test]
fn service_settings_backward_compat_missing_shutdown_timeout() {
    // Services running older protocol without shutdown_timeout_seconds should still parse
    let json = r#"{"type":"service_settings","renewal_window_hours":6,"ca_bundle_hash":"abc","ping_interval":300}"#;
    let msg: ControllerMessage = serde_json::from_str(json).unwrap();
    assert_eq!(
        msg,
        ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "abc".to_string(),
            capabilities: BTreeSet::new(),
            report_page_limits: ReportPageLimits::default(),
            shutdown_timeout: None,
            ping_interval: std::time::Duration::from_secs(300),
            tenant_id: None,
        })
    );
}

#[test]
fn duration_seconds_roundtrip() {
    let payload = ServiceSettingsPayload {
        renewal_window_hours: 6,
        ca_bundle_hash: String::new(),
        capabilities: BTreeSet::new(),
        report_page_limits: ReportPageLimits::default(),
        shutdown_timeout: None,
        ping_interval: std::time::Duration::from_secs(42),
        tenant_id: None,
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["ping_interval"], 42);
    let deserialized: ServiceSettingsPayload = serde_json::from_value(json).unwrap();
    assert_eq!(
        deserialized.ping_interval,
        std::time::Duration::from_secs(42)
    );
}

#[test]
fn request_cert_renewal_serialization_roundtrip() {
    let msg = ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
        reason: "CA rotation after backend URL change".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"request_cert_renewal"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn server_restarting_serialization_roundtrip() {
    let msg = ControllerMessage::ServerRestarting(ServerRestartingPayload {
        reason: "controller restarting for upgrade".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"server_restarting"#));
    assert!(json.contains(r#""reason":"controller restarting for upgrade"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn server_restarting_backward_compat_extra_fields() {
    let json = r#"{"type":"server_restarting","reason":"restart","unknown_field":"ignored"}"#;
    let msg: ControllerMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ControllerMessage::ServerRestarting(_)));
}

#[test]
fn check_versions_serialization_roundtrip() {
    let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
        host_machine_id: "test-machine-id".to_string(),
        assignments: vec![VersionCheckAssignment {
            software_item_id: TEST_UUID_1,
            name: "Test Software".to_string(),
            detect_version: Some(PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "octocat/hello-world".to_string(),
                config: serde_json::json!({}),
            }),
            fetch_releases: None,
            host_software_item_id: None,
        }],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"check_versions"#));
    assert!(json.contains(r#""software_item_id":"550e8400-e29b-41d4-a716-446655440000"#));
    assert!(json.contains(r#""plugin_type":"releases_github"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn execute_update_serialization_roundtrip() {
    let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
        host_machine_id: "test-machine-id".to_string(),
        update_history_id: Uuid::parse_str("01936a1e-7e8c-7f00-8000-000000000001").unwrap(),
        software_item_id: Uuid::parse_str("01936a1e-7e8c-7f00-8000-000000000002").unwrap(),
        software_item_name: "Node.js".to_string(),
        to_version: "20.10.0".to_string(),
        detect_version_plugin: Some(PluginAssignment {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            package_identifier: "nodejs/node".to_string(),
            config: serde_json::json!({}),
        }),
        execute_update_plugin: PluginAssignment {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            package_identifier: "nodejs/node".to_string(),
            config: serde_json::json!({}),
        },
        pre_update_hook_plugins: vec![PluginAssignment {
            plugin_type: plugin_ids::HOOK_SYSTEMD.clone(),
            package_identifier: String::new(),
            config: serde_json::json!({"service_name": "myapp"}),
        }],
        post_update_hook_plugins: vec![PluginAssignment {
            plugin_type: plugin_ids::HOOK_SYSTEMD.clone(),
            package_identifier: String::new(),
            config: serde_json::json!({"service_name": "myapp"}),
        }],
        release_info: Some(ReleaseInfo {
            tag: "v20.10.0".to_string(),
            release_url: "https://github.com/nodejs/node/releases/tag/v20.10.0".to_string(),
            assets: vec![ReleaseAsset {
                name: "node-v20.10.0-linux-x64.tar.gz".to_string(),
                download_url: "https://github.com/nodejs/node/releases/download/v20.10.0/node-v20.10.0-linux-x64.tar.gz".to_string(),
                size: Some(25_000_000),
                content_type: None,
                sha256_digest: None,
            }],
            attestation_status: None,
            require_attestation: false,
        }),
        timeout: std::time::Duration::from_secs(600),
        interactive: false,
    }));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"execute_update"#));
    assert!(json.contains(r#""plugin_type":"releases_github"#));
    assert!(json.contains(r#""hook_systemd"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn execute_update_minimal_serialization() {
    let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
        host_machine_id: "test-machine-id".to_string(),
        update_history_id: TEST_UUID_1,
        software_item_id: TEST_UUID_2,
        software_item_name: "Redis".to_string(),
        to_version: "7.2.0".to_string(),
        detect_version_plugin: None,
        execute_update_plugin: PluginAssignment {
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            package_identifier: "redis-server".to_string(),
            config: serde_json::json!({}),
        },
        pre_update_hook_plugins: vec![],
        post_update_hook_plugins: vec![],
        release_info: None,
        timeout: DEFAULT_UPDATE_TIMEOUT,
        interactive: false,
    }));
    let json = serde_json::to_string(&msg).unwrap();
    // Empty vectors should be omitted
    assert!(!json.contains("pre_update_hook_plugins"));
    assert!(!json.contains("post_update_hook_plugins"));
    assert!(!json.contains("release_info"));
    // detect_version_plugin should be omitted when None
    assert!(!json.contains("detect_version_plugin"));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn execute_update_default_timeout() {
    let json = r#"{
        "type": "execute_update",
        "host_machine_id": "test-machine-id",
        "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
        "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
        "software_item_name": "Test",
        "to_version": "1.0.0",
        "execute_update_plugin": {
            "plugin_type": "releases_github",
            "package_identifier": "test",
            "config": {}
        }
    }"#;
    let msg: ControllerMessage = serde_json::from_str(json).unwrap();
    if let ControllerMessage::ExecuteUpdate(payload) = msg {
        assert_eq!(payload.timeout, DEFAULT_UPDATE_TIMEOUT);
        assert!(payload.pre_update_hook_plugins.is_empty());
        assert!(payload.post_update_hook_plugins.is_empty());
    } else {
        panic!("Expected ExecuteUpdate");
    }
}

#[test]
fn execute_update_with_shell_hook_plugin() {
    let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
        host_machine_id: "test-machine-id".to_string(),
        update_history_id: TEST_UUID_1,
        software_item_id: TEST_UUID_2,
        software_item_name: "Test".to_string(),
        to_version: "1.0.0".to_string(),
        detect_version_plugin: None,
        execute_update_plugin: PluginAssignment {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            package_identifier: "test".to_string(),
            config: serde_json::json!({}),
        },
        pre_update_hook_plugins: vec![PluginAssignment {
            plugin_type: plugin_ids::HOOK_SHELL.clone(),
            package_identifier: String::new(),
            config: serde_json::json!({"pre_command": "echo hello", "shell": "sh"}),
        }],
        post_update_hook_plugins: vec![],
        release_info: None,
        timeout: DEFAULT_UPDATE_TIMEOUT,
        interactive: false,
    }));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""hook_shell"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn execute_update_backward_compat_extra_fields() {
    let json = r#"{
        "type": "execute_update",
        "host_machine_id": "test-machine-id",
        "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
        "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
        "software_item_name": "Test",
        "to_version": "1.0.0",
        "execute_update_plugin": {
            "plugin_type": "releases_github",
            "package_identifier": "test",
            "config": {}
        },
        "unknown_field": "ignored"
    }"#;
    let msg: ControllerMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ControllerMessage::ExecuteUpdate(_)));
}

// =========================================================================
// Shared payload and helper tests
// =========================================================================

#[test]
fn now_millis_returns_reasonable_value() {
    let ts = now_millis();
    // Should be after 2024-01-01 (1704067200000)
    assert!(ts > 1704067200000);
}

#[test]
fn host_info_minimal_serialization_roundtrip() {
    let info = HostInfo {
        machine_id: "unknown".to_string(),
        os_type: None,
        os_version: None,
        architecture: None,
        hostname: None,
        ip_address: None,
        agent_host_id: None,
        features: None,
    };
    let json = serde_json::to_string(&info).unwrap();
    assert_eq!(json, r#"{"machine_id":"unknown"}"#);
    let deserialized: HostInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, info);
}

#[test]
fn host_info_with_hostname_and_ip() {
    let info = HostInfo {
        machine_id: "abc-123".to_string(),
        os_type: Some("linux".to_string()),
        os_version: None,
        architecture: None,
        hostname: Some("web-01.example.com".to_string()),
        ip_address: Some("10.0.0.5".to_string()),
        agent_host_id: None,
        features: None,
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains(r#""hostname":"web-01.example.com"#));
    assert!(json.contains(r#""ip_address":"10.0.0.5"#));
    let deserialized: HostInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, info);
}

#[test]
fn host_info_deserializes_without_new_fields() {
    // Ensures backward compatibility: old agents that don't send hostname/ip_address
    // still deserialize correctly (fields default to None).
    let json = r#"{"machine_id":"legacy","os_type":"linux"}"#;
    let info: HostInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.machine_id, "legacy");
    assert_eq!(info.hostname, None);
    assert_eq!(info.ip_address, None);
}

#[test]
fn enrollment_status_all_variants() {
    for status in [EnrollmentStatus::Pending, EnrollmentStatus::Approved] {
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: EnrollmentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }
}

#[test]
fn hook_shell_all_variants() {
    for (shell, expected) in [
        (HookShell::Bash, "bash"),
        (HookShell::Sh, "sh"),
        (HookShell::PowerShell, "powershell"),
    ] {
        let json = serde_json::to_string(&shell).unwrap();
        assert_eq!(json, format!(r#""{expected}""#));
        let deserialized: HookShell = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, shell);
    }
}

#[test]
fn hook_shell_default_is_bash() {
    assert_eq!(HookShell::default(), HookShell::Bash);
}

#[test]
fn enrollment_status_display_matches_serde() {
    for status in [EnrollmentStatus::Pending, EnrollmentStatus::Approved] {
        let serde_str = serde_json::to_value(&status).unwrap();
        assert_eq!(
            status.to_string(),
            serde_str.as_str().unwrap(),
            "Display must match serde for {status:?}"
        );
    }
}

#[test]
fn enrollment_status_unknown_becomes_other() {
    let result: EnrollmentStatus = serde_json::from_str(r#""suspended""#).unwrap();
    assert_eq!(result, EnrollmentStatus::Other("suspended".to_string()));
}

#[test]
fn enrollment_status_other_round_trip() {
    let original = r#""future_status""#;
    let deserialized: EnrollmentStatus = serde_json::from_str(original).unwrap();
    assert_eq!(
        deserialized,
        EnrollmentStatus::Other("future_status".to_string())
    );
    let reserialized = serde_json::to_string(&deserialized).unwrap();
    assert_eq!(reserialized, original);
}

#[test]
fn error_code_display_matches_serde() {
    for code in [
        ErrorCode::BadRequest,
        ErrorCode::EnrollmentFailed,
        ErrorCode::NotApproved,
        ErrorCode::Forbidden,
        ErrorCode::CertificateError,
        ErrorCode::InternalError,
        ErrorCode::SequenceError,
    ] {
        let display = code.to_string();
        let serde_str = serde_json::to_value(&code).unwrap();
        assert_eq!(
            display,
            serde_str.as_str().unwrap(),
            "Display must match serde for {code:?}"
        );
    }
}

#[test]
fn hook_shell_rejects_invalid() {
    let result: std::result::Result<HookShell, _> = serde_json::from_str(r#""zsh""#);
    assert!(result.is_err());
}

#[test]
fn error_code_serde_roundtrip() {
    for (variant, expected_str) in [
        (ErrorCode::BadRequest, "bad_request"),
        (ErrorCode::EnrollmentFailed, "enrollment_failed"),
        (ErrorCode::NotApproved, "not_approved"),
        (ErrorCode::Forbidden, "forbidden"),
        (ErrorCode::CertificateError, "certificate_error"),
        (ErrorCode::InternalError, "internal_error"),
        (ErrorCode::SequenceError, "sequence_error"),
    ] {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, format!(r#""{expected_str}""#));
        let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, variant);
    }
}

#[test]
fn error_code_sequence_error_serde() {
    let json = serde_json::to_string(&ErrorCode::SequenceError).unwrap();
    assert_eq!(json, r#""sequence_error""#);
    let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, ErrorCode::SequenceError);
}

#[test]
fn error_code_unknown_becomes_other() {
    let result: ErrorCode = serde_json::from_str(r#""unknown_code""#).unwrap();
    assert_eq!(result, ErrorCode::Other("unknown_code".to_string()));
}

#[test]
fn error_code_other_round_trip() {
    let original = r#""future_error""#;
    let deserialized: ErrorCode = serde_json::from_str(original).unwrap();
    assert_eq!(deserialized, ErrorCode::Other("future_error".to_string()));
    let reserialized = serde_json::to_string(&deserialized).unwrap();
    assert_eq!(reserialized, original);
}

#[test]
fn error_code_display() {
    assert_eq!(ErrorCode::BadRequest.to_string(), "bad_request");
    assert_eq!(ErrorCode::EnrollmentFailed.to_string(), "enrollment_failed");
    assert_eq!(ErrorCode::NotApproved.to_string(), "not_approved");
    assert_eq!(ErrorCode::Forbidden.to_string(), "forbidden");
    assert_eq!(ErrorCode::CertificateError.to_string(), "certificate_error");
    assert_eq!(ErrorCode::InternalError.to_string(), "internal_error");
    assert_eq!(ErrorCode::SequenceError.to_string(), "sequence_error");
    assert_eq!(ErrorCode::Other("custom".to_string()).to_string(), "custom");
}

#[test]
fn disconnect_reason_all_variants() {
    for (reason, expected) in [
        (DisconnectReason::Shutdown, "shutdown"),
        (DisconnectReason::Restart, "restart"),
    ] {
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, format!(r#""{expected}""#));
        let deserialized: DisconnectReason = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, reason);
    }
}

#[test]
fn version_check_assignment_serialization() {
    let assignment = VersionCheckAssignment {
        software_item_id: TEST_UUID_1,
        name: "Docker Image".to_string(),
        detect_version: Some(PluginAssignment {
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            package_identifier: "nginx:latest".to_string(),
            config: serde_json::json!({}),
        }),
        fetch_releases: None,
        host_software_item_id: None,
    };
    let json = serde_json::to_string(&assignment).unwrap();
    assert!(json.contains(r#""plugin_type":"releases_docker""#));
    let deserialized: VersionCheckAssignment = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, assignment);
}

#[test]
fn plugin_type_all_variants() {
    for (plugin, expected) in [
        (plugin_ids::RELEASES_GITHUB.clone(), "releases_github"),
        (
            plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            "discovery_proxmox_helper_scripts",
        ),
        (plugin_ids::RELEASES_DOCKER.clone(), "releases_docker"),
    ] {
        let json = serde_json::to_string(&plugin).unwrap();
        assert_eq!(json, format!(r#""{expected}""#));
        let deserialized: PluginTypeId = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, plugin);
    }
}

/// A `VersionCheckAssignment` carrying an unknown plugin type from a
/// newer server must deserialize without error.  The entire message is
/// preserved so the agent can log the skip reason instead of crashing.
#[test]
fn version_check_assignment_with_unknown_plugin_type_deserializes() {
    let json = serde_json::json!({
        "software_item_id": "00000000-0000-0000-0000-000000000001",
        "name": "My App",
        "detect_version": {
            "plugin_type": "winget",
            "package_identifier": "my-app",
            "config": {}
        }
    });
    let assignment: VersionCheckAssignment =
        serde_json::from_value(json).expect("should deserialize");
    assert_eq!(
        assignment.detect_version.as_ref().unwrap().plugin_type,
        PluginTypeId::new("winget")
    );
}

/// `"package_manager_apt"` deserializes to the known `PackageManagerApt` variant in `VersionCheckAssignment`.
#[test]
fn version_check_assignment_apt_plugin_type_deserializes() {
    let json = serde_json::json!({
        "software_item_id": "00000000-0000-0000-0000-000000000001",
        "name": "nginx",
        "detect_version": {
            "plugin_type": "package_manager_apt",
            "package_identifier": "nginx",
            "config": {}
        }
    });
    let assignment: VersionCheckAssignment =
        serde_json::from_value(json).expect("should deserialize");
    assert_eq!(
        assignment.detect_version.as_ref().unwrap().plugin_type,
        plugin_ids::PACKAGE_MANAGER_APT.clone()
    );
}

#[test]
fn release_info_empty_assets_omitted() {
    let info = ReleaseInfo {
        tag: "v1.0.0".to_string(),
        release_url: "https://example.com/release".to_string(),
        assets: vec![],
        attestation_status: None,
        require_attestation: false,
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("assets"));
    assert!(!json.contains("attestation_status"));
    let deserialized: ReleaseInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, info);
}

#[test]
fn enroll_requires_capabilities() {
    // Enrollment without capabilities should fail deserialization.
    let json = r#"{"type":"enroll","hostname":"node-old","friendly_name":"Old Node"}"#;
    let result: std::result::Result<ServiceMessage, _> = serde_json::from_str(json);
    assert!(result.is_err(), "EnrollPayload requires capabilities");
}

// =========================================================================
// Envelope and sequence number tests
// =========================================================================

#[test]
fn service_envelope_serde_roundtrip() {
    let envelope = ServiceEnvelope {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        seq: 1,
        trace_context: TraceContext {
            trace_id: "0".repeat(32),
            span_id: None,
        },
        pagination: None,
        message: ServiceMessage::Ping(PingPayload {
            service_ts: 1706400000000,
        }),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains(r#""protocol_version":1"#));
    assert!(json.contains(r#""seq":1"#));
    assert!(json.contains(r#""trace_context""#));
    assert!(json.contains(r#""type":"ping""#));
    let deserialized: ServiceEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, envelope);
}

#[test]
fn controller_envelope_serde_roundtrip() {
    let envelope = ControllerEnvelope {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        seq: 42,
        trace_context: TraceContext {
            trace_id: "f".repeat(32),
            span_id: Some("a".repeat(16)),
        },
        message: ControllerMessage::Pong(PongPayload {
            service_ts: 1706400000000,
            controller_ts: 1706400000050,
        }),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains(r#""protocol_version":1"#));
    assert!(json.contains(r#""seq":42"#));
    assert!(json.contains(r#""trace_context""#));
    assert!(json.contains(r#""span_id""#));
    assert!(json.contains(r#""type":"pong""#));
    let deserialized: ControllerEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, envelope);
}

#[test]
fn service_message_unknown_type_deserializes_to_unknown_variant() {
    // Forward-compatibility: an unknown message type from a newer service
    // build must deserialize to `ServiceMessage::Unknown`, not fail.
    let json = r#"{"protocol_version":1,"seq":1,"type":"future_message","payload":{"foo":"bar"}}"#;
    let envelope: ServiceEnvelope = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(
        envelope.message,
        ServiceMessage::Unknown,
        "unknown type tag must produce ServiceMessage::Unknown"
    );
}

#[test]
fn controller_message_unknown_type_deserializes_to_unknown_variant() {
    // Forward-compatibility: an unknown message type from a newer controller
    // build must deserialize to `ControllerMessage::Unknown`, not fail.
    let json = r#"{"protocol_version":1,"seq":1,"type":"future_command","data":{}}"#;
    let envelope: ControllerEnvelope = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(
        envelope.message,
        ControllerMessage::Unknown,
        "unknown type tag must produce ControllerMessage::Unknown"
    );
}

#[test]
fn unknown_controller_message_is_not_nats_publishable() {
    assert!(
        !ControllerMessage::Unknown.is_nats_publishable(),
        "Unknown must not be published to NATS"
    );
}

#[test]
fn service_envelope_missing_protocol_version_fails() {
    // Old-format envelope without protocol_version must fail deserialization.
    let json = r#"{"seq":1,"type":"ping","service_ts":1706400000000}"#;
    assert!(serde_json::from_str::<ServiceEnvelope>(json).is_err());
}

#[test]
fn controller_envelope_missing_protocol_version_fails() {
    // Old-format envelope without protocol_version must fail deserialization.
    let json =
        r#"{"seq":42,"type":"pong","service_ts":1706400000000,"controller_ts":1706400000050}"#;
    assert!(serde_json::from_str::<ControllerEnvelope>(json).is_err());
}

#[test]
fn service_envelope_complex_message() {
    let envelope = ServiceEnvelope {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        seq: 3,
        trace_context: TraceContext::generate(),
        pagination: None,
        message: ServiceMessage::Enroll(EnrollPayload {
            hostname: "test-host".to_string(),
            friendly_name: "Test".to_string(),
            enrollment_token: None,
            capabilities: agent_capabilities(),
            service_app_name: "uptrakit-agent".to_string(),
        }),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains(r#""protocol_version":1"#));
    assert!(json.contains(r#""seq":3"#));
    assert!(json.contains(r#""type":"enroll"#));
    let deserialized: ServiceEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, envelope);
}

#[test]
fn controller_envelope_error_message() {
    let envelope = ControllerEnvelope {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        seq: 5,
        trace_context: TraceContext::generate(),
        message: ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::SequenceError,
            message: "sequence error: expected 3, received 5".to_string(),
        }),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains(r#""protocol_version":1"#));
    assert!(json.contains(r#""seq":5"#));
    assert!(json.contains(r#""code":"sequence_error""#));
    let deserialized: ControllerEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, envelope);
}

#[test]
fn outgoing_seq_increments() {
    let mut seq = OutgoingSeq::new();
    let e1 = seq.wrap_service(
        ServiceMessage::Ping(PingPayload { service_ts: 1 }),
        current_trace_context(),
    );
    let e2 = seq.wrap_service(
        ServiceMessage::Ping(PingPayload { service_ts: 2 }),
        current_trace_context(),
    );
    let e3 = seq.wrap_service(
        ServiceMessage::Ping(PingPayload { service_ts: 3 }),
        current_trace_context(),
    );
    assert_eq!(e1.protocol_version, CURRENT_PROTOCOL_VERSION);
    assert_eq!(e1.seq, 1);
    assert_eq!(e2.seq, 2);
    assert_eq!(e3.seq, 3);
}

#[test]
fn outgoing_seq_wrap_controller() {
    let mut seq = OutgoingSeq::new();
    let e1 = seq.wrap_controller(
        ControllerMessage::Pong(PongPayload {
            service_ts: 1,
            controller_ts: 2,
        }),
        current_trace_context(),
    );
    let e2 = seq.wrap_controller(
        ControllerMessage::Pong(PongPayload {
            service_ts: 3,
            controller_ts: 4,
        }),
        current_trace_context(),
    );
    assert_eq!(e1.protocol_version, CURRENT_PROTOCOL_VERSION);
    assert_eq!(e1.seq, 1);
    assert_eq!(e2.seq, 2);
}

#[test]
fn incoming_seq_accepts_sequential() {
    let mut seq = IncomingSeq::new();
    assert!(seq.validate(1).is_ok());
    assert!(seq.validate(2).is_ok());
    assert!(seq.validate(3).is_ok());
}

#[test]
fn incoming_seq_rejects_replay() {
    let mut seq = IncomingSeq::new();
    assert!(seq.validate(1).is_ok());
    let err = seq.validate(1).unwrap_err();
    assert_eq!(err.expected, 2);
    assert_eq!(err.received, 1);
}

#[test]
fn incoming_seq_rejects_skip() {
    let mut seq = IncomingSeq::new();
    let err = seq.validate(2).unwrap_err();
    assert_eq!(err.expected, 1);
    assert_eq!(err.received, 2);
}

#[test]
fn incoming_seq_rejects_zero() {
    let mut seq = IncomingSeq::new();
    let err = seq.validate(0).unwrap_err();
    assert_eq!(err.expected, 1);
    assert_eq!(err.received, 0);
}

#[test]
fn seq_error_display() {
    let err = SeqError {
        expected: 3,
        received: 5,
    };
    assert_eq!(err.to_string(), "sequence error: expected 3, received 5");
}

#[test]
fn outgoing_seq_default() {
    let mut seq = OutgoingSeq::default();
    let e = seq.wrap_service(
        ServiceMessage::Ping(PingPayload { service_ts: 1 }),
        current_trace_context(),
    );
    assert_eq!(e.protocol_version, CURRENT_PROTOCOL_VERSION);
    assert_eq!(e.seq, 1);
}

#[test]
fn incoming_seq_default() {
    let mut seq = IncomingSeq::default();
    assert!(seq.validate(1).is_ok());
}

// =========================================================================
// Timestamp serialization safety tests
// =========================================================================

#[test]
fn utc_datetime_millis_roundtrip_practical_range() {
    // Verify roundtrip for a practical timestamp (2024-01-28)
    let dt = UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap();
    let payload = CertificatePayload {
        cert_pem: "test".to_string(),
        not_after: dt,
    };
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.not_after, dt);
}

#[test]
fn utc_datetime_millis_roundtrip_epoch() {
    // Verify roundtrip for Unix epoch
    let dt = UtcDateTime::from_unix_timestamp(0).unwrap();
    let payload = CertificatePayload {
        cert_pem: "test".to_string(),
        not_after: dt,
    };
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.not_after, dt);
}

#[test]
fn utc_datetime_millis_roundtrip_far_future() {
    // Verify roundtrip for a far future date (year 9999)
    let dt = UtcDateTime::from_unix_timestamp(253_402_300_799).unwrap();
    let payload = CertificatePayload {
        cert_pem: "test".to_string(),
        not_after: dt,
    };
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.not_after, dt);
}

#[test]
fn utc_datetime_millis_roundtrip_negative_timestamp() {
    // Verify roundtrip for a negative timestamp (before Unix epoch)
    let dt = UtcDateTime::from_unix_timestamp(-1_000_000).unwrap();
    let payload = CertificatePayload {
        cert_pem: "test".to_string(),
        not_after: dt,
    };
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.not_after, dt);
}

// =========================================================================
// AsyncAPI spec-conformance tests
//
// Validate that serialized messages conform to the asyncapi.yaml schema.
// The spec is the source of truth for the wire protocol; these tests
// ensure Rust serde annotations stay in sync with it.
// =========================================================================

/// Minimal AsyncAPI schema validator for wire protocol tests.
struct AsyncApiSpec {
    schemas: serde_json::Map<String, serde_json::Value>,
}

impl AsyncApiSpec {
    fn load() -> Self {
        let yaml_str = include_str!("../asyncapi.yaml");
        let doc: serde_json::Value =
            serde_yaml_ng::from_str(yaml_str).expect("asyncapi.yaml should parse");
        let schemas = doc["components"]["schemas"]
            .as_object()
            .expect("components.schemas should be an object")
            .clone();
        Self { schemas }
    }

    /// Validate that a serialized JSON value conforms to the named schema.
    ///
    /// Checks:
    /// 1. Type discriminator (`const` field) matches
    /// 2. All required fields are present
    /// 3. Enum fields serialize to values in the spec's `enum` array
    fn validate(&self, schema_name: &str, json: &serde_json::Value) {
        let schema = self
            .schemas
            .get(schema_name)
            .unwrap_or_else(|| panic!("schema '{schema_name}' not found in asyncapi.yaml"));

        let obj = json
            .as_object()
            .unwrap_or_else(|| panic!("expected JSON object for schema '{schema_name}'"));

        // Check required fields
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for field in required {
                let field_name = field.as_str().unwrap();
                assert!(
                    obj.contains_key(field_name),
                    "schema '{schema_name}': required field '{field_name}' missing from \
                     serialized JSON.\nJSON: {json}"
                );
            }
        }

        // Check const and enum constraints on properties
        if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
            for (prop_name, prop_schema) in properties {
                if let Some(json_val) = obj.get(prop_name) {
                    // Check const
                    if let Some(const_val) = prop_schema.get("const") {
                        assert_eq!(
                            json_val, const_val,
                            "schema '{schema_name}': field '{prop_name}' should be \
                             const {const_val}, got {json_val}"
                        );
                    }

                    // Check enum
                    if let Some(enum_vals) = prop_schema.get("enum").and_then(|e| e.as_array()) {
                        assert!(
                            enum_vals.contains(json_val),
                            "schema '{schema_name}': field '{prop_name}' value {json_val} \
                             not in enum {enum_vals:?}"
                        );
                    }

                    // Check $ref to enum schemas
                    if let Some(ref_val) = prop_schema.get("$ref").and_then(|r| r.as_str()) {
                        let ref_schema_name = ref_val
                            .strip_prefix("#/components/schemas/")
                            .unwrap_or(ref_val);
                        if let Some(ref_schema) = self.schemas.get(ref_schema_name)
                            && let Some(enum_vals) =
                                ref_schema.get("enum").and_then(|e| e.as_array())
                        {
                            assert!(
                                enum_vals.contains(json_val),
                                "schema '{schema_name}': field '{prop_name}' value \
                                 {json_val} not in enum {ref_schema_name} {enum_vals:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Wrap a service message in an envelope and serialize to JSON value.
fn service_envelope_json(msg: ServiceMessage) -> serde_json::Value {
    let envelope = ServiceEnvelope {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        seq: 1,
        trace_context: TraceContext::generate(),
        pagination: None,
        message: msg,
    };
    serde_json::to_value(envelope).unwrap()
}

/// Wrap a controller message in an envelope and serialize to JSON value.
fn controller_envelope_json(msg: ControllerMessage) -> serde_json::Value {
    let envelope = ControllerEnvelope {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        seq: 1,
        trace_context: TraceContext::generate(),
        message: msg,
    };
    serde_json::to_value(envelope).unwrap()
}

// ── ServiceMessage spec conformance ─────────────────────────────

#[test]
fn spec_conformance_ping() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::Ping(PingPayload {
        service_ts: 1706400000000,
    }));
    spec.validate("pingPayload", &json);
}

#[test]
fn spec_conformance_enroll() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::Enroll(EnrollPayload {
        hostname: "node-1".to_string(),
        friendly_name: "Node One".to_string(),
        enrollment_token: Some(SecretString::new("tok-123")),
        capabilities: agent_capabilities(),
        service_app_name: "uptrakit-agent".to_string(),
    }));
    spec.validate("enrollPayload", &json);
}

#[test]
fn spec_conformance_request_certificate() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::RequestCertificate(
        RequestCertificatePayload {
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
        },
    ));
    spec.validate("requestCertificatePayload", &json);
}

#[test]
fn spec_conformance_renew_certificate() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::RenewCertificate(RenewCertificatePayload {
        csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\nrenew\n-----END CERTIFICATE REQUEST-----\n"
            .to_string(),
    }));
    spec.validate("renewCertificatePayload", &json);
}

#[test]
fn spec_conformance_report_hosts() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "machine-42".to_string(),
            os_type: Some("linux".to_string()),
            os_version: Some("Ubuntu 24.04 LTS".to_string()),
            architecture: Some("x86_64".to_string()),
            hostname: Some("web-01.example.com".to_string()),
            ip_address: Some("10.0.0.5".to_string()),
            agent_host_id: None,
            features: None,
        }],
        agent_version: "0.0.1".to_string(),
        capabilities: [Capability::SoftwareDiscovery, Capability::GracefulShutdown]
            .into_iter()
            .collect(),
    }));
    spec.validate("reportHostsPayload", &json);
}

#[test]
fn spec_conformance_version_check_results() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::VersionCheckResults(
        VersionCheckResultsPayload {
            results: vec![VersionCheckResult {
                software_item_id: TEST_UUID_1,
                host_software_item_id: None,
                installed_version: Some("1.2.3".to_string()),
                installed_display_version: None,
                latest_version: Some("1.3.0".to_string()),
                error: None,
                update_category: UpdateCategory::Unknown,
            }],
        },
    ));
    spec.validate("versionCheckResultsPayload", &json);
}

#[test]
fn spec_conformance_update_started() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::UpdateStarted(UpdateStartedPayload {
        update_history_id: TEST_UUID_1,
        from_version: Some("1.0.0".to_string()),
        interactive: false,
    }));
    spec.validate("updateStartedPayload", &json);
}

#[test]
fn spec_conformance_update_output() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::UpdateOutput(UpdateOutputPayload {
        update_history_id: TEST_UUID_1,
        output: "Downloading package...".to_string(),
        stream: OutputStreamType::Stdout,
    }));
    spec.validate("updateOutputPayload", &json);
}

#[test]
fn spec_conformance_update_result() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::UpdateResult(UpdateResultPayload {
        update_history_id: TEST_UUID_1,
        status: UpdateFinalStatus::Completed,
        from_version: Some("1.0.0".to_string()),
        to_version: Some("2.0.0".to_string()),
        output: "Update completed successfully".to_string(),
        error: None,
    }));
    spec.validate("updateResultPayload", &json);
}

#[test]
fn spec_conformance_disconnecting() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::Disconnecting(DisconnectingPayload::new(
        DisconnectReason::Shutdown,
    )));
    spec.validate("disconnectingPayload", &json);
}

// ── ControllerMessage spec conformance ──────────────────────────

#[test]
fn spec_conformance_pong() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::Pong(PongPayload {
        service_ts: 1706400000000,
        controller_ts: 1706400000050,
    }));
    spec.validate("pongPayload", &json);
}

#[test]
fn spec_conformance_enrolled() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::Enrolled(EnrolledPayload {
        service_id: TEST_UUID_1,
        enrollment_secret: SecretString::new("secret-abc"),
        status: EnrollmentStatus::Pending,
    }));
    spec.validate("enrolledPayload", &json);
}

#[test]
fn spec_conformance_approved() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::Approved(ApprovedPayload {
        service_id: TEST_UUID_1,
    }));
    spec.validate("approvedPayload", &json);
}

#[test]
fn spec_conformance_rejected() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::Rejected(RejectedPayload {
        service_id: TEST_UUID_1,
    }));
    spec.validate("rejectedPayload", &json);
}

#[test]
fn spec_conformance_certificate() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::Certificate(CertificatePayload {
        cert_pem: "-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----\n".to_string(),
        not_after: UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap(),
    }));
    spec.validate("certificatePayload", &json);
}

#[test]
fn spec_conformance_error() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::Error(ErrorPayload {
        code: ErrorCode::EnrollmentFailed,
        message: "The enrollment token is invalid".to_string(),
    }));
    spec.validate("errorPayload", &json);
}

#[test]
fn spec_conformance_service_settings() {
    let spec = AsyncApiSpec::load();
    let json =
        controller_envelope_json(ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "abc123".to_string(),
            capabilities: [
                Capability::SoftwareDiscovery,
                Capability::UpdateHooks,
                Capability::GracefulShutdown,
                Capability::UpdateTracking,
                Capability::SshRemote,
            ]
            .into_iter()
            .collect(),
            report_page_limits: ReportPageLimits::default(),
            shutdown_timeout: Some(std::time::Duration::from_secs(120)),
            ping_interval: std::time::Duration::from_secs(300),
            tenant_id: Some(TEST_UUID_1),
        }));
    spec.validate("serviceSettingsPayload", &json);
}

#[test]
fn spec_conformance_ca_bundle_updated() {
    let spec = AsyncApiSpec::load();
    let json =
        controller_envelope_json(ControllerMessage::CaBundleUpdated(CaBundleUpdatedPayload {
            ca_bundle_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n"
                .to_string(),
        }));
    spec.validate("caBundleUpdatedPayload", &json);
}

#[test]
fn spec_conformance_request_cert_renewal() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::RequestCertRenewal(
        RequestCertRenewalPayload {
            reason: "CA rotation after backend URL change".to_string(),
        },
    ));
    spec.validate("requestCertRenewalPayload", &json);
}

#[test]
fn spec_conformance_server_restarting() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::ServerRestarting(
        ServerRestartingPayload {
            reason: "controller restarting for upgrade".to_string(),
        },
    ));
    spec.validate("serverRestartingPayload", &json);
}

#[test]
fn spec_conformance_check_versions() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::CheckVersions(CheckVersionsPayload {
        host_machine_id: "test-machine-id".to_string(),
        assignments: vec![VersionCheckAssignment {
            software_item_id: TEST_UUID_1,
            name: "Test Software".to_string(),
            detect_version: Some(PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "octocat/hello-world".to_string(),
                config: serde_json::json!({}),
            }),
            fetch_releases: None,
            host_software_item_id: None,
        }],
    }));
    spec.validate("checkVersionsPayload", &json);
}

#[test]
fn spec_conformance_execute_update() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::ExecuteUpdate(Box::new(
        ExecuteUpdatePayload {
            host_machine_id: "test-machine-id".to_string(),
            update_history_id: TEST_UUID_1,
            software_item_id: TEST_UUID_2,
            software_item_name: "Node.js".to_string(),
            to_version: "20.10.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "nodejs/node".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            release_info: None,
            timeout: DEFAULT_UPDATE_TIMEOUT,
            interactive: false,
        },
    )));
    spec.validate("executeUpdatePayload", &json);
}

// =========================================================================
// Autodiscovery wire message tests
// =========================================================================

#[test]
fn discover_software_payload_roundtrip() {
    let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
        host_machine_id: "machine-abc".to_string(),
        plugins: vec![
            DiscoveryPluginAssignment {
                plugin_config_id: Some(TEST_UUID_1),
                plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                config: serde_json::json!({"package_type": "formula"}),
            },
            DiscoveryPluginAssignment {
                plugin_config_id: None,
                plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
                config: serde_json::Value::Object(Default::default()),
            },
        ],
    });
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn discover_software_payload_type_tag() {
    let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
        host_machine_id: "machine-abc".to_string(),
        plugins: vec![],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"discover_software""#));
}

#[test]
fn discovery_results_payload_roundtrip() {
    let msg = ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
        host_machine_id: "machine-abc".to_string(),
        results: vec![
            DiscoveryPluginResult {
                plugin_config_id: Some(TEST_UUID_1),
                plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                discoveries: vec![DiscoveredSoftware {
                    package_identifier: "wget".to_string(),
                    name: "Wget".to_string(),
                    installed_version: "1.21.4".to_string(),
                    targets: vec![],
                    extra: Some(serde_json::json!({"package_type": "formula"})),
                    featured: false,
                    qualifier: None,
                    plugin_package_identifier: None,
                    installed_display_version: None,
                }],
                error: None,
            },
            DiscoveryPluginResult {
                plugin_config_id: None,
                plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
                discoveries: vec![],
                error: Some("no update script found".to_string()),
            },
        ],
    });
    let json = serde_json::to_string(&msg).unwrap();
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn discovery_results_payload_type_tag() {
    let msg = ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
        host_machine_id: "machine-abc".to_string(),
        results: vec![],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"discovery_results""#));
}

#[test]
fn discovery_plugin_assignment_none_config_id_omitted() {
    let assignment = DiscoveryPluginAssignment {
        plugin_config_id: None,
        plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
        config: serde_json::Value::Object(Default::default()),
    };
    let json = serde_json::to_value(&assignment).unwrap();
    assert!(!json.as_object().unwrap().contains_key("plugin_config_id"));
}

#[test]
fn spec_conformance_discover_software() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::DiscoverSoftware(
        DiscoverSoftwarePayload {
            host_machine_id: "machine-abc".to_string(),
            plugins: vec![DiscoveryPluginAssignment {
                plugin_config_id: Some(TEST_UUID_1),
                plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                config: serde_json::json!({"package_type": "formula"}),
            }],
        },
    ));
    spec.validate("discoverSoftwarePayload", &json);
}

#[test]
fn spec_conformance_discovery_results() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
        host_machine_id: "machine-abc".to_string(),
        results: vec![DiscoveryPluginResult {
            plugin_config_id: Some(TEST_UUID_1),
            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
            discoveries: vec![DiscoveredSoftware {
                package_identifier: "wget".to_string(),
                name: "Wget".to_string(),
                installed_version: "1.21.4".to_string(),
                targets: vec![],
                extra: None,
                featured: false,
                qualifier: None,
                plugin_package_identifier: None,
                installed_display_version: None,
            }],
            error: None,
        }],
    }));
    spec.validate("discoveryResultsPayload", &json);
}

#[test]
fn execute_batch_update_serialization_roundtrip() {
    let msg = ControllerMessage::ExecuteBatchUpdate(Box::new(ExecuteBatchUpdatePayload {
        host_machine_id: "test-machine-id".to_string(),
        batch_id: TEST_UUID_1,
        plugin_type: plugin_ids::PACKAGE_MANAGER_APT.clone(),
        plugin_config: serde_json::json!({}),
        updates: vec![BatchUpdateItem {
            host_software_item_id: TEST_UUID_1,
            update_history_id: TEST_UUID_2,
            package_identifier: "nginx".to_string(),
            to_version: "1.24.0-2".to_string(),
            release_info: None,
        }],
        pre_update_hook_plugins: vec![],
        post_update_hook_plugins: vec![],
        timeout: std::time::Duration::from_secs(7200),
        interactive: false,
    }));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"execute_batch_update"#));
    assert!(json.contains(r#""plugin_type":"package_manager_apt"#));
    assert!(json.contains(r#""package_identifier":"nginx"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn execute_batch_update_backward_compat_old_type_tag() {
    // Old wire messages with the old type tag must still deserialize.
    let json = r#"{"type":"execute_batch_host_package_update","host_machine_id":"test","batch_id":"550e8400-e29b-41d4-a716-446655440000","plugin_type":"package_manager_apt","plugin_config":{},"updates":[{"host_package_id":"550e8400-e29b-41d4-a716-446655440000","update_history_id":"550e8400-e29b-41d4-a716-446655440001","package_identifier":"nginx","to_version":"1.24.0-2"}],"timeout_seconds":7200}"#;
    let msg: ControllerMessage = serde_json::from_str(json).unwrap();
    if let ControllerMessage::ExecuteBatchUpdate(payload) = msg {
        assert_eq!(payload.updates[0].host_software_item_id, TEST_UUID_1);
    } else {
        panic!("expected ExecuteBatchUpdate");
    }
}

#[test]
fn batch_update_result_serialization_roundtrip() {
    let msg = ServiceMessage::BatchUpdateResult(BatchUpdateResultPayload {
        batch_id: TEST_UUID_1,
        results: vec![BatchUpdateItemResult {
            host_software_item_id: TEST_UUID_1,
            update_history_id: TEST_UUID_2,
            status: UpdateFinalStatus::Completed,
            output: "Unpacking nginx 1.24.0-2 ...\n".to_string(),
            installed_version: Some("1.24.0-2".to_string()),
            error: None,
        }],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"batch_update_result"#));
    assert!(json.contains(r#""status":"completed"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn batch_update_result_backward_compat_old_type_tag() {
    // Old wire messages with the old type tag must still deserialize.
    let json = r#"{"type":"batch_host_package_update_result","batch_id":"550e8400-e29b-41d4-a716-446655440000","results":[{"host_package_id":"550e8400-e29b-41d4-a716-446655440000","update_history_id":"550e8400-e29b-41d4-a716-446655440001","status":"completed","output":"done\n"}]}"#;
    let msg: ServiceMessage = serde_json::from_str(json).unwrap();
    if let ServiceMessage::BatchUpdateResult(payload) = msg {
        assert_eq!(payload.results[0].host_software_item_id, TEST_UUID_1);
    } else {
        panic!("expected BatchUpdateResult");
    }
}

#[test]
fn spec_conformance_execute_batch_update() {
    let spec = AsyncApiSpec::load();
    let json = controller_envelope_json(ControllerMessage::ExecuteBatchUpdate(Box::new(
        ExecuteBatchUpdatePayload {
            host_machine_id: "test-machine-id".to_string(),
            batch_id: TEST_UUID_1,
            plugin_type: plugin_ids::PACKAGE_MANAGER_APT.clone(),
            plugin_config: serde_json::json!({}),
            updates: vec![BatchUpdateItem {
                host_software_item_id: TEST_UUID_1,
                update_history_id: TEST_UUID_2,
                package_identifier: "nginx".to_string(),
                to_version: "1.24.0-2".to_string(),
                release_info: None,
            }],
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            timeout: std::time::Duration::from_secs(7200),
            interactive: false,
        },
    )));
    spec.validate("executeBatchUpdatePayload", &json);
}

#[test]
fn spec_conformance_batch_update_result() {
    let spec = AsyncApiSpec::load();
    let json = service_envelope_json(ServiceMessage::BatchUpdateResult(
        BatchUpdateResultPayload {
            batch_id: TEST_UUID_1,
            results: vec![BatchUpdateItemResult {
                host_software_item_id: TEST_UUID_1,
                update_history_id: TEST_UUID_2,
                status: UpdateFinalStatus::Completed,
                output: "Unpacking nginx 1.24.0-2 ...\n".to_string(),
                installed_version: Some("1.24.0-2".to_string()),
                error: None,
            }],
        },
    ));
    spec.validate("batchUpdateResultPayload", &json);
}

// =========================================================================
// Host summary MQTT types tests
// =========================================================================

#[test]
fn mqtt_host_summary_roundtrip() {
    let state = HostPackageSummary {
        host_id: TEST_UUID_1,
        hostname: "myserver.local".to_string(),
        friendly_name: "My Server".to_string(),
        pending_count: 3,
        security_pending_count: 1,
        total_count: 42,
        bugfix_count: 2,
        feature_count: 1,
        update_in_progress: false,
    };
    let json = serde_json::to_string(&state).unwrap();
    let deserialized: HostPackageSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, state);
}

#[test]
fn mqtt_software_states_payload_default_host_summaries() {
    // Deserializing a payload without host_summaries should default to empty vec.
    let json = r#"{"tenant_id":"11111111-1111-1111-1111-111111111111","items":[],"page":{"page_index":0,"total_pages":1}}"#;
    let payload: SoftwareStatesPayload = serde_json::from_str(json).unwrap();
    assert!(payload.host_summaries.is_empty());
}

#[test]
fn mqtt_software_states_payload_backward_compat_host_package_hosts() {
    // Old wire messages with "host_package_hosts" should deserialize via alias.
    let json = r#"{"tenant_id":"11111111-1111-1111-1111-111111111111","items":[],"host_package_hosts":[{"host_id":"550e8400-e29b-41d4-a716-446655440001","hostname":"host1","pending_count":5,"security_pending_count":2,"total_count":100,"update_in_progress":true}],"page":{"page_index":0,"total_pages":1}}"#;
    let payload: SoftwareStatesPayload = serde_json::from_str(json).unwrap();
    assert_eq!(payload.host_summaries.len(), 1);
    assert_eq!(payload.host_summaries[0].pending_count, 5);
}

#[test]
fn mqtt_software_states_payload_with_host_summaries_roundtrip() {
    let payload = SoftwareStatesPayload {
        tenant_id: TEST_UUID_1,
        items: vec![],
        host_summaries: vec![HostPackageSummary {
            host_id: TEST_UUID_2,
            hostname: "host1".to_string(),
            friendly_name: "Host 1".to_string(),
            pending_count: 5,
            security_pending_count: 2,
            total_count: 100,
            bugfix_count: 3,
            feature_count: 2,
            update_in_progress: true,
        }],
        hosts: vec![],
        page: SoftwareStatesPage {
            page_index: 0,
            total_pages: 1,
        },
    };
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: SoftwareStatesPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, payload);
}

#[test]
fn service_trigger_host_batch_update_roundtrip() {
    let msg = ServiceMessage::ServiceTriggerHostBatchUpdate(ServiceHostBatchUpdateTriggerPayload {
        tenant_id: TEST_UUID_1,
        host_id: TEST_UUID_2,
        actor_service_id: TEST_UUID_3,
        security_only: false,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"service_trigger_host_batch_update""#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_trigger_host_batch_update_security_only_roundtrip() {
    let msg = ServiceMessage::ServiceTriggerHostBatchUpdate(ServiceHostBatchUpdateTriggerPayload {
        tenant_id: TEST_UUID_1,
        host_id: TEST_UUID_2,
        actor_service_id: TEST_UUID_3,
        security_only: true,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""security_only":true"#));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_trigger_host_batch_update_security_only_defaults_false() {
    // When `security_only` is absent in wire messages, it should deserialize as false.
    let json = r#"{"protocol_version":1,"seq":1,"type":"service_trigger_host_batch_update","tenant_id":"11111111-1111-1111-1111-111111111111","host_id":"22222222-2222-2222-2222-222222222222","actor_service_id":"33333333-3333-3333-3333-333333333333"}"#;
    let msg: ServiceMessage = serde_json::from_str(json).unwrap();
    if let ServiceMessage::ServiceTriggerHostBatchUpdate(p) = msg {
        assert!(!p.security_only);
    } else {
        panic!("expected ServiceTriggerHostBatchUpdate");
    }
}

// =========================================================================
// Capability enum tests
// =========================================================================

#[test]
fn capability_serde_known_variants() {
    let cases = [
        (Capability::SoftwareDiscovery, "software_discovery"),
        (Capability::UpdateHooks, "update_hooks"),
        (Capability::GracefulShutdown, "graceful_shutdown"),
        (Capability::UpdateTracking, "update_tracking"),
        (Capability::SshRemote, "ssh_remote"),
    ];
    for (variant, wire_str) in &cases {
        let json = serde_json::to_string(variant).unwrap();
        assert_eq!(json, format!(r#""{wire_str}""#));
        let deserialized: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(&deserialized, variant);
    }
}

#[test]
fn capability_other_roundtrip() {
    let cap: Capability = serde_json::from_str(r#""future_capability_xyz""#).unwrap();
    assert_eq!(cap, Capability::Other("future_capability_xyz".to_string()));
    let json = serde_json::to_string(&cap).unwrap();
    assert_eq!(json, r#""future_capability_xyz""#);
}

#[test]
fn capability_display_matches_serde() {
    for cap in [
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
        Capability::GracefulShutdown,
        Capability::UpdateTracking,
        Capability::SshRemote,
    ] {
        let serde_str = serde_json::to_value(&cap).unwrap();
        assert_eq!(
            cap.to_string(),
            serde_str.as_str().unwrap(),
            "Display must match serde for {cap:?}"
        );
    }
}

#[test]
fn capability_ordering() {
    // BTreeSet should produce a stable sorted order for capabilities.
    let set: BTreeSet<Capability> = [
        Capability::SshRemote,
        Capability::GracefulShutdown,
        Capability::SoftwareDiscovery,
    ]
    .into_iter()
    .collect();
    let mut iter = set.into_iter();
    // Alphabetical by wire string: graceful_shutdown < software_discovery < ssh_remote
    assert_eq!(iter.next(), Some(Capability::GracefulShutdown));
    assert_eq!(iter.next(), Some(Capability::SoftwareDiscovery));
    assert_eq!(iter.next(), Some(Capability::SshRemote));
}

#[test]
fn capability_is_known() {
    assert!(Capability::SoftwareDiscovery.is_known());
    assert!(Capability::UpdateHooks.is_known());
    assert!(Capability::GracefulShutdown.is_known());
    assert!(Capability::UpdateTracking.is_known());
    assert!(Capability::SshRemote.is_known());
    assert!(!Capability::Other("future".to_string()).is_known());
}

#[test]
fn report_hosts_empty_capabilities_omitted() {
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: vec![HostInfo {
            machine_id: "m-1".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            hostname: None,
            ip_address: None,
            agent_host_id: None,
            features: None,
        }],
        agent_version: "0.0.1".to_string(),
        capabilities: BTreeSet::new(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(
        !json.contains("capabilities"),
        "empty capabilities should be omitted"
    );
    // Deserializes back with empty set.
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn capability_intersection_excludes_other() {
    let controller_caps: BTreeSet<Capability> = [
        Capability::SoftwareDiscovery,
        Capability::GracefulShutdown,
        Capability::UpdateTracking,
    ]
    .into_iter()
    .collect();
    let service_caps: BTreeSet<Capability> = [
        Capability::SoftwareDiscovery,
        Capability::GracefulShutdown,
        Capability::Other("new_cap".to_string()),
    ]
    .into_iter()
    .collect();
    let agreed: BTreeSet<Capability> = controller_caps
        .intersection(&service_caps)
        .filter(|c| c.is_known())
        .cloned()
        .collect();
    assert_eq!(
        agreed,
        [Capability::SoftwareDiscovery, Capability::GracefulShutdown]
            .into_iter()
            .collect()
    );
}

// =========================================================================
// New capability variants
// =========================================================================

#[test]
fn scheduler_capability_roundtrip() {
    let cap = Capability::Scheduler;
    assert_eq!(cap.as_str(), "scheduler");
    assert!(cap.is_known());
    let json = serde_json::to_string(&cap).unwrap();
    assert_eq!(json, r#""scheduler""#);
    let parsed: Capability = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, cap);
}

#[test]
fn database_access_capability_roundtrip() {
    let cap = Capability::DatabaseAccess;
    assert_eq!(cap.as_str(), "database_access");
    assert!(cap.is_known());
    let parsed: Capability = "database_access".parse().unwrap();
    assert_eq!(parsed, cap);
}

#[test]
fn nats_access_capability_roundtrip() {
    let cap = Capability::NatsAccess;
    assert_eq!(cap.as_str(), "nats_access");
    let parsed: Capability = "nats_access".parse().unwrap();
    assert_eq!(parsed, cap);
}

#[test]
fn master_key_access_capability_roundtrip() {
    let cap = Capability::MasterKeyAccess;
    assert_eq!(cap.as_str(), "master_key_access");
    let parsed: Capability = "master_key_access".parse().unwrap();
    assert_eq!(parsed, cap);
}

#[test]
fn ca_management_capability_roundtrip() {
    let cap = Capability::CaManagement;
    assert_eq!(cap.as_str(), "ca_management");
    let parsed: Capability = "ca_management".parse().unwrap();
    assert_eq!(parsed, cap);
}

#[test]
fn system_service_capability_roundtrip() {
    let cap = Capability::SystemService;
    assert_eq!(cap.as_str(), "system_service");
    assert!(cap.is_known());
    let json = serde_json::to_string(&cap).unwrap();
    assert_eq!(json, r#""system_service""#);
    let parsed: Capability = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, cap);
    let from_str: Capability = "system_service".parse().unwrap();
    assert_eq!(from_str, cap);
}

#[test]
fn reset_data_capability_roundtrip() {
    let cap = Capability::ResetData;
    assert_eq!(cap.as_str(), "reset_data");
    assert!(cap.is_known());
    let json = serde_json::to_string(&cap).unwrap();
    assert_eq!(json, r#""reset_data""#);
    let parsed: Capability = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, cap);
    let from_str: Capability = "reset_data".parse().unwrap();
    assert_eq!(from_str, cap);
}

// =========================================================================
// ServiceCredentials and RequestCaRotation payloads
// =========================================================================

#[test]
fn service_credentials_serialization_roundtrip() {
    let msg = ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
        db_url: Some(SecretString::new("postgres://localhost/uptrakit")),
        master_key_hex: Some(SecretString::new("aa".repeat(32))),
        nats_url: Some("nats://localhost:4222".to_string()),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"service_credentials"#));
    assert!(json.contains(r#""db_url":"#));
    assert!(json.contains(r#""nats_url":"#));
    assert!(json.contains(r#""master_key_hex":"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn service_credentials_omits_none_fields() {
    let msg = ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
        db_url: Some(SecretString::new("sqlite://test.db")),
        master_key_hex: None,
        nats_url: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""db_url":"#));
    assert!(!json.contains("master_key_hex"));
    assert!(!json.contains("nats_url"));
}

#[test]
fn request_ca_rotation_serialization_roundtrip() {
    let msg = ControllerMessage::RequestCaRotation(RequestCaRotationPayload {
        reason: "CA certificate expiring in 30 days".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"request_ca_rotation"#));
    assert!(json.contains(r#""reason":"CA certificate expiring in 30 days"#));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

// ── is_nats_publishable ───────────────────────────────────────────────────

#[test]
fn is_nats_publishable_blocks_credential_bearing_variants() {
    // ServiceCredentials must never be published to NATS.
    assert!(
        !ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
            db_url: Some(SecretString::new("postgres://localhost/db")),
            master_key_hex: None,
            nats_url: None,
        })
        .is_nats_publishable()
    );

    // Session-targeted surface action variants must not be published to NATS.
    assert!(
        !ControllerMessage::SurfaceActionRequest(test_surface_action_request())
            .is_nats_publishable()
    );
    assert!(
        !ControllerMessage::SurfaceActionResponse(surfaces::SurfaceActionResponse {
            request_id: TEST_UUID_1,
            success: true,
            result: None,
            error: None,
        })
        .is_nats_publishable()
    );
    assert!(
        !ControllerMessage::SurfaceActionCancel(surfaces::SurfaceActionCancel {
            request_id: TEST_UUID_1,
            target_provider_id: "uptrakit-agent-ssh".to_string(),
            reason: surfaces::SurfaceActionCancelReason::Timeout,
        })
        .is_nats_publishable()
    );
}

#[test]
fn is_nats_publishable_allows_non_credential_variants() {
    // Ordinary messages must be publishable.
    assert!(
        ControllerMessage::Pong(PongPayload {
            service_ts: 0,
            controller_ts: 0,
        })
        .is_nats_publishable()
    );

    assert!(
        ControllerMessage::Approved(ApprovedPayload {
            service_id: TEST_UUID_1,
        })
        .is_nats_publishable()
    );

    assert!(
        ControllerMessage::RequestCaRotation(RequestCaRotationPayload {
            reason: "test".into(),
        })
        .is_nats_publishable()
    );
}

// ── SetUpdateFreeze tests ────────────────────────────────────────────

#[test]
fn set_update_freeze_serialization_roundtrip() {
    let msg = ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
        enabled: true,
        reason: Some("emergency maintenance".to_string()),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"set_update_freeze"#));
    assert!(json.contains(r#""enabled":true"#));
    assert!(json.contains(r#""reason":"emergency maintenance""#));
    let roundtripped: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, roundtripped);
}

#[test]
fn set_update_freeze_disable_without_reason() {
    let msg = ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
        enabled: false,
        reason: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("reason"));
    let roundtripped: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, roundtripped);
}

#[test]
fn set_update_freeze_is_nats_publishable() {
    let msg = ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
        enabled: true,
        reason: None,
    });
    assert!(
        msg.is_nats_publishable(),
        "SetUpdateFreeze contains no credentials and should be NATS-publishable"
    );
}

// ── ResetData tests ─────────────────────────────────────────────────────

#[test]
fn reset_data_serialization_roundtrip() {
    let msg = ControllerMessage::ResetData;
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"type":"reset_data"}"#);
    let roundtripped: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, roundtripped);
}

#[test]
fn reset_data_is_not_nats_publishable() {
    assert!(
        !ControllerMessage::ResetData.is_nats_publishable(),
        "ResetData is an internal broadcast and must not be published to NATS"
    );
}

#[test]
fn report_plugin_config_roundtrip() {
    let msg = ServiceMessage::ReportPluginConfig(ReportPluginConfigPayload {
        request_id: "req-42".to_string(),
        plugin_type: "infrastructure_proxmox".to_string(),
        name: "pve.local".to_string(),
        config: serde_json::json!({"api_url": "https://pve:8006", "api_token": "test"}),
    });
    let json = serde_json::to_string(&msg).expect("serialize");
    let parsed: ServiceMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, parsed);
}

#[test]
fn report_plugin_config_response_roundtrip() {
    let msg = ControllerMessage::ReportPluginConfigResponse(ReportPluginConfigResponsePayload {
        request_id: "req-42".to_string(),
        success: true,
        plugin_config_id: Some(Uuid::nil()),
        error: None,
    });
    let json = serde_json::to_string(&msg).expect("serialize");
    let parsed: ControllerMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, parsed);
}

#[test]
fn report_plugin_config_response_is_nats_publishable() {
    let msg = ControllerMessage::ReportPluginConfigResponse(ReportPluginConfigResponsePayload {
        request_id: "req-1".to_string(),
        success: true,
        plugin_config_id: None,
        error: None,
    });
    assert!(
        msg.is_nats_publishable(),
        "ReportPluginConfigResponse contains no credentials and should be NATS-publishable"
    );
}

fn test_surface_registration() -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "uptrakit-agent-ssh".to_string(),
            provider_kind: surfaces::ProviderKind::Service,
            provider_namespace: "uptrakit.agent.ssh".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::SectionNode,
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: Some(TEST_UUID_1.to_string()),
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                label: "SSH Guest Panel".to_string(),
                priority: 100,
                slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
                scope: surfaces::Scope::Tenant,
                targeting: surfaces::Targeting::Universal,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::Service,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::SectionNode,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::Section {
                    title: Some("Guests".to_string()),
                    children: vec![surfaces::SurfaceNode::TextBlock {
                        text: "Guest overview".to_string(),
                    }],
                },
            },
            interactions: vec![],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

fn test_surface_action_request() -> surfaces::SurfaceActionRequest {
    surfaces::SurfaceActionRequest {
        request_id: TEST_UUID_2,
        tenant_id: TEST_UUID_1.to_string(),
        surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
        interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
        idempotency_key: "idem-1".to_string(),
        target_provider_id: Some("uptrakit-agent-ssh".to_string()),
        caller_origin: surfaces::CallerOrigin::Provider {
            provider_id: "uptrakit-agent-ssh".to_string(),
        },
        params: serde_json::json!({"host_machine_id":"m-1"})
            .as_object()
            .unwrap()
            .clone(),
        encrypted_sensitive_params: None,
    }
}

#[test]
fn service_surface_registration_roundtrip() {
    let msg = ServiceMessage::SurfaceRegistration(test_surface_registration());
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "surface_registration");
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn service_surface_action_request_roundtrip() {
    let msg = ServiceMessage::SurfaceActionRequest(test_surface_action_request());
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "surface_action_request");
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn service_surface_action_response_roundtrip() {
    let msg = ServiceMessage::SurfaceActionResponse(surfaces::SurfaceActionResponse {
        request_id: TEST_UUID_2,
        success: true,
        result: Some(serde_json::json!({"status":"ok"})),
        error: None,
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "surface_action_response");
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn controller_surface_action_request_roundtrip() {
    let msg = ControllerMessage::SurfaceActionRequest(test_surface_action_request());
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "surface_action_request");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn controller_surface_action_cancel_roundtrip() {
    let msg = ControllerMessage::SurfaceActionCancel(surfaces::SurfaceActionCancel {
        request_id: TEST_UUID_2,
        target_provider_id: "uptrakit-agent-ssh".to_string(),
        reason: surfaces::SurfaceActionCancelReason::RequestCancelled,
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "surface_action_cancel");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn controller_surface_action_response_roundtrip() {
    let msg = ControllerMessage::SurfaceActionResponse(surfaces::SurfaceActionResponse {
        request_id: TEST_UUID_2,
        success: false,
        result: None,
        error: Some(surfaces::SurfaceActionError {
            code: surfaces::SurfaceActionErrorCode::InvalidRequest,
            message: "invalid params".to_string(),
            details: None,
        }),
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "surface_action_response");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn surface_action_request_wire_validation_rejects_long_idempotency_key() {
    let mut request = test_surface_action_request();
    request.idempotency_key = "x".repeat(MAX_SHORT_STRING_LEN + 1);
    let msg = ServiceMessage::SurfaceActionRequest(request);

    let err = msg
        .wire_validate()
        .expect_err("idempotency key should fail");
    assert_eq!(err.field, "idempotency_key");
}

// =========================================================================
// Workload claim protocol tests
// =========================================================================

#[test]
fn workload_claim_serialization_roundtrip() {
    use std::collections::BTreeMap;
    let mut claims = BTreeMap::new();
    claims.insert("clients.aaa".to_string(), TEST_UUID_1);
    claims.insert("clients.bbb".to_string(), TEST_UUID_2);
    let msg = ServiceMessage::WorkloadClaim(WorkloadClaimPayload::new(claims));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_claim");
    assert!(json["claims"]["clients.aaa"].is_string());
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_empty_claims() {
    use std::collections::BTreeMap;
    let msg = ServiceMessage::WorkloadClaim(WorkloadClaimPayload::new(BTreeMap::new()));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_claim");
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_result_serialization_roundtrip() {
    let msg = ControllerMessage::WorkloadClaimResult(WorkloadClaimResultPayload::new(
        ["clients.aaa".to_string()].into(),
        ["clients.bbb".to_string()].into(),
    ));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_claim_result");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_result_empty_sets_omitted() {
    let msg = ControllerMessage::WorkloadClaimResult(WorkloadClaimResultPayload::new(
        BTreeSet::new(),
        BTreeSet::new(),
    ));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("granted"));
    assert!(!json.contains("rejected"));
    let roundtripped: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_release_serialization_roundtrip() {
    let msg = ServiceMessage::WorkloadRelease(WorkloadReleasePayload::new(
        ["clients.aaa".to_string()].into(),
    ));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_release");
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_announcement_roundtrip() {
    use std::collections::BTreeMap;
    let mut claimed = BTreeMap::new();
    claimed.insert("clients.aaa".to_string(), TEST_UUID_1);
    let msg = ControllerMessage::WorkloadClaimAnnouncement(WorkloadClaimAnnouncementPayload::new(
        TEST_UUID_1,
        TEST_UUID_2,
        claimed,
        ["clients.old".to_string()].into(),
        "2026-03-16T12:00:00Z".to_string(),
    ));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_claim_announcement");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_announcement_is_nats_publishable() {
    use std::collections::BTreeMap;
    let msg = ControllerMessage::WorkloadClaimAnnouncement(WorkloadClaimAnnouncementPayload::new(
        TEST_UUID_1,
        TEST_UUID_2,
        BTreeMap::new(),
        BTreeSet::new(),
        "2026-03-16T12:00:00Z".to_string(),
    ));
    assert!(msg.is_nats_publishable());
}

#[test]
fn workload_claim_result_is_not_nats_publishable() {
    let msg = ControllerMessage::WorkloadClaimResult(WorkloadClaimResultPayload::new(
        BTreeSet::new(),
        BTreeSet::new(),
    ));
    assert!(!msg.is_nats_publishable());
}

#[test]
fn workload_claim_sync_request_roundtrip() {
    let msg = ControllerMessage::WorkloadClaimSyncRequest(WorkloadClaimSyncRequestPayload::new(
        TEST_UUID_1,
    ));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_claim_sync_request");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_sync_response_roundtrip() {
    use std::collections::BTreeMap;
    let mut claims = BTreeMap::new();
    claims.insert(
        "clients.aaa".to_string(),
        WorkloadClaimSyncEntry::new(TEST_UUID_1, TEST_UUID_2, "2026-03-16T12:00:00Z".to_string()),
    );
    let msg = ControllerMessage::WorkloadClaimSyncResponse(WorkloadClaimSyncResponsePayload::new(
        TEST_UUID_3,
        claims,
    ));
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "workload_claim_sync_response");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn workload_claim_sync_request_is_nats_publishable() {
    let msg = ControllerMessage::WorkloadClaimSyncRequest(WorkloadClaimSyncRequestPayload::new(
        TEST_UUID_1,
    ));
    assert!(msg.is_nats_publishable());
}

#[test]
fn workload_claim_sync_response_is_nats_publishable() {
    use std::collections::BTreeMap;
    let msg = ControllerMessage::WorkloadClaimSyncResponse(WorkloadClaimSyncResponsePayload::new(
        TEST_UUID_1,
        BTreeMap::new(),
    ));
    assert!(msg.is_nats_publishable());
}

#[test]
fn workload_claims_capability_roundtrip() {
    let cap = Capability::WorkloadClaims;
    assert_eq!(cap.as_str(), "workload_claims");
    assert!(cap.is_known());
    let parsed: Capability = "workload_claims".parse().unwrap();
    assert_eq!(parsed, Capability::WorkloadClaims);
    let json = serde_json::to_value(&cap).unwrap();
    assert_eq!(json, "workload_claims");
    let deserialized: Capability = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized, cap);
}

// =========================================================================
// Config test payload tests
// =========================================================================

#[test]
fn config_test_kind_serialization_roundtrip() {
    let variants = [
        (ConfigTestKind::VersionDetection, "\"version_detection\""),
        (
            ConfigTestKind::UpdateCommandValidation,
            "\"update_command_validation\"",
        ),
        (ConfigTestKind::PreUpdateHook, "\"pre_update_hook\""),
        (ConfigTestKind::PostUpdateHook, "\"post_update_hook\""),
        (ConfigTestKind::Connectivity, "\"connectivity\""),
    ];
    for (variant, expected_json) in &variants {
        let json = serde_json::to_string(variant).unwrap();
        assert_eq!(
            &json, expected_json,
            "serialization mismatch for {variant:?}"
        );
        let deserialized: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(&deserialized, variant, "roundtrip mismatch for {variant:?}");
    }
}

#[test]
fn test_plugin_config_payload_roundtrip() {
    let payload = TestPluginConfigPayload {
        request_id: "req-001".to_string(),
        host_machine_id: "host-abc".to_string(),
        test_kind: ConfigTestKind::VersionDetection,
        plugin_type: "releases_github".to_string(),
        config: serde_json::json!({"repo": "owner/repo"}),
        package_identifier: Some("my-pkg".to_string()),
    };
    let msg = ControllerMessage::TestPluginConfig(payload.clone());
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "test_plugin_config");
    assert_eq!(json["test_kind"], "version_detection");
    assert_eq!(json["package_identifier"], "my-pkg");
    let roundtripped: ControllerMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn test_plugin_config_payload_minimal() {
    let payload = TestPluginConfigPayload::new(
        "req-002".to_string(),
        "host-xyz".to_string(),
        ConfigTestKind::Connectivity,
        "releases_docker".to_string(),
        serde_json::json!({}),
    );
    let msg = ControllerMessage::TestPluginConfig(payload);
    let json = serde_json::to_string(&msg).unwrap();
    // package_identifier should be omitted when None
    assert!(!json.contains("package_identifier"));
    let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn test_plugin_config_result_payload_roundtrip() {
    let payload = TestPluginConfigResultPayload {
        request_id: "req-001".to_string(),
        success: true,
        output: Some("v1.2.3 detected".to_string()),
        error: None,
        detected_version: Some("1.2.3".to_string()),
        duration_ms: 150,
    };
    let msg = ServiceMessage::TestPluginConfigResult(payload.clone());
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "test_plugin_config_result");
    assert_eq!(json["success"], true);
    assert_eq!(json["detected_version"], "1.2.3");
    assert!(json.get("error").is_none());
    let roundtripped: ServiceMessage = serde_json::from_value(json).unwrap();
    assert_eq!(roundtripped, msg);
}

#[test]
fn test_plugin_config_result_payload_failure() {
    let payload = TestPluginConfigResultPayload {
        request_id: "req-003".to_string(),
        success: false,
        output: None,
        error: Some("command not found".to_string()),
        detected_version: None,
        duration_ms: 42,
    };
    let msg = ServiceMessage::TestPluginConfigResult(payload);
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("command not found"));
    assert!(!json.contains("detected_version"));
    assert!(!json.contains("output"));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn test_plugin_config_result_payload_minimal() {
    let payload = TestPluginConfigResultPayload::new("req-004".to_string(), true, 100);
    let msg = ServiceMessage::TestPluginConfigResult(payload);
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("output"));
    assert!(!json.contains("error"));
    assert!(!json.contains("detected_version"));
    let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn test_plugin_config_not_nats_publishable() {
    let msg = ControllerMessage::TestPluginConfig(TestPluginConfigPayload::new(
        "req-005".to_string(),
        "host-1".to_string(),
        ConfigTestKind::UpdateCommandValidation,
        "shell".to_string(),
        serde_json::json!({}),
    ));
    assert!(!msg.is_nats_publishable());
}

// =========================================================================
// Variant catalog guardrail — SDK/handler partition enforcement
//
// These tests provide a compile-time exhaustive check that every `ControllerMessage`
// variant is explicitly classified as either SDK-owned (consumed/routed by the
// service-sdk event loop) or handler-owned (forwarded to `ServiceHandler::on_message`).
//
// Because `ControllerMessage` is `#[non_exhaustive]`, this exhaustive match is only
// possible inside the wire crate itself — no wildcard arm is allowed here. That is
// the enforcement mechanism: adding a new variant without updating the match will
// produce a compile error.
//
// The authoritative SDK/handler split is:
//
// SDK-owned (8): Pong, Certificate, ServiceSettings, CaBundleUpdated,
//   RequestCertRenewal, ServerRestarting, ServiceConfigAck, Unknown
//
// Handler-owned (30): all remaining variants.
// =========================================================================

/// Classification of a `ControllerMessage` variant with respect to the SDK dispatch tier.
#[derive(Debug, PartialEq, Eq)]
enum VariantOwnership {
    /// Handled directly by the service-sdk event loop (never reaches `on_message`).
    SdkOwned,
    /// Forwarded to `ServiceHandler::on_message` by the catch-all arm.
    HandlerOwned,
}

/// Classify a `ControllerMessage` variant as SDK-owned or handler-owned.
///
/// This match MUST be exhaustive — no wildcard arm. That is the guardrail:
/// adding a new variant to `ControllerMessage` forces an update here.
///
/// When adding a new SDK-owned arm here, also update `handle_controller_message`
/// in `crates/shared/service-sdk/src/event_loop.rs` and the
/// `expected_sdk_owned` set in `test_variant_catalog_classification` below.
fn classify_controller_message_variant(msg: &ControllerMessage) -> VariantOwnership {
    match msg {
        // --- SDK-owned: consumed or callback-routed by the service-sdk event loop ---
        ControllerMessage::Pong(_) => VariantOwnership::SdkOwned,
        ControllerMessage::Certificate(_) => VariantOwnership::SdkOwned,
        ControllerMessage::ServiceSettings(_) => VariantOwnership::SdkOwned,
        ControllerMessage::CaBundleUpdated(_) => VariantOwnership::SdkOwned,
        ControllerMessage::RequestCertRenewal(_) => VariantOwnership::SdkOwned,
        ControllerMessage::ServerRestarting(_) => VariantOwnership::SdkOwned,
        ControllerMessage::ServiceConfigAck(_) => VariantOwnership::SdkOwned,
        ControllerMessage::Unknown => VariantOwnership::SdkOwned,
        // --- Handler-owned: forwarded to ServiceHandler::on_message ---
        ControllerMessage::Enrolled(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::Approved(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::Rejected(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::Error(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::CheckVersions(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ExecuteUpdate(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ExecuteBatchUpdate(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::DiscoverSoftware(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::SetUpdateFreeze(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::UpdateStdinData(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ResetData => VariantOwnership::HandlerOwned,
        ControllerMessage::SoftwareStates(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::HostConnectivityUpdated(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ReportPluginConfigResponse(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::SurfaceActionRequest(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::SurfaceActionCancel(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::SurfaceActionResponse(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ServiceCredentials(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ServiceConfigDelivery(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::ServiceConfigUpdated(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::RequestCaRotation(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::RequestCrlRenewal(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::SoftwareStatesChanged(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::TokenRevoked(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::BroadcastAdminEvent(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::WorkloadClaimResult(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::WorkloadClaimAnnouncement(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::WorkloadClaimSyncRequest(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::WorkloadClaimSyncResponse(_) => VariantOwnership::HandlerOwned,
        ControllerMessage::TestPluginConfig(_) => VariantOwnership::HandlerOwned,
    }
}

/// Return the Rust variant name of a `ControllerMessage` as a `&'static str`.
///
/// Used by `test_variant_catalog_classification` to detect duplicate entries in
/// `make_all_controller_message_variants`. Must be kept in sync with the enum.
fn variant_discriminant_name(msg: &ControllerMessage) -> &'static str {
    match msg {
        ControllerMessage::Pong(_) => "Pong",
        ControllerMessage::Certificate(_) => "Certificate",
        ControllerMessage::ServiceSettings(_) => "ServiceSettings",
        ControllerMessage::CaBundleUpdated(_) => "CaBundleUpdated",
        ControllerMessage::RequestCertRenewal(_) => "RequestCertRenewal",
        ControllerMessage::ServerRestarting(_) => "ServerRestarting",
        ControllerMessage::ServiceConfigAck(_) => "ServiceConfigAck",
        ControllerMessage::Unknown => "Unknown",
        ControllerMessage::Enrolled(_) => "Enrolled",
        ControllerMessage::Approved(_) => "Approved",
        ControllerMessage::Rejected(_) => "Rejected",
        ControllerMessage::Error(_) => "Error",
        ControllerMessage::CheckVersions(_) => "CheckVersions",
        ControllerMessage::ExecuteUpdate(_) => "ExecuteUpdate",
        ControllerMessage::ExecuteBatchUpdate(_) => "ExecuteBatchUpdate",
        ControllerMessage::DiscoverSoftware(_) => "DiscoverSoftware",
        ControllerMessage::SetUpdateFreeze(_) => "SetUpdateFreeze",
        ControllerMessage::UpdateStdinData(_) => "UpdateStdinData",
        ControllerMessage::ResetData => "ResetData",
        ControllerMessage::SoftwareStates(_) => "SoftwareStates",
        ControllerMessage::HostConnectivityUpdated(_) => "HostConnectivityUpdated",
        ControllerMessage::ReportPluginConfigResponse(_) => "ReportPluginConfigResponse",
        ControllerMessage::SurfaceActionRequest(_) => "SurfaceActionRequest",
        ControllerMessage::SurfaceActionCancel(_) => "SurfaceActionCancel",
        ControllerMessage::SurfaceActionResponse(_) => "SurfaceActionResponse",
        ControllerMessage::ServiceCredentials(_) => "ServiceCredentials",
        ControllerMessage::ServiceConfigDelivery(_) => "ServiceConfigDelivery",
        ControllerMessage::ServiceConfigUpdated(_) => "ServiceConfigUpdated",
        ControllerMessage::RequestCaRotation(_) => "RequestCaRotation",
        ControllerMessage::RequestCrlRenewal(_) => "RequestCrlRenewal",
        ControllerMessage::SoftwareStatesChanged(_) => "SoftwareStatesChanged",
        ControllerMessage::TokenRevoked(_) => "TokenRevoked",
        ControllerMessage::BroadcastAdminEvent(_) => "BroadcastAdminEvent",
        ControllerMessage::WorkloadClaimResult(_) => "WorkloadClaimResult",
        ControllerMessage::WorkloadClaimAnnouncement(_) => "WorkloadClaimAnnouncement",
        ControllerMessage::WorkloadClaimSyncRequest(_) => "WorkloadClaimSyncRequest",
        ControllerMessage::WorkloadClaimSyncResponse(_) => "WorkloadClaimSyncResponse",
        ControllerMessage::TestPluginConfig(_) => "TestPluginConfig",
    }
}

/// Construct one representative instance of every `ControllerMessage` variant.
///
/// `Unknown` cannot be constructed directly (it exists only as a deserialization
/// catch-all), so it is produced via `serde_json::from_value` with an unknown type tag.
/// All other variants use direct struct construction.
fn make_all_controller_message_variants() -> Vec<ControllerMessage> {
    use std::collections::BTreeMap;

    let uuid1 = TEST_UUID_1;
    let uuid2 = TEST_UUID_2;
    let uuid3 = TEST_UUID_3;

    vec![
        // --- SDK-owned ---
        ControllerMessage::Pong(PongPayload {
            service_ts: 0,
            controller_ts: 0,
        }),
        ControllerMessage::Certificate(CertificatePayload {
            cert_pem: String::new(),
            not_after: time::UtcDateTime::from_unix_timestamp(1_767_225_600).unwrap(),
        }),
        ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: String::new(),
            capabilities: BTreeSet::new(),
            report_page_limits: ReportPageLimits::default(),
            shutdown_timeout: None,
            ping_interval: std::time::Duration::from_secs(300),
            tenant_id: None,
        }),
        ControllerMessage::CaBundleUpdated(CaBundleUpdatedPayload {
            ca_bundle_pem: String::new(),
        }),
        ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
            reason: String::new(),
        }),
        ControllerMessage::ServerRestarting(ServerRestartingPayload {
            reason: String::new(),
        }),
        ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::success("r".into())),
        // Unknown is only reachable via deserialization of an unknown type tag.
        serde_json::from_value::<ControllerMessage>(
            serde_json::json!({"type": "__test_unknown__"}),
        )
        .unwrap(),
        // --- Handler-owned ---
        ControllerMessage::Enrolled(EnrolledPayload {
            service_id: uuid1,
            enrollment_secret: uptrakit_shared_types::SecretString::new("s"),
            status: EnrollmentStatus::Pending,
        }),
        ControllerMessage::Approved(ApprovedPayload { service_id: uuid1 }),
        ControllerMessage::Rejected(RejectedPayload { service_id: uuid1 }),
        ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::EnrollmentFailed,
            message: "test".into(),
        }),
        ControllerMessage::CheckVersions(CheckVersionsPayload {
            host_machine_id: "m".into(),
            assignments: vec![],
        }),
        ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            host_machine_id: "m".into(),
            update_history_id: uuid1,
            software_item_id: uuid2,
            software_item_name: "pkg".into(),
            to_version: "1.0".into(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "owner/repo".into(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            release_info: None,
            timeout: std::time::Duration::from_secs(600),
            interactive: false,
        })),
        ControllerMessage::ExecuteBatchUpdate(Box::new(ExecuteBatchUpdatePayload {
            host_machine_id: "m".into(),
            batch_id: uuid1,
            plugin_type: plugin_ids::PACKAGE_MANAGER_APT.clone(),
            plugin_config: serde_json::json!({}),
            updates: vec![],
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            timeout: std::time::Duration::from_secs(3600),
            interactive: false,
        })),
        ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
            host_machine_id: "m".into(),
            plugins: vec![],
        }),
        ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
            enabled: false,
            reason: None,
        }),
        ControllerMessage::UpdateStdinData(UpdateStdinDataPayload::new(uuid1, String::new())),
        ControllerMessage::ResetData,
        ControllerMessage::SoftwareStates(SoftwareStatesPayload {
            tenant_id: uuid1,
            items: vec![],
            host_summaries: vec![],
            hosts: vec![],
            page: SoftwareStatesPage::single(),
        }),
        ControllerMessage::HostConnectivityUpdated(HostConnectivityUpdatedPayload::new(
            uuid1,
            vec![],
        )),
        ControllerMessage::ReportPluginConfigResponse(ReportPluginConfigResponsePayload {
            request_id: "r".into(),
            success: true,
            plugin_config_id: None,
            error: None,
        }),
        ControllerMessage::SurfaceActionRequest(test_surface_action_request()),
        ControllerMessage::SurfaceActionCancel(surfaces::SurfaceActionCancel {
            request_id: uuid1,
            target_provider_id: "uptrakit-agent-ssh".to_string(),
            reason: surfaces::SurfaceActionCancelReason::Timeout,
        }),
        ControllerMessage::SurfaceActionResponse(surfaces::SurfaceActionResponse {
            request_id: uuid1,
            success: true,
            result: None,
            error: None,
        }),
        ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
            db_url: None,
            nats_url: None,
            master_key_hex: None,
        }),
        ControllerMessage::ServiceConfigDelivery(ServiceConfigDeliveryPayload::new(vec![])),
        ControllerMessage::ServiceConfigUpdated(ServiceConfigUpdatedPayload::new(vec![], vec![])),
        ControllerMessage::RequestCaRotation(RequestCaRotationPayload {
            reason: "test".into(),
        }),
        ControllerMessage::RequestCrlRenewal(RequestCrlRenewalPayload::default()),
        ControllerMessage::SoftwareStatesChanged(SoftwareStatesChangedPayload::new(uuid1)),
        ControllerMessage::TokenRevoked(TokenRevokedPayload {
            jti: Some("jti".into()),
            exp: Some(9_999_999_999),
            user_id: None,
            iat_cutoff: None,
            purge_after: None,
        }),
        ControllerMessage::BroadcastAdminEvent(BroadcastAdminEventPayload {
            tenant_id: Some(uuid1),
            event_json: "{}".into(),
        }),
        ControllerMessage::WorkloadClaimResult(WorkloadClaimResultPayload::new(
            BTreeSet::new(),
            BTreeSet::new(),
        )),
        ControllerMessage::WorkloadClaimAnnouncement(WorkloadClaimAnnouncementPayload::new(
            uuid1,
            uuid2,
            BTreeMap::new(),
            BTreeSet::new(),
            "2026-01-01T00:00:00Z".into(),
        )),
        ControllerMessage::WorkloadClaimSyncRequest(WorkloadClaimSyncRequestPayload::new(uuid3)),
        ControllerMessage::WorkloadClaimSyncResponse(WorkloadClaimSyncResponsePayload::new(
            uuid3,
            BTreeMap::new(),
        )),
        ControllerMessage::TestPluginConfig(TestPluginConfigPayload::new(
            "r".into(),
            "m".into(),
            ConfigTestKind::UpdateCommandValidation,
            "shell".into(),
            serde_json::json!({}),
        )),
    ]
}

#[test]
fn test_variant_catalog_spot_checks() {
    assert_eq!(
        classify_controller_message_variant(&ControllerMessage::ServerRestarting(
            ServerRestartingPayload {
                reason: "test".into()
            }
        )),
        VariantOwnership::SdkOwned,
        "ServerRestarting must be SDK-owned"
    );
    assert_eq!(
        classify_controller_message_variant(&ControllerMessage::CheckVersions(
            CheckVersionsPayload {
                host_machine_id: "m".into(),
                assignments: vec![]
            }
        )),
        VariantOwnership::HandlerOwned,
        "CheckVersions must be handler-owned"
    );
    assert_eq!(
        classify_controller_message_variant(&ControllerMessage::Unknown),
        VariantOwnership::SdkOwned,
        "Unknown must be SDK-owned"
    );
    assert_eq!(
        classify_controller_message_variant(&ControllerMessage::ServiceConfigAck(
            ServiceConfigAckPayload::success("r".into())
        )),
        VariantOwnership::SdkOwned,
        "ServiceConfigAck must be SDK-owned"
    );
}

#[test]
fn test_variant_catalog_classification() {
    let variants = make_all_controller_message_variants();

    // 1. Total count must be exactly 38 — one entry per variant.
    assert_eq!(
        variants.len(),
        38,
        "make_all_controller_message_variants must return exactly 38 entries (one per variant); \
         update it when adding or removing ControllerMessage variants"
    );

    // 2. Each entry must map to a unique discriminant name (no duplicates in the catalog).
    let names: Vec<&'static str> = variants.iter().map(variant_discriminant_name).collect();
    let unique: HashSet<&'static str> = names.iter().copied().collect();
    assert_eq!(
        unique.len(),
        38,
        "make_all_controller_message_variants contains duplicate variant entries: {:?}",
        {
            let mut seen = HashSet::new();
            names
                .iter()
                .filter(|n| !seen.insert(*n))
                .copied()
                .collect::<Vec<_>>()
        }
    );

    // 3. Exact SDK-owned membership — must be precisely the 8 variants handled by the
    //    service-sdk event loop. Update this set when the SDK/handler boundary changes,
    //    and also update `handle_controller_message` in event_loop.rs.
    let expected_sdk_owned: HashSet<&'static str> = [
        "Pong",
        "Certificate",
        "ServiceSettings",
        "CaBundleUpdated",
        "RequestCertRenewal",
        "ServerRestarting",
        "ServiceConfigAck",
        "Unknown",
    ]
    .into_iter()
    .collect();

    let actual_sdk_owned: HashSet<&'static str> = variants
        .iter()
        .filter(|m| classify_controller_message_variant(m) == VariantOwnership::SdkOwned)
        .map(variant_discriminant_name)
        .collect();

    assert_eq!(
        actual_sdk_owned,
        expected_sdk_owned,
        "SDK-owned variant set mismatch.\n\
         Missing from SDK: {:?}\n\
         Unexpected in SDK: {:?}",
        expected_sdk_owned
            .difference(&actual_sdk_owned)
            .collect::<Vec<_>>(),
        actual_sdk_owned
            .difference(&expected_sdk_owned)
            .collect::<Vec<_>>(),
    );
}
