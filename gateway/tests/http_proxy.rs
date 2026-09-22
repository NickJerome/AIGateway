mod support;
use gateway::http::Server;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_request_response_and_kv() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"hello",r#"export default {async fetch(req,env){await env.CONFIG.put('x',JSON.stringify({ok:true}));return Response.json({method:req.method,input:await req.text(),saved:await env.CONFIG.get('x',{type:'json'})},{headers:[['set-cookie','a=1'],['set-cookie','b=2']]})}}"#,"[[kv_namespaces]]\nbinding='CONFIG'\nid='hello-data'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .post(format!("http://{}/hello/echo", server.address))
            .body("payload")
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    let status = response.status();
    let cookies = response.headers().get_all("set-cookie").iter().count();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert_eq!(cookies, 2);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["input"], "payload");
    assert_eq!(value["saved"]["ok"], true);
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbound_fetch_and_stream_transform() {
    let upstream_dir = tempfile::tempdir().unwrap();
    support::install(upstream_dir.path(),"up",r#"export default {fetch(){return new Response(new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('first\n'));setTimeout(()=>{c.enqueue(new TextEncoder().encode('last\n'));c.close()},150)}}),{status:429,headers:{'content-type':'text/event-stream'}})}}"#,"").await;
    let upstream = Server::start(support::config(upstream_dir.path()))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let code = format!(
        r#"export default {{async fetch(){{const r=await fetch('http://{}/up');return new Response(r.body.pipeThrough(new TransformStream({{transform(v,c){{c.enqueue(v)}}}})),{{status:r.status,headers:r.headers}})}}}}"#,
        upstream.address
    );
    support::install(dir.path(), "proxy", &code, "").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reqwest::get(format!("http://{}/proxy", server.address)),
    )
    .await
    .unwrap()
    .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 429, "{text}");
    assert_eq!(text, "first\nlast\n");
    server.shutdown().await.unwrap();
    upstream.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn administrator_hono_upload_config_disable_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut process = support::Process::start(dir.path(), "[http]\ndrain_timeout_ms=150");
    let client = reqwest::Client::new();
    let endpoint = format!("{}/__gateway/plugins", process.base);
    assert_eq!(client.get(&endpoint).send().await.unwrap().status(), 401);
    assert_eq!(
        client
            .get(&endpoint)
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let code = format!(
        "{}\nconst app=new HonoQuick.Hono();app.get('/hello/*',c=>c.text(c.env.GREETING));export default app;",
        include_str!("fixtures/hono-quick-4.13.8.mjs")
    );
    let response = client
        .post(&endpoint)
        .bearer_auth("fixture-admin-token")
        .header("content-type", "application/zip")
        .body(support::package(
            "hello",
            &code,
            "[vars]\nGREETING='hello'\n[build]\ncommand='exit 99'",
        ))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 201, "{text}");
    let url = format!("{}/hello/world", process.base);
    assert_eq!(
        client.get(&url).send().await.unwrap().text().await.unwrap(),
        "hello"
    );
    let response = client
        .put(format!("{endpoint}/hello/config"))
        .bearer_auth("fixture-admin-token")
        .body(r#"{"vars":{"GREETING":"override-secret"},"bindings":{}}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(!response.text().await.unwrap().contains("override-secret"));
    assert_eq!(
        client.get(&url).send().await.unwrap().text().await.unwrap(),
        "override-secret"
    );
    assert_eq!(
        client
            .put(format!("{endpoint}/hello/enabled"))
            .bearer_auth("fixture-admin-token")
            .body(r#"{"enabled":false}"#)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(client.get(&url).send().await.unwrap().status(), 503);
    process.stop();
    let mut process = support::Process::start(dir.path(), "[http]\ndrain_timeout_ms=150");
    let endpoint = format!("{}/__gateway/plugins/hello", process.base);
    assert_eq!(
        client
            .get(format!("{}/hello/world", process.base))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        client
            .delete(endpoint)
            .bearer_auth("fixture-admin-token")
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        client
            .get(format!("{}/hello/world", process.base))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    process.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_passthrough_body_sizes_and_http2() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    support::install(
        dir.path(),
        "echo",
        "export default {fetch(r){return new Response(r.body,{headers:r.headers})}}",
        "",
    )
    .await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    for h2 in [false, true] {
        let client = if h2 {
            reqwest::Client::builder()
                .http2_prior_knowledge()
                .build()
                .unwrap()
        } else {
            reqwest::Client::new()
        };
        for length in [8800, 478000] {
            let input = (0..length).map(|i| (i % 251) as u8).collect::<Vec<_>>();
            let expected = Sha256::digest(&input);
            let response = client
                .post(format!("http://{}/echo", server.address))
                .body(input)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let output = response.bytes().await.unwrap();
            assert_eq!(output.len(), length);
            assert_eq!(Sha256::digest(output), expected);
        }
    }
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_limit_and_permit_recovery_after_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"sse",r#"export default {fetch(){return new Response(new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('first\n'))},pull(){return new Promise(()=>{})}}))}}"#,"").await;
    let mut config = support::config(dir.path());
    config.limits.max_in_flight = 1;
    config.http.request_body_bytes = 8;
    let server = Server::start(config).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/sse", server.address);
    assert_eq!(
        client
            .post(&url)
            .body("123456789")
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    let mut response = client.get(&url).send().await.unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap(), "first\n");
    drop(response);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut second =
        tokio::time::timeout(std::time::Duration::from_secs(2), client.get(&url).send())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(second.status(), 200);
    assert_eq!(second.chunk().await.unwrap().unwrap(), "first\n");
    drop(second);
    server.shutdown().await.unwrap();
}

#[path = "fixtures/upstream/mod.rs"]
mod upstream_fixture;
use gateway::network::{Budget, FetchRequest, HttpLimits, NativeBody, Network};
fn outgoing(url: String, proxy: String) -> FetchRequest {
    FetchRequest {
        url,
        proxy,
        method: "GET".into(),
        headers: vec![],
        redirect: "follow".into(),
    }
}
async fn drain(body: &NativeBody) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(chunk) = body.read().await.unwrap() {
        out.extend_from_slice(&chunk.bytes);
    }
    out
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_https_connect_socks_auth_dns_and_pooling() {
    use std::sync::atomic::Ordering;
    use upstream_fixture::{Fixture, Mode};
    let origin = Fixture::start(Mode::Http, false).await;
    let http_proxy = Fixture::start(Mode::Proxy, false).await;
    let https_proxy = Fixture::start(Mode::Proxy, true).await;
    let socks = Fixture::start(Mode::Socks, false).await;
    let tls_origin = Fixture::start(Mode::Http, true).await;
    let roots = [
        https_proxy.certificate.as_ref().unwrap(),
        tls_origin.certificate.as_ref().unwrap(),
    ]
    .into_iter()
    .map(|v| reqwest::Certificate::from_der(v).unwrap())
    .collect();
    let net = Network::new(HttpLimits::default()).with_roots(roots);
    for proxy in [
        "direct".into(),
        format!("http://user:pass@{}", http_proxy.address),
        format!("https://user:pass@{}", https_proxy.address),
        format!("socks5://user:pass@{}", socks.address),
        format!("socks5h://user:pass@{}", socks.address),
    ] {
        let response = net
            .fetch(
                outgoing(
                    format!("http://localhost:{}/ok", origin.address.port()),
                    proxy.clone(),
                ),
                None,
                Budget::new(1 << 20),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", proxy.split(":").next().unwrap()));
        assert_eq!(
            drain(&response.body).await,
            if proxy.starts_with("http") {
                b"proxied".as_slice()
            } else {
                b"fixture".as_slice()
            }
        );
    }
    assert_ne!(socks.socks_types.lock().unwrap()[0], 3);
    assert_eq!(socks.socks_types.lock().unwrap()[1], 3);
    for _ in 0..2 {
        let response = net
            .fetch(
                outgoing(
                    format!("https://localhost:{}/ok", tls_origin.address.port()),
                    format!("http://user:pass@{}", http_proxy.address),
                ),
                None,
                Budget::new(1 << 20),
            )
            .await
            .unwrap();
        assert_eq!(drain(&response.body).await, b"fixture");
    }
    assert!(
        http_proxy
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.starts_with("CONNECT "))
    );
    assert_eq!(
        tls_origin.connections.load(Ordering::SeqCst),
        1,
        "TLS connection should be reused"
    );
    assert!(
        tls_origin
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.contains("HTTP/2"))
    );
    assert!(
        Network::new(HttpLimits::default())
            .fetch(
                outgoing(
                    format!("https://localhost:{}/ok", tls_origin.address.port()),
                    "direct".into()
                ),
                None,
                Budget::new(1 << 20)
            )
            .await
            .is_err(),
        "untrusted certificate must fail"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirect_modes_replay_sensitive_headers_and_headers_timeout() {
    use upstream_fixture::{Fixture, Mode};
    let a = Fixture::start(Mode::Http, false).await;
    let b = Fixture::start(Mode::Http, false).await;
    let net = Network::new(HttpLimits::default());
    let mut req = outgoing(format!("http://{}/redirect", a.address), "direct".into());
    req.method = "POST".into();
    req.headers = vec![
        ("authorization".into(), "fixture-secret".into()),
        ("x-location".into(), format!("http://{}/echo", b.address)),
    ];
    let budget = Budget::new(1 << 20);
    let body = NativeBody::bytes(bytes::Bytes::from_static(b"replayed"), budget.clone()).unwrap();
    let response = net.fetch(req, Some(body), budget).await.unwrap();
    assert_eq!(drain(&response.body).await, b"replayed");
    assert!(!b.requests.lock().unwrap()[0].contains("fixture-secret"));
    for (mode, expected) in [("manual", Some(307)), ("error", None)] {
        let mut req = outgoing(format!("http://{}/redirect", a.address), "direct".into());
        req.redirect = mode.into();
        let result = net.fetch(req, None, Budget::new(1 << 20)).await;
        match expected {
            Some(status) => assert_eq!(result.unwrap().status, status),
            None => assert!(result.is_err()),
        }
    }
    assert!(
        net.fetch(
            outgoing(format!("http://{}/loop", a.address), "direct".into()),
            None,
            Budget::new(1 << 20)
        )
        .await
        .is_err()
    );
    let net = Network::new(HttpLimits {
        headers_timeout_ms: 20,
        ..Default::default()
    });
    assert!(
        net.fetch(
            outgoing(format!("http://{}/headers", a.address), "direct".into()),
            None,
            Budget::new(1 << 20)
        )
        .await
        .is_err()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downstream_disconnect_closes_upstream_within_100ms() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (closed, rx) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(socket.read_u8().await.unwrap());
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/event-stream\r\n\r\n6\r\nfirst\n\r\n").await.unwrap();
        let _ = socket.read_u8().await;
        let _ = closed.send(());
    });
    let dir = tempfile::tempdir().unwrap();
    support::install(
        dir.path(),
        "proxy",
        &format!("export default {{async fetch(){{return fetch('http://{address}/sse')}}}}"),
        "",
    )
    .await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let mut response = reqwest::get(format!("http://{}/proxy", server.address))
        .await
        .unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap(), "first\n");
    drop(response);
    tokio::time::timeout(std::time::Duration::from_millis(100), rx)
        .await
        .expect("upstream not closed within 100 ms")
        .unwrap();
    fixture.await.unwrap();
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmed_kv_commit_survives_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"kv",r#"export default {async fetch(r,e){if(r.method==='PUT'){await e.CONFIG.put('durable','confirmed');return new Response('committed')}return new Response(await e.CONFIG.get('durable'))}}"#,"[[kv_namespaces]]\nbinding='CONFIG'\nid='stable'").await;
    let mut process = support::Process::start(dir.path(), "");
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .put(format!("{}/kv", process.base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "committed"
    );
    process.child.kill().unwrap();
    process.child.wait().unwrap();
    let mut restarted = support::Process::start(dir.path(), "");
    assert_eq!(
        client
            .get(format!("{}/kv", restarted.base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "confirmed"
    );
    restarted.stop();
}
#[tokio::test]
async fn native_buffer_budget_follows_actual_transport_references() {
    let budget = Budget::new(128 << 10);
    let body = NativeBody::bytes(vec![0; 128 << 10].into(), budget.clone()).unwrap();
    assert_eq!(budget.used(), 128 << 10);
    let fork = body.fork().unwrap().unwrap();
    assert_eq!(budget.used(), 128 << 10);
    drop(fork);
    let first = body.read().await.unwrap().unwrap();
    assert_eq!(first.bytes.len(), 64 << 10);
    let retained = first.bytes.clone();
    drop(first);
    let second = body.read().await.unwrap().unwrap();
    drop(second);
    assert!(body.read().await.unwrap().is_none());
    assert_eq!(budget.used(), 128 << 10);
    assert!(NativeBody::bytes(vec![0; 1].into(), budget.clone()).is_err());
    drop(retained);
    assert_eq!(budget.used(), 0);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_tee_branch_hits_memory_limit_without_poisoning_worker() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"tee",r#"export default {async fetch(r){if(r.url.endsWith('/ok'))return new Response('ok');const [fast,slow]=new ReadableStream({pull(c){c.enqueue(new TextEncoder().encode('x'.repeat(65536)))}}).tee();for await(const v of fast){}return new Response('should not complete')}}"#,"").await;
    let mut config = support::config(dir.path());
    config.limits.memory_bytes = 8 << 20;
    config.limits.slice_ms = 500;
    let server = Server::start(config).await.unwrap();
    let client = reqwest::Client::new();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.get(format!("http://{}/tee", server.address)).send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 500);
    assert_eq!(
        client
            .get(format!("http://{}/tee/ok", server.address))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_admission_and_shutdown_with_active_sse_and_wait_until() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"busy",r#"export default {fetch(r,e,ctx){ctx.waitUntil(new Promise(()=>{}));return new Response(new ReadableStream({start(c){c.enqueue(new Uint8Array([1]))},pull(){return new Promise(()=>{})}}))}}"#,"").await;
    let mut config = support::config(dir.path());
    config.limits.max_in_flight = 1;
    config.http.drain_timeout_ms = 100;
    let server = Server::start(config).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/busy", server.address);
    let mut first = client.get(&url).send().await.unwrap();
    assert!(first.chunk().await.unwrap().is_some());
    let pending = tokio::spawn({
        let client = client.clone();
        let url = url.clone();
        async move { client.get(url).send().await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(client.get(&url).send().await.unwrap().status(), 503);
    let start = std::time::Instant::now();
    tokio::time::timeout(std::time::Duration::from_secs(2), server.shutdown())
        .await
        .expect("shutdown hung")
        .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    drop(first);
    let _ = pending.await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reinstall_cancels_old_stream_before_publishing_new_code() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"replace",r#"export default {fetch(){return new Response(new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('old'))},pull(){return new Promise(()=>{})}}))}}"#,"").await;
    let mut process = support::Process::start(dir.path(), "[http]\ndrain_timeout_ms=150");
    let client = reqwest::Client::new();
    let url = format!("{}/replace", process.base);
    let mut old = client.get(&url).send().await.unwrap();
    assert_eq!(old.chunk().await.unwrap().unwrap(), "old");
    let installed = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client
            .post(format!("{}/__gateway/plugins", process.base))
            .bearer_auth("fixture-admin-token")
            .header("content-type", "application/zip")
            .body(support::package(
                "replace",
                "export default {fetch(){return new Response('new')}}",
                "",
            ))
            .send(),
    )
    .await
    .expect("reinstall waiting for old engine")
    .unwrap();
    assert_eq!(installed.status(), 200);
    assert_eq!(
        client.get(&url).send().await.unwrap().text().await.unwrap(),
        "new"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), old.chunk())
            .await
            .unwrap()
            .is_err()
    );
    process.stop();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_and_null_body_statuses_do_not_send_body() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"empty",r#"export default {fetch(r){const code=Number(new URL(r.url).searchParams.get('status')||200);return new Response(code===200?'hello':null,{status:code})}}"#,"").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/empty", server.address);
    let head = client.head(&url).send().await.unwrap();
    assert_eq!(head.status(), 200);
    assert!(head.headers().get("content-length").is_none());
    assert!(head.bytes().await.unwrap().is_empty());
    for code in [204, 304] {
        let r = client
            .get(format!("{url}?status={code}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), code);
        assert!(r.bytes().await.unwrap().is_empty());
    }
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_uses_idle_timeout_without_a_total_sse_deadline() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn fixture(stall: bool) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            if stall {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                for _ in 0..6 {
                    socket.write_all(b"1\r\nx\r\n").await.unwrap();
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                socket.write_all(b"0\r\n\r\n").await.unwrap();
            }
        });
        (address, task)
    }
    let net = Network::new(HttpLimits {
        headers_timeout_ms: 80,
        idle_timeout_ms: 80,
        ..Default::default()
    });
    let (address, task) = fixture(false).await;
    let start = std::time::Instant::now();
    let response = net
        .fetch(
            outgoing(format!("http://{address}"), "direct".into()),
            None,
            Budget::new(1 << 20),
        )
        .await
        .unwrap();
    assert_eq!(drain(&response.body).await, b"xxxxxx");
    assert!(start.elapsed() > std::time::Duration::from_millis(100));
    task.await.unwrap();
    let (address, task) = fixture(true).await;
    let response = net
        .fetch(
            outgoing(format!("http://{address}"), "direct".into()),
            None,
            Budget::new(1 << 20),
        )
        .await
        .unwrap();
    assert!(response.body.read().await.is_err());
    task.abort();
}
