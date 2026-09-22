#[path = "fixtures/mock/mod.rs"]
mod mock;
mod support;
use futures::{SinkExt, StreamExt};
use gateway::{
    http::Server,
    plugins::{Overrides, PluginManager},
    storage::Database,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};
// Tungstenite fixes the callback error type to an HTTP response.
#[allow(clippy::result_large_err)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bootstrap_fixed_account_exclusive_call_close_release_and_native_relay() {
    let accounts = Arc::new(Mutex::new(Vec::<String>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_address = listener.local_addr().unwrap();
    let captured = accounts.clone();
    let ws_task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            tokio::select! {a=listener.accept()=>{let Ok((stream,_))=a else{break;};let captured=captured.clone();children.spawn(async move{let mut socket=tokio_tungstenite::accept_hdr_async(stream,move|request:&tokio_tungstenite::tungstenite::handshake::server::Request,response|{captured.lock().unwrap().push(request.headers().get("chatgpt-account-id").unwrap().to_str().unwrap().into());Ok(response)}).await.unwrap();while let Some(Ok(message))=socket.next().await{if message.is_close(){let _=socket.flush().await;break;}socket.send(message).await.unwrap();}});},_=children.join_next(),if !children.is_empty()=>{}}
        }
    });
    let bootstrap_account = Arc::new(Mutex::new(String::new()));
    let capture = bootstrap_account.clone();
    let upstream=mock::Mock::start(move|path,headers,_|{let capture=capture.clone();async move{
  if path=="/calls"{*capture.lock().unwrap()=headers.get("chatgpt-account-id").unwrap().to_str().unwrap().into();return(201,vec![("location".into(),"/realtime/calls/test-call".into()),("content-type".into(),"application/sdp".into())],"mock-sdp".into());}
  let value=if path.ends_with("/usercode"){json!({"device_auth_id":"device","user_code":"TEST","interval":1})}else if path.ends_with("/deviceauth/token"){json!({"authorization_code":"code","code_verifier":"verifier"})}else{json!({"access_token":"mock-access","refresh_token":"mock-refresh","expires_in":3600})};(200,vec![("content-type".into(),"application/json".into())],value.to_string())
 }}).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 2, 16).unwrap();
    let mut manager = PluginManager::open(dir.path().join("plugins"), db, Default::default())
        .await
        .unwrap();
    manager
        .install(include_bytes!("../examples/codex-pool/codex-pool.zip").to_vec())
        .await
        .unwrap();
    let mut overrides = Overrides::default();
    overrides
        .vars
        .insert("ADMIN_TOKEN".into(), json!("plugin-fixture"));
    overrides
        .vars
        .insert("AUTH_BASE".into(), json!(upstream.base));
    overrides.vars.insert(
        "REALTIME_UPSTREAM".into(),
        json!(format!("http://{ws_address}/realtime")),
    );
    overrides.vars.insert(
        "REALTIME_BOOTSTRAP".into(),
        json!(format!("{}/calls", upstream.base)),
    );
    manager.configure("codex-pool", overrides).await.unwrap();
    drop(manager);
    let mut config = support::config(dir.path());
    config.threads = 2;
    let server = Server::start(config).await.unwrap();
    let base = format!("http://{}", server.address);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let login: Value = serde_json::from_str(
        &client
            .post(format!("{base}/codex/admin/accounts/login"))
            .bearer_auth("plugin-fixture")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    let poll = client
        .post(format!(
            "{base}/codex/admin/accounts/login/{}/poll",
            login["id"].as_str().unwrap()
        ))
        .bearer_auth("plugin-fixture")
        .send()
        .await
        .unwrap();
    assert!(poll.text().await.unwrap().contains("done"));
    let boot = client
        .post(format!("{base}/codex/realtime/calls"))
        .header("content-type", "application/sdp")
        .body("offer")
        .send()
        .await
        .unwrap();
    let status = boot.status();
    let location = boot
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_owned());
    let body = boot.text().await.unwrap();
    assert_eq!(status, 201, "{body}");
    assert_eq!(
        location,
        Some(format!("{base}/codex/realtime/calls/test-call"))
    );
    let url = format!("ws://{}/codex/realtime/calls/test-call", server.address);
    let (mut first, _) = connect_async(&url).await.unwrap();
    first.send(Message::Text("sideband".into())).await.unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text("sideband".into())
    );
    let second = connect_async(&url).await;
    assert!(
        matches!(second,Err(tokio_tungstenite::tungstenite::Error::Http(ref r)) if r.status()==409)
    );
    first.close(None).await.unwrap();
    drop(first);
    let mut reopened = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if let Ok((socket, _)) = connect_async(&url).await {
            reopened = Some(socket);
            break;
        }
    }
    let mut reopened = reopened.expect("close waitUntil did not release call lease");
    reopened.close(None).await.unwrap();
    drop(reopened);
    let (mut native, _) =
        connect_async(format!("ws://{}/codex/realtime?model=test", server.address))
            .await
            .unwrap();
    native
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(
        native.next().await.unwrap().unwrap(),
        Message::Binary(vec![1, 2, 3].into())
    );
    native.close(None).await.unwrap();
    drop(native);
    assert!(
        accounts
            .lock()
            .unwrap()
            .iter()
            .all(|a| a == &*bootstrap_account.lock().unwrap())
    );
    server.shutdown().await.unwrap();
    ws_task.abort();
}
