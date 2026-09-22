mod support;
use gateway::{http::Server, ir::Document};
#[test]
fn offset_edits_preserve_duplicates_formatting_and_snapshot() {
    let raw=bytes::Bytes::from_static(br#" { "model":"old", "number":1.0,"zero":-0,"escape":"\u00e9", "model" : "last", "nest":{"x":1,"x":2}, "array":[1,2] } "#);
    let mut doc = Document::parse(raw.clone()).unwrap();
    assert_eq!(doc.snapshot().unwrap().concat(), raw);
    doc.edit(&["model".into()], Some(serde_json::json!("new")))
        .unwrap();
    let first = doc.snapshot().unwrap().concat();
    assert_eq!(
        String::from_utf8(first.clone()).unwrap(),
        String::from_utf8(raw.to_vec())
            .unwrap()
            .replace("\"last\"", "\"new\"")
    );
    doc.edit(&["nest".into(), "x".into()], None).unwrap();
    doc.edit(
        &["array".into(), "length".into()],
        Some(serde_json::json!(1)),
    )
    .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&doc.snapshot().unwrap().concat()).unwrap();
    assert_eq!(parsed["nest"], serde_json::json!({}));
    assert_eq!(parsed["array"], serde_json::json!([1]));
    assert!(String::from_utf8(first).unwrap().contains("\"x\":2"));
}
#[tokio::test]
async fn four_dialects_twenty_shapes_and_native_replay() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"ir",r#"export default {async fetch(request,env){const url=new URL(request.url);const ir=env.IR.from(request,url.searchParams.get('dialect'));if(url.searchParams.has('model'))ir.model='changed';return new Response(env.IR.to(ir));}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    for (dialect, shape) in [
        (
            "anthropic.messages",
            r#""messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]"#,
        ),
        (
            "openai.responses",
            r#""input":[{"role":"user","content":"hello"}]"#,
        ),
        (
            "openai.chat",
            r#""messages":[{"role":"user","content":"hello"}]"#,
        ),
        (
            "gemini.generate",
            r#""contents":[{"role":"user","parts":[{"text":"hello"}]}]"#,
        ),
    ] {
        for extra in [
            r#""n":1.0"#,
            r#""n":-0"#,
            r#""escaped":"\u00e9""#,
            r#""dup":1,"dup":2"#,
            r#""20":"x","3":"y""#,
        ] {
            let body = format!(" {{ \"model\" : \"original\", {shape}, {extra} }} \n");
            for edit in [false, true] {
                let url = format!(
                    "http://{}/ir/?dialect={dialect}{}",
                    server.address,
                    if edit { "&model=1" } else { "" }
                );
                let response = client.post(url).body(body.clone()).send().await.unwrap();
                assert_eq!(response.status(), 200, "{dialect}");
                assert_eq!(
                    response.text().await.unwrap(),
                    if edit {
                        body.replace("\"original\"", "\"changed\"")
                    } else {
                        body.clone()
                    }
                );
            }
        }
    }
    server.shutdown().await.unwrap();
}
