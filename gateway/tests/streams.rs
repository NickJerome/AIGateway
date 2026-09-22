mod support;
use gateway::http::Server;
async fn evaluate(source: &str) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    support::install(
        dir.path(),
        "test",
        &format!("export default {{async fetch(){{{source}}}}}"),
        "",
    )
    .await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        reqwest::get(format!("http://{}/test", server.address)),
    )
    .await
    .unwrap()
    .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    server.shutdown().await.unwrap();
    assert_eq!(status, 200, "{text}");
    serde_json::from_str(&text).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_transform_utf8_and_flush() {
    let result=evaluate(r#"
        const encoded=new TextEncoder().encode('a€\r\nb');
        const source=new ReadableStream({start(c){c.enqueue(encoded.slice(0,2));c.enqueue(encoded.slice(2));c.close()}});
        const transformed=source.pipeThrough(new TextDecoderStream()).pipeThrough(new TransformStream({async transform(x,c){await new Promise(r=>setTimeout(r,1));c.enqueue(x.toUpperCase())},flush(c){c.enqueue('!')}}));
        let text='';for await(const x of transformed)text+=x;
        return Response.json({text,standard:transformed instanceof ReadableStream});
    "#).await;
    assert_eq!(result["text"], "A€\r\nB!");
    assert_eq!(result["standard"], true);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tee_cancel_and_pipe_prevent_options() {
    let result=evaluate(r#"
        let reason;
        const source=new ReadableStream({cancel(r){reason=r}});const [a,b]=source.tee();
        const first=a.cancel('a');await Promise.resolve();const single=reason===undefined;
        await Promise.all([first,b.cancel('b')]);
        let closed=false;const out=new WritableStream({close(){closed=true}});
        await new ReadableStream({start(c){c.enqueue(1);c.close()}}).pipeTo(out,{preventClose:true});
        const released=!out.locked;await out.close();return Response.json({single,reason,released,closed});
    "#).await;
    assert_eq!(
        result,
        serde_json::json!({"single":true,"reason":["a","b"],"released":true,"closed":true})
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_lock_used_clone_and_kv_rejections() {
    let result=evaluate(r#"
        const original=new Response('value');const clone=original.clone();
        const reader=original.body.getReader();let locked=false;try{original.clone()}catch{locked=true}
        const before=original.bodyUsed;await reader.read();const after=original.bodyUsed;reader.releaseLock();
        let reuse=false;try{await original.text()}catch{reuse=true}
        const text=await clone.text();return Response.json({locked,before,after,reuse,text});
    "#).await;
    assert_eq!(
        result,
        serde_json::json!({"locked":true,"before":false,"after":true,"reuse":true,"text":"value"})
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workerd_streams_and_body_subset() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "{}\n{}",
        include_str!("fixtures/workerd/node-assert-shim.mjs"),
        include_str!("fixtures/workerd/spec02.js")
    );
    support::install(dir.path(), "workerd", &source, "").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let response = reqwest::get(format!("http://{}/workerd", server.address))
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    server.shutdown().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["passed"],
        18
    );
}
