mod support;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};
fn rss(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual release WebSocket capacity measurement (20 seconds)"]
async fn native_and_accepted_websocket_capacity() {
    let upstream_dir = tempfile::tempdir().unwrap();
    let pair = "const p=new WebSocketPair();p[1].accept();p[1].onmessage=e=>p[1].send(e.data);return new Response(null,{status:101,webSocket:p[0]});";
    support::install(
        upstream_dir.path(),
        "echo",
        &format!("export default {{fetch(){{{pair}}}}}"),
        "",
    )
    .await;
    let upstream = gateway::http::Server::start(support::config(upstream_dir.path()))
        .await
        .unwrap();
    for mode in ["native", "accepted"] {
        let dir = tempfile::tempdir().unwrap();
        let code = if mode == "native" {
            format!(
                "export default {{async fetch(r,e,c){{const up=await c.fetch('http://{}/echo/',{{headers:{{Upgrade:'websocket'}}}});return new Response(null,{{status:101,webSocket:up.webSocket}});}}}}",
                upstream.address
            )
        } else {
            format!("export default {{fetch(){{{pair}}}}}")
        };
        support::install(dir.path(), "capacity-ws", &code, "").await;
        let mut process = support::Process::start(dir.path(), "max_in_flight=64");
        let idle = rss(process.child.id());
        let mut sockets = Vec::new();
        for _ in 0..32 {
            sockets.push(
                connect_async(format!(
                    "{}/capacity-ws/",
                    process.base.replacen("http:", "ws:", 1)
                ))
                .await
                .unwrap()
                .0,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let client = reqwest::Client::new();
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
        assert_eq!(metrics["requests"], if mode == "native" { 0 } else { 32 });
        assert_eq!(metrics["websockets"], 32);
        let started = std::time::Instant::now();
        let mut frames = 0u64;
        let mut peak = rss(process.child.id());
        let payload = Message::Binary(vec![7; 256].into());
        while started.elapsed() < std::time::Duration::from_secs(10) {
            futures::future::join_all(sockets.iter_mut().map(|ws| {
                let payload = payload.clone();
                async move {
                    ws.send(payload.clone()).await.unwrap();
                    assert_eq!(ws.next().await.unwrap().unwrap(), payload);
                }
            }))
            .await;
            frames += 32;
            peak = peak.max(rss(process.child.id()));
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        println!(
            "WS_CAPACITY {}",
            json!({"mode":mode,"clients":32,"seconds":started.elapsed().as_secs_f64(),"message_bytes":256,"echoes":frames,"idle_rss_bytes":idle,"peak_rss_bytes":peak,"active_metrics":metrics,"build":if cfg!(debug_assertions){"debug"}else{"release"}})
        );
        for ws in &mut sockets {
            let _ = ws.close(None).await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), ws.next()).await;
        }
        drop(sockets);
        process.stop();
    }
    upstream.shutdown().await.unwrap();
}
