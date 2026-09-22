mod support;
use gateway::{
    durable::{object_id, owner},
    http::Server,
    plugins::PluginManager,
    storage::Database,
};
#[tokio::test]
async fn migration_history_and_stable_identity() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 2, 8).unwrap();
    let mut manager =
        PluginManager::open(dir.path().join("plugins"), db.clone(), Default::default())
            .await
            .unwrap();
    let source = "import {DurableObject} from 'cloudflare:workers';export class State extends DurableObject {} export default {fetch(){return new Response('ok')}}";
    let extra = "[[durable_objects.bindings]]\nname='STATE'\nclass_name='State'\n[[migrations]]\ntag='one'\nnew_sqlite_classes=['State']";
    manager
        .install(support::package("identity", source, extra))
        .await
        .unwrap();
    manager
        .install(support::package("identity", source, extra))
        .await
        .unwrap();
    manager.uninstall("identity").await.unwrap();
    manager
        .install(support::package("identity", source, extra))
        .await
        .unwrap();
    let id = object_id("identity:State", "中文-name");
    assert_eq!(id, object_id("identity:State", "中文-name"));
    assert_ne!(id, object_id("another:State", "中文-name"));
    assert!(owner(&id, 2) < 2 && owner(&id, 3) < 3);
    let counts = db
        .run(|c| {
            Ok(c.query_row("SELECT count(*) FROM do_migrations", [], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .await
        .unwrap();
    assert_eq!(counts, 1);
    assert!(
        manager
            .install(support::package(
                "identity",
                source,
                &extra.replace("tag='one'", "tag='changed'")
            ))
            .await
            .is_err()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_input_gate_serializes_read_modify_write_and_reconstructs_failed_init() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"counter",r#"
 import {DurableObject} from 'cloudflare:workers';
 let initialized=0;
 export class Counter extends DurableObject {
  constructor(ctx,env){super(ctx,env);ctx.blockConcurrencyWhile(async()=>{if(++initialized===1)throw new Error('first init fails');await ctx.storage.get('n');});}
  async add(){const n=await this.ctx.storage.get('n')??0;await this.ctx.storage.put('n',n+1);return n+1;}
 }
 export default {async fetch(r,env){try{return Response.json(await env.STATE.get(env.STATE.idFromName('one')).add());}catch(e){return new Response(e.message,{status:502});}}}
 "#,"[[durable_objects.bindings]]\nname='STATE'\nclass_name='Counter'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['Counter']").await;
    let mut config = support::config(dir.path());
    config.limits.max_in_flight = 64;
    let server = Server::start(config).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/counter/", server.address);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 502);
    let values = futures::future::join_all((0..40).map(|_| {
        let client = client.clone();
        let url = url.clone();
        async move {
            let response = client.get(url).send().await.unwrap();
            let status = response.status();
            let value = response.text().await.unwrap();
            assert_eq!(status, 200, "{value}");
            value.parse::<usize>().unwrap()
        }
    }))
    .await;
    let mut values = values;
    values.sort();
    assert_eq!(values, (1..=40).collect::<Vec<_>>());
    server.shutdown().await.unwrap();
}

#[test]
fn export_validation_rejects_missing_classes_without_executing_user_code() {
    gateway::bytecode::validate_exports("for(;;) {} export class Yes {}", &["Yes".into()]).unwrap();
    assert!(
        gateway::bytecode::validate_exports("export class Yes {}", &["Missing".into()]).is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oom_discards_do_engine_and_recovers_confirmed_storage() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(), "oom", r#"
import {DurableObject} from 'cloudflare:workers';
export class State extends DurableObject {
 async save(){await this.ctx.storage.put('saved','confirmed');return 'saved';}
 crash(){this.bytes=new ArrayBuffer(128*1024*1024);return 'unexpected';}
 async read(){return await this.ctx.storage.get('saved');}
}
export default {async fetch(r,e){try{const s=e.STATE.get(e.STATE.idFromName('one'));return new Response(await s[new URL(r.url).pathname.split('/').pop()]());}catch(e){return new Response(String(e),{status:502});}}}
"#, "[[durable_objects.bindings]]\nname='STATE'\nclass_name='State'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['State']").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://{}/oom", server.address);
    assert_eq!(
        client
            .get(format!("{base}/save"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "saved"
    );
    assert_eq!(
        client
            .get(format!("{base}/crash"))
            .send()
            .await
            .unwrap()
            .status(),
        502
    );
    assert_eq!(
        client
            .get(format!("{base}/read"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "confirmed"
    );
    server.shutdown().await.unwrap();
}
