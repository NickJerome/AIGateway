#[path = "fixtures/mock/mod.rs"]
mod mock;
mod support;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zip_login_shared_refresh_sse_stickiness_admin_and_restart() {
    let refreshes = Arc::new(AtomicUsize::new(0));
    let issued = Arc::new(AtomicUsize::new(0));
    let responses = Arc::new(AtomicUsize::new(0));
    let upstream=mock::Mock::start({let refreshes=refreshes.clone();move|path,headers,body|{let refreshes=refreshes.clone();let issued=issued.clone();let responses=responses.clone();async move{
  let data=if path.ends_with("/usercode"){let id=issued.fetch_add(1,Ordering::SeqCst);json!({"device_auth_id":format!("device-{id}"),"user_code":"TEST","interval":1,"expires_in":900})}
  else if path.ends_with("/deviceauth/token"){json!({"authorization_code":"code","code_verifier":"verifier"})}
  else if path=="/oauth/token"{let text=String::from_utf8_lossy(&body);if text.contains("refresh_token"){refreshes.fetch_add(1,Ordering::SeqCst);tokio::time::sleep(std::time::Duration::from_millis(80)).await;json!({"access_token":"fresh-token","expires_in":3600})}else{json!({"access_token":"expired-token","refresh_token":"fixture-refresh","expires_in":0})}}
  else if path=="/responses"{
   assert_eq!(headers.get("authorization").unwrap(),"Bearer fresh-token");let input:Value=serde_json::from_slice(&body).unwrap();let id=format!("resp-{}",responses.fetch_add(1,Ordering::SeqCst));let value=json!({"id":id,"model":"test","status":"completed","output":[],"usage":{"input_tokens":3,"output_tokens":2}});
   if input["stream"]==true{return (200,vec![("content-type".into(),"text/event-stream".into())],format!("event: response.created\r\ndata: {}\r\n\r\nevent: response.completed\ndata: {}\n\n",json!({"type":"response.created","response":{"id":id}}),json!({"type":"response.completed","response":value})));}value
  }else{return (404,vec![],"missing".into())};(200,vec![("content-type".into(),"application/json".into())],data.to_string())
 }}}).await;
    let dir = tempfile::tempdir().unwrap();
    let mut process = support::Process::start(dir.path(), "max_in_flight=128");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap();
    let zip = include_bytes!("../examples/codex-pool/codex-pool.zip");
    let response = client
        .post(format!("{}/__gateway/plugins", process.base))
        .bearer_auth("fixture-admin-token")
        .header("content-type", "application/zip")
        .body(zip.to_vec())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 201, "{text}");
    let response=client.put(format!("{}/__gateway/plugins/codex-pool/config",process.base)).bearer_auth("fixture-admin-token").header("content-type","application/json").body(json!({"vars":{"ADMIN_TOKEN":"fixture-plugin-token","AUTH_BASE":upstream.base,"UPSTREAM":format!("{}/responses",upstream.base)}}).to_string()).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        client
            .get(format!("{}/codex/admin/accounts", process.base))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let login: Value = serde_json::from_str(
        &client
            .post(format!("{}/codex/admin/accounts/login", process.base))
            .bearer_auth("fixture-plugin-token")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(login["id"].is_string(), "{login}");
    let poll = format!(
        "{}/codex/admin/accounts/login/{}/poll",
        process.base,
        login["id"].as_str().unwrap()
    );
    let result: Value = serde_json::from_str(
        &client
            .post(&poll)
            .bearer_auth("fixture-plugin-token")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["state"], "done", "{result}");
    let output = futures::future::join_all((0..100).map(|_| {
        let client = client.clone();
        let url = format!("{}/codex/responses", process.base);
        async move {
            let r = client
                .post(url)
                .body(r#"{"model":"test","input":"hello"}"#)
                .send()
                .await
                .unwrap();
            let status = r.status();
            let text = r.text().await.unwrap();
            assert_eq!(status, 200, "{text}");
        }
    }))
    .await;
    assert_eq!(output.len(), 100);
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    let stream = client
        .post(format!("{}/codex/responses", process.base))
        .body(r#"{"model":"test","input":"hello","stream":true}"#)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(stream.contains("response.completed"), "{stream}");
    let created: Value = serde_json::from_str(
        stream
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    let id = &created["response"]["id"];
    let continuation = client
        .post(format!("{}/codex/responses", process.base))
        .body(json!({"input":"again","previous_response_id":id}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(continuation.status(), 200);
    let missing = client
        .post(format!("{}/codex/responses", process.base))
        .body(r#"{"input":"again","previous_response_id":"unknown"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 409);
    let accounts = client
        .get(format!("{}/codex/admin/accounts", process.base))
        .bearer_auth("fixture-plugin-token")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!accounts.contains("fresh-token") && !accounts.contains("fixture-refresh"));
    process.stop();
    let mut process = support::Process::start(dir.path(), "max_in_flight=128");
    let response = client
        .post(format!("{}/codex/responses", process.base))
        .body(r#"{"input":"after restart"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    process.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn account_rotation_hard_soft_affinity_reused_refresh_and_reinstall() {
    let phase = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let issued = Arc::new(AtomicUsize::new(0));
    let reused = Arc::new(AtomicUsize::new(0));
    let upstream=mock::Mock::start({let phase=phase.clone();let requests=requests.clone();let reused=reused.clone();move |path,headers,body|{let phase=phase.clone();let requests=requests.clone();let issued=issued.clone();let reused=reused.clone();async move {
        let json_header=vec![("content-type".into(),"application/json".into())];
        let value=if path.ends_with("/usercode"){let id=issued.fetch_add(1,Ordering::SeqCst);json!({"device_auth_id":id.to_string(),"user_code":"TEST","interval":1})}
        else if path.ends_with("/deviceauth/token"){let input:Value=serde_json::from_slice(&body).unwrap();json!({"authorization_code":input["device_auth_id"],"code_verifier":"verifier"})}
        else if path=="/oauth/token" {
            let fields:std::collections::HashMap<_,_>=url::form_urlencoded::parse(&body).into_owned().collect();
            if fields["grant_type"]=="refresh_token" {reused.fetch_add(1,Ordering::SeqCst);return(400,json_header,json!({"error":"refresh_token_reused"}).to_string());}
            let id=&fields["code"];json!({"access_token":format!("token-{id}"),"refresh_token":format!("refresh-{id}"),"expires_in":if id=="2"{0}else{3600}})
        } else if path=="/responses" {
            let token=headers["authorization"].to_str().unwrap().to_owned();requests.lock().unwrap().push(token.clone());
            let n=phase.load(Ordering::SeqCst);
            if token=="Bearer token-0" && n>=1 {return(429,json_header,"rate limited".into());}
            if token=="Bearer token-1" && n>=2 {return(401,json_header,"unauthorized".into());}
            json!({"id":format!("response-{n}"),"model":"m","output":[]})
        }else{return(404,vec![],"missing".into());};(200,json_header,value.to_string())
    }}}).await;
    let dir = tempfile::tempdir().unwrap();
    let mut process = support::Process::start(dir.path(), "max_in_flight=16");
    let client = reqwest::Client::new();
    let zip = include_bytes!("../examples/codex-pool/codex-pool.zip").to_vec();
    assert_eq!(
        client
            .post(format!("{}/__gateway/plugins", process.base))
            .bearer_auth("fixture-admin-token")
            .header("content-type", "application/zip")
            .body(zip.clone())
            .send()
            .await
            .unwrap()
            .status(),
        201
    );
    assert_eq!(client.put(format!("{}/__gateway/plugins/codex-pool/config",process.base)).bearer_auth("fixture-admin-token").header("content-type","application/json").body(json!({"vars":{"ADMIN_TOKEN":"plugin-fixture","AUTH_BASE":upstream.base,"UPSTREAM":format!("{}/responses",upstream.base)}}).to_string()).send().await.unwrap().status(),200);
    for _ in 0..3 {
        let value: Value = serde_json::from_str(
            &client
                .post(format!("{}/codex/admin/accounts/login", process.base))
                .bearer_auth("plugin-fixture")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
        )
        .unwrap();
        let url = format!(
            "{}/codex/admin/accounts/login/{}/poll",
            process.base,
            value["id"].as_str().unwrap()
        );
        let results = futures::future::join_all(
            (0..4).map(|_| client.post(&url).bearer_auth("plugin-fixture").send()),
        )
        .await;
        for result in results {
            assert_eq!(result.unwrap().status(), 200);
        }
    }
    let url = format!("{}/codex/responses", process.base);
    assert_eq!(
        client
            .post(&url)
            .body(r#"{"model":"m","input":"first","prompt_cache_key":"soft"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    phase.store(1, Ordering::SeqCst);
    assert_eq!(
        client
            .post(&url)
            .body(r#"{"model":"m","input":"fallback","prompt_cache_key":"soft"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        *requests.lock().unwrap(),
        vec!["Bearer token-0", "Bearer token-0", "Bearer token-1"]
    );
    let previous = requests.lock().unwrap().len();
    assert_eq!(
        client
            .post(&url)
            .body(r#"{"input":"hard","previous_response_id":"response-0"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(requests.lock().unwrap().len(), previous);
    phase.store(2, Ordering::SeqCst);
    assert_eq!(
        client
            .post(&url)
            .body(r#"{"input":"unavailable"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(reused.load(Ordering::SeqCst), 1);
    let accounts = client
        .get(format!("{}/codex/admin/accounts", process.base))
        .bearer_auth("plugin-fixture")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(accounts.contains("dead"), "{accounts}");
    assert_eq!(
        serde_json::from_str::<Value>(&accounts)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        client
            .post(format!("{}/__gateway/plugins", process.base))
            .bearer_auth("fixture-admin-token")
            .header("content-type", "application/zip")
            .body(zip)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .post(&url)
            .body(r#"{"input":"after reinstall"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(reused.load(Ordering::SeqCst), 1);
    process.stop();
}

async fn configured(dir: &std::path::Path, auth: &str, upstream: &str) -> support::Process {
    let process = support::Process::start(dir, "");
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/__gateway/plugins", process.base))
        .bearer_auth("fixture-admin-token")
        .header("content-type", "application/zip")
        .body(include_bytes!("../examples/codex-pool/codex-pool.zip").to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let response = client
        .put(format!(
            "{}/__gateway/plugins/codex-pool/config",
            process.base
        ))
        .bearer_auth("fixture-admin-token")
        .header("content-type", "application/json")
        .body(
            json!({"vars":{"ADMIN_TOKEN":"plugin-fixture","AUTH_BASE":auth,"UPSTREAM":upstream}})
                .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    process
}
async fn start_login(client: &reqwest::Client, base: &str) -> String {
    let value: Value = serde_json::from_str(
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
    format!(
        "{base}/codex/admin/accounts/login/{}/poll",
        value["id"].as_str().unwrap()
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_expiry_pending_slow_down_and_error_are_distinct() {
    let issued = Arc::new(AtomicUsize::new(0));
    let polled = Arc::new(AtomicUsize::new(0));
    let auth=mock::Mock::start({let polled=polled.clone();move |path,_,body|{let issued=issued.clone();let polled=polled.clone();async move{
        let headers=vec![("content-type".into(),"application/json".into())];
        if path.ends_with("/usercode"){let n=issued.fetch_add(1,Ordering::SeqCst);return(200,headers,json!({"device_auth_id":n.to_string(),"user_code":"TEST","interval":1,"expires_in":if n==0 {0}else{900}}).to_string());}
        let input:Value=serde_json::from_slice(&body).unwrap();if input["device_auth_id"]=="2"{return(500,headers,json!({"error":"unexpected"}).to_string());}
        let n=polled.fetch_add(1,Ordering::SeqCst);(if n==0{400}else{429},headers,json!({"error":if n==0{"authorization_pending"}else{"slow_down"}}).to_string())
    }}}).await;
    let dir = tempfile::tempdir().unwrap();
    let mut process = configured(dir.path(), &auth.base, &format!("{}/responses", auth.base)).await;
    let client = reqwest::Client::new();
    let expired = start_login(&client, &process.base).await;
    assert_eq!(
        client
            .post(expired)
            .bearer_auth("plugin-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        410
    );
    let url = start_login(&client, &process.base).await;
    let first = client
        .post(&url)
        .bearer_auth("plugin-fixture")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(first.contains("pending"), "{first}");
    let next = client
        .post(&url)
        .bearer_auth("plugin-fixture")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(next.contains("slow_down"));
    assert_eq!(polled.load(Ordering::SeqCst), 1);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let next = client
        .post(&url)
        .bearer_auth("plugin-fixture")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&next).unwrap()["retry_after"],
        6
    );
    let error = start_login(&client, &process.base).await;
    assert_eq!(
        client
            .post(error)
            .bearer_auth("plugin-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        502
    );
    process.stop();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codex_sse_disconnect_closes_upstream_within_100ms() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = mock::Mock::start(|path, _, _| async move {
        let value = if path.ends_with("/usercode") {
            json!({"device_auth_id":"one","user_code":"TEST","interval":1})
        } else if path.ends_with("/deviceauth/token") {
            json!({"authorization_code":"code","code_verifier":"v"})
        } else {
            json!({"access_token":"mock","refresh_token":"mock","expires_in":3600})
        };
        (
            200,
            vec![("content-type".into(), "application/json".into())],
            value.to_string(),
        )
    })
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let (closed, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = vec![];
        while !head.ends_with(b"\r\n\r\n") {
            head.push(socket.read_u8().await.unwrap());
        }
        let text = String::from_utf8(head).unwrap().to_lowercase();
        if text.contains("transfer-encoding: chunked") {
            loop {
                let mut line = vec![];
                while !line.ends_with(b"\r\n") {
                    line.push(socket.read_u8().await.unwrap());
                }
                let n = usize::from_str_radix(String::from_utf8_lossy(&line).trim(), 16).unwrap();
                let mut data = vec![0; n + 2];
                socket.read_exact(&mut data).await.unwrap();
                if n == 0 {
                    break;
                }
            }
        } else {
            let n = text
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap_or("0")
                .parse()
                .unwrap();
            socket.read_exact(&mut vec![0; n]).await.unwrap();
        }
        let frame =
            b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"stream-id\"}}\n\n";
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",frame.len()).as_bytes()).await.unwrap();
        socket.write_all(frame).await.unwrap();
        socket.write_all(b"\r\n").await.unwrap();
        let _ = socket.read_u8().await;
        let _ = closed.send(());
    });
    let dir = tempfile::tempdir().unwrap();
    let mut process = configured(
        dir.path(),
        &auth.base,
        &format!("http://{upstream}/responses"),
    )
    .await;
    let client = reqwest::Client::new();
    let poll = start_login(&client, &process.base).await;
    assert_eq!(
        client
            .post(poll)
            .bearer_auth("plugin-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut response = client
        .post(format!("{}/codex/responses", process.base))
        .body(r#"{"input":"stream","stream":true}"#)
        .send()
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&response.chunk().await.unwrap().unwrap()).contains("stream-id")
    );
    drop(response);
    tokio::time::timeout(std::time::Duration::from_millis(100), rx)
        .await
        .expect("Codex transform retained upstream after cancellation")
        .unwrap();
    task.await.unwrap();
    let metrics: Value = serde_json::from_str(
        &client
            .get(format!("{}/__gateway/metrics", process.base))
            .bearer_auth("fixture-admin-token")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(metrics["requests"], 0);
    process.stop();
}
