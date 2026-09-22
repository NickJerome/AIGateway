mod support;
use gateway::http::Server;
use std::time::Duration;
const BINDINGS: &str = "[[durable_objects.bindings]]\nname='COUNTER'\nclass_name='Counter'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['Counter']";
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_object_shared_refresh_clone_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"
import {DurableObject} from 'cloudflare:workers';
export class Counter extends DurableObject {
 constructor(ctx,env){super(ctx,env);this.refreshes=0;ctx.blockConcurrencyWhile(async()=>{this.n=await ctx.storage.get('n')??0;});}
 async token(){if(!this.refreshing)this.refreshing=(async()=>{this.refreshes++;await new Promise(r=>setTimeout(r,80));await this.ctx.storage.put('n',++this.n);return {n:this.n,refreshes:this.refreshes};})();return this.refreshing;}
 async echo(value){await this.ctx.storage.put('value',value);return this.ctx.storage.get('value');}
}
export default {async fetch(r,env){const stub=env.COUNTER.get(env.COUNTER.idFromName('same'));if(new URL(r.url).pathname.endsWith('clone')){const x={n:-0,b:42n,map:new Map(),bytes:new Uint16Array([1,500])};x.self=x;x.map.set(x,new Set([x]));const y=await stub.echo(x);return Response.json({cycle:y===y.self,map:y.map.get(y).has(y),zero:Object.is(y.n,-0),big:y.b===42n,bytes:y.bytes[1]});}return Response.json(await stub.token());}}
"#;
    support::install(dir.path(), "rpc", source, BINDINGS).await;
    let mut config = support::config(dir.path());
    config.threads = 2;
    config.limits.max_in_flight = 128;
    let server = Server::start(config.clone()).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/rpc/token", server.address);
    let replies = tokio::time::timeout(
        Duration::from_secs(10),
        futures::future::join_all((0..100).map(|_| {
            let c = client.clone();
            let u = url.clone();
            async move { c.get(u).send().await.unwrap().text().await.unwrap() }
        })),
    )
    .await
    .expect("RPC deadlock");
    for reply in replies {
        assert_eq!(reply, "{\"n\":1,\"refreshes\":1}");
    }
    let clone = client
        .get(format!("http://{}/rpc/clone", server.address))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        clone,
        "{\"cycle\":true,\"map\":true,\"zero\":true,\"big\":true,\"bytes\":500}"
    );
    server.shutdown().await.unwrap();
    let server = Server::start(config).await.unwrap();
    let body = client
        .get(format!("http://{}/rpc/token", server.address))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "{\"n\":2,\"refreshes\":1}");
    server.shutdown().await.unwrap();
}
