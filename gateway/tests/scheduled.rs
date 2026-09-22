mod support;
use gateway::{
    http::Server,
    scheduled::{Cron, Schedule},
};
#[test]
fn utc_calendar_and_clock_jumps() {
    for value in [
        "* * * *",
        "60 * * * *",
        "* 24 * * *",
        "* * 0 * *",
        "* * * 13 *",
        "* * * * 8",
        "*/0 * * * *",
        "TZ=UTC * * * * *",
    ] {
        assert!(Cron::parse(value).is_err(), "{value}");
    }
    let leap = Cron::parse("0 0 29 2 *").unwrap();
    assert_eq!(leap.next(1709164799999).unwrap(), 1709164800000); // 2024-02-29
    let sunday = Cron::parse("0 0 * * 0").unwrap();
    let seven = Cron::parse("0 0 * * 7").unwrap();
    assert_eq!(
        sunday.next(1709164800000).unwrap(),
        seven.next(1709164800000).unwrap()
    );
    let mut clock = Schedule::default();
    assert_eq!(clock.advance(60000), None);
    assert_eq!(clock.advance(120000), Some(120000));
    assert_eq!(clock.advance(60000), None);
    assert_eq!(clock.advance(120000), None);
    assert_eq!(clock.advance(600000), Some(600000));
    assert!(Cron::parse("0 0 29 2 0").unwrap().matches(1708819200000)); // Sunday even if not day 29
}
#[tokio::test]
async fn real_scheduled_entry_fresh_globals_and_no_overlap() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"cron",r#"globalThis.count=0;export default {async scheduled(event,env,ctx){count++;await env.KV.put('seen',JSON.stringify({count,event}));ctx.waitUntil(new Promise(resolve=>setTimeout(resolve,150)));},async fetch(r,env){return new Response(await env.KV.get('seen')??'missing');}}"#,"[[kv_namespaces]]\nbinding='KV'\nid='cron-kv'\n[triggers]\ncrons=['* * * * *','* * * * *']").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/cron/", server.address);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let at = (now / 60000 + 1) * 60000;
    server.tick(at);
    let mut seen = String::new();
    for _ in 0..100 {
        seen = client.get(&url).send().await.unwrap().text().await.unwrap();
        if seen != "missing" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let value: serde_json::Value = serde_json::from_str(&seen).unwrap();
    assert_eq!(value["count"], 1);
    assert_eq!(value["event"]["scheduledTime"], at);
    server.tick(at + 60000); // active waitUntil: skip, no backlog
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        client.get(&url).send().await.unwrap().text().await.unwrap(),
        seen
    );
    server.tick(at + 120000);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let value: serde_json::Value =
        serde_json::from_str(&client.get(&url).send().await.unwrap().text().await.unwrap())
            .unwrap();
    assert_eq!(value["count"], 1);
    assert_eq!(value["event"]["scheduledTime"], at + 120000);
    server.shutdown().await.unwrap();
}
