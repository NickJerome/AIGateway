mod support;
use gateway::http::Server;
use std::time::Duration;
const BINDINGS: &str = "[[durable_objects.bindings]]\nname='OBJECTS'\nclass_name='State'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['State']";
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_output_gates_and_unawaited_write_failure() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"gates",r#"
 import {DurableObject} from 'cloudflare:workers';
 export class State extends DurableObject {
  async write(){this.ctx.storage.put('x',1);this.ctx.storage.put('x',2);return 'done';}
  async read(){return await this.ctx.storage.get('x');}
  async fail(){this.ctx.storage.put('x',9);return 'must-not-escape';}
 }
 export default {async fetch(r,env){const method=new URL(r.url).pathname.split('/').pop();try{return Response.json(await env.OBJECTS.get(env.OBJECTS.idFromName(method==='fail'?'bad':'one'))[method]());}catch(e){return new Response(e.message,{status:502});}}}
 "#,BINDINGS).await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let base = format!("http://{}/gates", server.address);
    let conn = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let task = tokio::spawn({
        let c = client.clone();
        let url = format!("{base}/write");
        async move { c.get(url).send().await.unwrap().text().await.unwrap() }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!task.is_finished(), "output escaped before FULL commit");
    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(task.await.unwrap(), "\"done\"");
    assert_eq!(
        client
            .get(format!("{base}/read"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "2"
    );
    conn.execute_batch("CREATE TRIGGER fail_do BEFORE INSERT ON do_values BEGIN SELECT RAISE(ABORT,'injected write failure'); END").unwrap();
    let response = client.get(format!("{base}/fail")).send().await.unwrap();
    assert_eq!(response.status(), 502);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("injected write failure")
    );
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn objects_interleave_network_wait_and_caller_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"gates",r#"
 import {DurableObject} from 'cloudflare:workers';
 export class State extends DurableObject {
  constructor(ctx,env){super(ctx,env);this.n=0;}
  async token(){if(!this.work)this.work=(async()=>{this.n++;await new Promise(r=>setTimeout(r,250));await this.ctx.storage.put('n',this.n);return this.n;})();return this.work;}
  async quick(){return this.ctx.id.toString();}
  async nested(){return this.env.OBJECTS.get(this.env.OBJECTS.idFromName('b')).quick();}
 }
 export default {async fetch(r,env){const u=new URL(r.url);const name=u.searchParams.get('name')??'a';const op=u.pathname.split('/').pop();return Response.json(await env.OBJECTS.get(env.OBJECTS.idFromName(name))[op]());}}
 "#,BINDINGS).await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let base = format!("http://{}/gates", server.address);
    let cancelled = tokio::spawn({
        let c = client.clone();
        let url = format!("{base}/token");
        async move { c.get(url).send().await }
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    let quick = tokio::time::timeout(
        Duration::from_millis(150),
        client.get(format!("{base}/nested?name=c")).send(),
    )
    .await
    .expect("another object blocked")
    .unwrap();
    assert!(quick.status().is_success());
    cancelled.abort();
    assert_eq!(
        client
            .get(format!("{base}/token"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "1"
    );
    server.shutdown().await.unwrap();
}

#[path = "fixtures/mock/mod.rs"]
mod mock;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialization_can_await_its_own_network_and_timer_without_opening_rpc_gate() {
    let upstream = mock::Mock::start(|_, _, _| async {
        tokio::time::sleep(Duration::from_millis(60)).await;
        (200, vec![], "ready".into())
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r#"import {{DurableObject}} from 'cloudflare:workers';export class State extends DurableObject {{constructor(ctx,e){{super(ctx,e);ctx.blockConcurrencyWhile(async()=>{{const r=await ctx.fetch('{}');await new Promise(resolve=>setTimeout(resolve,10));this.value=await r.text();}});ctx.blockConcurrencyWhile(async()=>{{await new Promise(resolve=>setTimeout(resolve,20));this.value+=' second';}});}}read(){{return this.value??'too early';}}}}export default {{async fetch(r,e){{return new Response(await e.OBJECTS.get(e.OBJECTS.idFromName('one')).read());}}}}"#,
        upstream.base
    );
    support::install(dir.path(), "init-net", &source, BINDINGS).await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let results = futures::future::join_all((0..4).map(|_| {
        client
            .get(format!("http://{}/init-net/", server.address))
            .send()
    }))
    .await;
    for result in results {
        assert_eq!(result.unwrap().text().await.unwrap(), "ready second");
    }
    server.shutdown().await.unwrap();
}
