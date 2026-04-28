use std::path::PathBuf;
use std::sync::Arc;

use uptrakit_agent_runtime::{AgentRuntime, AgentRuntimeConfig, AgentRuntimeEvent};
use uptrakit_command::NoopCommandExecutor;
use uptrakit_service_sdk::test_support::MockTransport;
use uptrakit_wire::{ConfigTestKind, ControllerMessage, ServiceMessage, TestPluginConfigPayload};

fn runtime_config(freeze_file_path: PathBuf) -> AgentRuntimeConfig {
    AgentRuntimeConfig::new(
        Arc::new(NoopCommandExecutor),
        freeze_file_path,
        "test-agent-version".to_string(),
    )
}

fn register_runtime_instance_id(msg: &ServiceMessage) -> Option<uuid::Uuid> {
    match msg {
        ServiceMessage::Register(payload) => payload.runtime_instance_id,
        other => panic!("expected Register message, got {other:?}"),
    }
}

#[tokio::test]
async fn connected_runtime_stages_initial_report_until_flushed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runtime = AgentRuntime::new(runtime_config(temp.path().join("update-freeze")));
    let mut transport = MockTransport::new();

    runtime
        .on_connected(&mut transport)
        .await
        .expect("connect should succeed");

    assert_eq!(transport.send_log().len(), 1);
    assert!(matches!(
        transport.send_log()[0],
        ServiceMessage::Register(_)
    ));

    runtime
        .send_pending_initial_report(&mut transport)
        .await
        .expect("report should send");

    assert_eq!(transport.send_log().len(), 2);
    assert!(matches!(
        transport.send_log()[1],
        ServiceMessage::ReportHosts(_)
    ));
}

#[tokio::test]
async fn register_reuses_runtime_instance_id_across_reconnects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runtime = AgentRuntime::new(runtime_config(temp.path().join("update-freeze")));

    let mut first_transport = MockTransport::new();
    runtime
        .on_connected(&mut first_transport)
        .await
        .expect("first connect should succeed");

    let mut second_transport = MockTransport::new();
    runtime
        .on_connected(&mut second_transport)
        .await
        .expect("second connect should succeed");

    let first_id = register_runtime_instance_id(&first_transport.send_log()[0]);
    let second_id = register_runtime_instance_id(&second_transport.send_log()[0]);

    assert!(
        first_id.is_some(),
        "register must include runtime_instance_id"
    );
    assert_eq!(
        first_id, second_id,
        "runtime_instance_id must be stable across reconnects",
    );
}

#[tokio::test]
async fn test_plugin_config_message_produces_background_result_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runtime = AgentRuntime::new(runtime_config(temp.path().join("update-freeze")));
    let mut transport = MockTransport::new();

    runtime
        .on_connected(&mut transport)
        .await
        .expect("connect should succeed");

    let payload = TestPluginConfigPayload::new(
        "request-1".to_string(),
        runtime.machine_id().expect("machine id").to_string(),
        ConfigTestKind::Connectivity,
        "generic_shell".to_string(),
        serde_json::json!({}),
    );

    runtime
        .handle_controller_message(ControllerMessage::TestPluginConfig(payload), &mut transport)
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), runtime.poll_event())
        .await
        .expect("runtime should emit an event");

    assert!(matches!(event, AgentRuntimeEvent::BackgroundResult(_)));

    runtime.handle_event(event, &mut transport).await;

    assert!(
        transport
            .send_log()
            .iter()
            .any(|msg| matches!(msg, ServiceMessage::TestPluginConfigResult(_))),
        "expected TestPluginConfigResult to be forwarded",
    );
}
