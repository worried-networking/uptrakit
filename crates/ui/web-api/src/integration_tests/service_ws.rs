use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::test_harness::TestApp;

/// Build a test app, bind it to a random TCP port, and return the base URL.
async fn serve_app() -> (String, TestApp) {
    let app = TestApp::new().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let router = app.router.clone();
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });

    (format!("ws://127.0.0.1:{}", addr.port()), app)
}

#[tokio::test]
async fn anonymous_connect_and_enroll() {
    let (base_url, _app) = serve_app().await;

    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("{base_url}/api/v1/ws/service"))
        .await
        .expect("ws connect");

    // Send an enroll message.
    let enroll = serde_json::json!({
        "protocol_version": 1,
        "seq": 1,
        "type": "enroll",
        "hostname": "test-host",
        "friendly_name": "Test Host",
        "capabilities": ["SoftwareDiscovery"],
        "service_app_name": "uptrakit-agent"
    });
    ws.send(Message::Text(enroll.to_string().into()))
        .await
        .expect("send enroll");

    // Read the response — should be an Enrolled message.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("timeout")
        .expect("stream ended")
        .expect("read");

    let text = msg.into_text().expect("text message");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(parsed["type"], "enrolled");
    assert!(parsed["service_id"].as_str().is_some());
    assert!(parsed["enrollment_secret"].as_str().is_some());
}

#[tokio::test]
async fn enrolled_reconnect_with_bearer() {
    let (base_url, _app) = serve_app().await;

    // First: enroll anonymously.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{base_url}/api/v1/ws/service"))
        .await
        .expect("ws connect");

    let enroll = serde_json::json!({
        "protocol_version": 1,
        "seq": 1,
        "type": "enroll",
        "hostname": "reconnect-host",
        "friendly_name": "Reconnect Host",
        "capabilities": ["SoftwareDiscovery"],
        "service_app_name": "uptrakit-agent"
    });
    ws.send(Message::Text(enroll.to_string().into()))
        .await
        .expect("send enroll");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("timeout")
        .expect("stream ended")
        .expect("read");

    let text = msg.into_text().expect("text");
    let enrolled: serde_json::Value = serde_json::from_str(&text).expect("parse");
    assert_eq!(enrolled["type"], "enrolled");

    let secret = enrolled["enrollment_secret"]
        .as_str()
        .expect("secret")
        .to_string();

    // Close first connection.
    let _ = ws.close(None).await;

    // Reconnect with the bearer secret.
    let ws_url = format!("{base_url}/api/v1/ws/service");
    let request = http::Request::builder()
        .uri(&ws_url)
        .header("authorization", format!("Bearer {secret}"))
        .header(
            "sec-websocket-key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header("sec-websocket-version", "13")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("host", "127.0.0.1")
        .body(())
        .expect("build request");

    let (mut ws2, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("reconnect");

    // Send a ping to verify the connection is alive.
    let ping = serde_json::json!({
        "protocol_version": 1,
        "seq": 1,
        "type": "ping",
        "service_ts": 0
    });
    ws2.send(Message::Text(ping.to_string().into()))
        .await
        .expect("send ping");

    // Should receive a pong back.
    let pong = tokio::time::timeout(std::time::Duration::from_secs(5), ws2.next())
        .await
        .expect("timeout")
        .expect("stream ended")
        .expect("read pong");

    let pong_text = pong.into_text().expect("pong text");
    let pong_parsed: serde_json::Value = serde_json::from_str(&pong_text).expect("parse pong");
    assert_eq!(pong_parsed["type"], "pong");
}

#[tokio::test]
async fn service_connection_registry_send() {
    use crate::service_connections::ServiceConnectionRegistry;
    use uptrakit_internal_wire::ControllerMessage;

    let registry = ServiceConnectionRegistry::new();
    let service_id = uuid::Uuid::now_v7();

    let (mut rx, _cancel) = registry
        .register(service_id, Default::default(), None, None, None)
        .await;

    // Send a message through the registry.
    let sent = registry
        .send(
            &service_id,
            ControllerMessage::Pong(uptrakit_internal_wire::PongPayload::new(0, 0)),
        )
        .await;
    assert!(sent);

    // The receiver should get the message.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout")
        .expect("recv");

    assert!(matches!(msg, ControllerMessage::Pong(_)));
}

#[tokio::test]
async fn service_connection_registry_broadcast() {
    use crate::service_connections::ServiceConnectionRegistry;
    use uptrakit_internal_wire::ControllerMessage;

    let registry = ServiceConnectionRegistry::new();

    let id1 = uuid::Uuid::now_v7();
    let id2 = uuid::Uuid::now_v7();
    let (mut rx1, _c1) = registry
        .register(id1, Default::default(), None, None, None)
        .await;
    let (mut rx2, _c2) = registry
        .register(id2, Default::default(), None, None, None)
        .await;

    registry
        .broadcast(ControllerMessage::Pong(
            uptrakit_internal_wire::PongPayload::new(0, 0),
        ))
        .await;

    let m1 = tokio::time::timeout(std::time::Duration::from_secs(1), rx1.recv())
        .await
        .expect("timeout1")
        .expect("recv1");
    let m2 = tokio::time::timeout(std::time::Duration::from_secs(1), rx2.recv())
        .await
        .expect("timeout2")
        .expect("recv2");

    assert!(matches!(m1, ControllerMessage::Pong(_)));
    assert!(matches!(m2, ControllerMessage::Pong(_)));
}
