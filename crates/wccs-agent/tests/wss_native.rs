//! Opt-in transport soak against an isolated, already enrolled test device.
use futures_util::{SinkExt, StreamExt};
use std::{fs, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use wccs_protocol::control::{
    Actual, CONTROL_PROTOCOL, CONTROL_VERSION, ControlPoll, ControlReply,
};

#[test]
#[ignore = "requires WCCS_WSS_TEST_IDENTITY and a stopped agent on an isolated Windows test machine"]
fn native_wss_repeated_exchange() {
    let path = std::path::PathBuf::from(std::env::var_os("WCCS_WSS_TEST_IDENTITY").unwrap());
    let identity: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let server = identity["server"].as_str().unwrap();
    assert!(server.starts_with("https://"));
    let credential = fs::read_to_string(path.with_extension("credential")).unwrap();
    let mut request = format!("wss://{}/api/device/control", &server[8..])
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {credential}").parse().unwrap(),
    );
    request.headers_mut().insert(
        "x-wccs-device-id",
        identity["device_id"].as_str().unwrap().parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("sec-websocket-protocol", CONTROL_PROTOCOL.parse().unwrap());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut socket, _) = connect_async(request).await.unwrap();
        let poll = ControlPoll {
            protocol_version: CONTROL_VERSION,
            instance_id: wccs_protocol::token(),
            session_key: None,
            capabilities: vec!["waiting-v1".into()],
            actual: Actual::default(),
            report: None,
        };
        for index in 0..384 {
            socket
                .send(Message::Text(serde_json::to_string(&poll).unwrap().into()))
                .await
                .unwrap();
            let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap_or_else(|_| panic!("receive timeout at exchange {index}"))
                .unwrap()
                .unwrap();
            let Message::Text(text) = message else {
                panic!("unexpected response at exchange {index}");
            };
            let reply: ControlReply = serde_json::from_str(&text).unwrap();
            assert_eq!(reply.protocol_version, CONTROL_VERSION);
            assert!(
                reply.command.is_none(),
                "test device must have no active command"
            );
            if index % 32 == 0 {
                println!("exchange {index} complete");
            }
        }
        socket.close(None).await.unwrap();
    });
}
