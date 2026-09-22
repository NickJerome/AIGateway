mod support;
use futures::{SinkExt, StreamExt};
use gateway::http::Server;
use tokio_tungstenite::{connect_async, tungstenite::Message};
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_js_callbacks_and_normal_request_cannot_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"ws",r#"export default {fetch(r,env,ctx){if(new URL(r.url).pathname.endsWith('normal'))return new Response('ok');const pair=new WebSocketPair();const server=pair[1];server.accept();server.addEventListener('message',e=>server.send(e.data));server.addEventListener('close',()=>ctx.waitUntil(new Promise(resolve=>setTimeout(resolve,20))));return new Response(null,{status:101,webSocket:pair[0]});}}"#,"").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let normal = reqwest::get(format!("http://{}/ws/", server.address))
        .await
        .unwrap();
    assert_eq!(normal.status(), 500);
    let (mut socket, _) = connect_async(format!("ws://{}/ws/", server.address))
        .await
        .unwrap();
    socket.send(Message::Text("hello".into())).await.unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response, Message::Text("hello".into()));
    socket
        .send(Message::Binary(vec![0, 255, 1].into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Binary(vec![0, 255, 1].into())
    );
    socket.close(None).await.unwrap();
    drop(socket);
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_relay_releases_engine_and_returns_failed_handshake_status() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            tokio::select! {accept=listener.accept()=>{let Ok((stream,_))=accept else{break;};children.spawn(async move{let mut ws=tokio_tungstenite::accept_async(stream).await.unwrap();while let Some(Ok(message))=ws.next().await{if message.is_close(){break;}ws.send(message).await.unwrap();}});},_=children.join_next(),if !children.is_empty()=>{}}
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "export default {{async fetch(r,env,ctx){{if(new URL(r.url).pathname.endsWith('normal'))return new Response('free');const upstream=await ctx.fetch('http://{upstream}/',{{headers:{{Upgrade:'websocket'}}}});return new Response(null,{{status:101,webSocket:upstream.webSocket}});}}}}"
    );
    support::install(dir.path(), "relay", &source, "").await;
    let mut config = support::config(dir.path());
    config.limits.max_in_flight = 1;
    let server = Server::start(config).await.unwrap();
    let (mut ws, _) = connect_async(format!("ws://{}/relay/", server.address))
        .await
        .unwrap();
    ws.send(Message::Text("native".into())).await.unwrap();
    assert_eq!(
        ws.next().await.unwrap().unwrap(),
        Message::Text("native".into())
    );
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reqwest::get(format!("http://{}/relay/normal", server.address)),
    )
    .await
    .expect("native websocket retained request permit")
    .unwrap();
    assert_eq!(response.status(), 200);
    ws.close(None).await.unwrap();
    drop(ws);
    server.shutdown().await.unwrap();
    fixture.abort();
}

#[tokio::test]
async fn workerd_constructor_and_event_ports() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "{}\n{}",
        include_str!("fixtures/workerd/node-assert-shim.mjs"),
        include_str!("fixtures/workerd/spec11.js")
    );
    support::install(dir.path(), "ws-port", &source, "").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let response = reqwest::get(format!("http://{}/ws-port/", server.address))
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert_eq!(text, "15 workerd groups passed");
    server.shutdown().await.unwrap();
}

#[path = "fixtures/mock/mod.rs"]
mod mock;
// Tungstenite's callback requires its large HTTP response error type.
#[allow(clippy::result_large_err)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_constructor_protocol_errors_and_failed_handshake_bodies() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            tokio::select! {accept=listener.accept()=>{let Ok((stream,_))=accept else {break;};children.spawn(async move {
                            let path=std::sync::Arc::new(std::sync::Mutex::new(String::new()));let copy=path.clone();
                            let mut ws=tokio_tungstenite::accept_hdr_async(stream,move |request:&tokio_tungstenite::tungstenite::handshake::server::Request,mut response:tokio_tungstenite::tungstenite::handshake::server::Response| {
                                *copy.lock().unwrap()=request.uri().path().into();
                                if request.headers().contains_key("sec-websocket-protocol") {response.headers_mut().insert("sec-websocket-protocol","chat".parse().unwrap());}
                                Ok(response)
                            }).await.unwrap();
                            let large=path.lock().unwrap().contains("large");
                            if large {let _=ws.send(Message::Binary(vec![0;2048].into())).await;} else {while let Some(Ok(message))=ws.next().await{if message.is_close(){let _=ws.flush().await;break;}
            if ws.send(message).await.is_err(){break;}}}
                        });},_=children.join_next(),if !children.is_empty()=>{}}
        }
    });
    let status = mock::Mock::start(|path, _, _| async move {
        let code = path.trim_matches('/').parse().unwrap();
        (code, vec![], format!("upstream {code}"))
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r#"export default {{async fetch(r,env,ctx){{const q=new URL(r.url).searchParams;if(q.has('status'))return ctx.fetch('{}/'+q.get('status'),{{headers:{{Upgrade:'websocket'}}}});return new Response(await new Promise((resolve,reject)=>{{const mode=q.get('mode');const ws=new WebSocket('ws://{address}/'+mode,['chat']);let error=false;ws.onerror=e=>{{error=true;}};ws.onopen=()=>{{if(ws.protocol!=='chat')reject(new Error('protocol'));if(mode!=='large')ws.send('echo');}};ws.onmessage=e=>{{resolve(e.data);ws.close();}};ws.onclose=e=>{{if(mode==='large')resolve(JSON.stringify({{error,code:e.code}}));}};setTimeout(()=>reject(new Error('websocket timeout')),3000);}}));}}}}"#,
        status.base
    );
    support::install(dir.path(), "client-ws", &source, "").await;
    let mut config = support::config(dir.path());
    config.websocket.max_message_bytes = 1024;
    let server = Server::start(config).await.unwrap();
    let client = reqwest::Client::new();
    for code in [401, 429, 404, 501] {
        let response = client
            .get(format!(
                "http://{}/client-ws/?status={code}",
                server.address
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), code);
        assert_eq!(response.text().await.unwrap(), format!("upstream {code}"));
    }
    for (mode, expected) in [("echo", "echo"), ("large", r#"{"error":true,"code":1009}"#)] {
        let response = client
            .get(format!("http://{}/client-ws/?mode={mode}", server.address))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, 200, "{text}");
        assert_eq!(text, expected);
    }
    server.shutdown().await.unwrap();
    fixture.abort();
    let _ = fixture.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reinstall_closes_both_socket_modes_and_pending_durable_rpc() {
    let upstream_dir = tempfile::tempdir().unwrap();
    support::install(upstream_dir.path(),"echo",r#"export default {fetch(){const p=new WebSocketPair();p[1].accept();p[1].onmessage=e=>p[1].send(e.data);return new Response(null,{status:101,webSocket:p[0]});}}"#,"").await;
    let upstream = Server::start(support::config(upstream_dir.path()))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let binding = "[[durable_objects.bindings]]\nname='STATE'\nclass_name='State'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['State']";
    let class = "import {DurableObject} from 'cloudflare:workers';export class State extends DurableObject {async wait(){await this.ctx.storage.put('v','confirmed');await new Promise(()=>{});}async read(){return await this.ctx.storage.get('v');}}";
    let code = format!(
        r#"{class} export default {{async fetch(r,e,c){{const path=new URL(r.url).pathname;const s=e.STATE.get(e.STATE.idFromName('one'));if(path.endsWith('pending'))return new Response(await s.wait());if(path.endsWith('state'))return new Response(await s.read()??'waiting');if(path.endsWith('native')){{const up=await c.fetch('http://{}/echo/',{{headers:{{Upgrade:'websocket'}}}});return new Response(null,{{status:101,webSocket:up.webSocket}});}}const p=new WebSocketPair();p[1].accept();return new Response(null,{{status:101,webSocket:p[0]}});}}}}"#,
        upstream.address
    );
    support::install(dir.path(), "replace-ws", &code, binding).await;
    let mut process =
        support::Process::start(dir.path(), "max_in_flight=4\n[http]\ndrain_timeout_ms=300");
    let base = process.base.replacen("http:", "ws:", 1);
    let (mut pair, _) = connect_async(format!("{base}/replace-ws/pair"))
        .await
        .unwrap();
    let (mut native, _) = connect_async(format!("{base}/replace-ws/native"))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let call = client.get(format!("{}/replace-ws/pending", process.base));
    let pending = tokio::spawn(async move { call.send().await });
    for i in 0..100 {
        let value = client
            .get(format!("{}/replace-ws/state", process.base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        if value == "confirmed" {
            break;
        }
        assert!(i < 99);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let code = format!(
        "{class} export default {{async fetch(r,e){{return new Response('new:'+await e.STATE.get(e.STATE.idFromName('one')).read())}}}}"
    );
    let response = client
        .post(format!("{}/__gateway/plugins", process.base))
        .bearer_auth("fixture-admin-token")
        .header("content-type", "application/zip")
        .body(support::package("replace-ws", &code, binding))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    for socket in [&mut pair, &mut native] {
        let next = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("reinstall left socket open");
        assert!(!matches!(
            next,
            Some(Ok(Message::Text(_) | Message::Binary(_)))
        ));
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .expect("DO caller remained pending");
    assert_eq!(
        client
            .get(format!("{}/replace-ws/state", process.base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "new:confirmed"
    );
    process.stop();
    upstream.shutdown().await.unwrap();
}
