mod support;
use gateway::http::Server;
#[tokio::test]
async fn json_and_fragmented_sse_all_twelve_pairs() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"response-map",r#"export default {async fetch(r,env){try{const u=new URL(r.url);let body=await r.text();const sse=u.searchParams.has('sse');if(sse){const bytes=new TextEncoder().encode(body);let i=0;const step=Number(u.searchParams.get('step')??1);body=new ReadableStream({pull(c){if(i>=bytes.length){c.close();return;}c.enqueue(bytes.slice(i,i+step));i+=step;}});}const response=new Response(body,{headers:{'content-type':sse?'text/event-stream':'application/json'}});return new Response(env.IR.to(env.IR.from(response,u.searchParams.get('from')),u.searchParams.get('to')));}catch(e){return new Response(e.message,{status:422});}}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let cases = [
        (
            "anthropic.messages",
            r#"{"id":"resp","model":"m","content":[{"type":"text","text":"héllo"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}"#,
            "event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"resp\",\"model\":\"m\"}}\r\n\r\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"héllo\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ),
        (
            "openai.responses",
            r#"{"id":"resp","model":"m","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"héllo"}]}],"usage":{"input_tokens":3,"output_tokens":2}}"#,
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp\",\"model\":\"m\"}}\n\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"héllo\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
        ),
        (
            "openai.chat",
            r#"{"id":"resp","model":"m","choices":[{"message":{"role":"assistant","content":"héllo"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
            "data: {\"id\":\"resp\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"héllo\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n",
        ),
        (
            "gemini.generate",
            r#"{"responseId":"resp","modelVersion":"m","candidates":[{"content":{"role":"model","parts":[{"text":"héllo"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2}}"#,
            "data: {\"responseId\":\"resp\",\"modelVersion\":\"m\",\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"héllo\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2}}\n\n",
        ),
    ];
    for (from, json, sse) in &cases {
        for (to, _, _) in &cases {
            if from == to {
                continue;
            }
            let tools = match *from {
                "anthropic.messages" => {
                    r#"{"id":"r","content":[{"type":"tool_use","id":"a","name":"alpha","input":{"city":"Paris"}},{"type":"tool_use","id":"b","name":"beta","input":{"city":"Rome"}}],"stop_reason":"tool_use"}"#
                }
                "openai.responses" => {
                    r#"{"id":"r","status":"completed","output":[{"type":"function_call","call_id":"a","name":"alpha","arguments":"{\"city\":\"Paris\"}"},{"type":"function_call","call_id":"b","name":"beta","arguments":"{\"city\":\"Rome\"}"}]}"#
                }
                "openai.chat" => {
                    r#"{"id":"r","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"a","function":{"name":"alpha","arguments":"{\"city\":\"Paris\"}"}},{"id":"b","function":{"name":"beta","arguments":"{\"city\":\"Rome\"}"}}]},"finish_reason":"tool_calls"}]}"#
                }
                _ => {
                    r#"{"responseId":"r","candidates":[{"content":{"role":"model","parts":[{"functionCall":{"id":"a","name":"alpha","args":{"city":"Paris"}}},{"functionCall":{"id":"b","name":"beta","args":{"city":"Rome"}}}]},"finishReason":"STOP"}]}"#
                }
            };
            let output = client
                .post(format!(
                    "http://{}/response-map/?from={from}&to={to}",
                    server.address
                ))
                .body(tools)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            for word in ["alpha", "beta", "Paris", "Rome"] {
                assert!(output.contains(word), "{from}->{to}: {output}");
            }
            assert!(
                !output.contains("usage"),
                "missing usage must stay missing: {output}"
            );
            for (mode, body) in [
                ("", *json),
                ("&sse=1&step=1", *sse),
                ("&sse=1&step=17", *sse),
                ("&sse=1&step=65536", *sse),
            ] {
                let response = client
                    .post(format!(
                        "http://{}/response-map/?from={from}&to={to}{mode}",
                        server.address
                    ))
                    .body(body.to_owned())
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                let output = response.text().await.unwrap();
                assert_eq!(status, 200, "{from}->{to}: {output}");
                assert!(output.contains("héllo"), "{from}->{to}{mode}: {output}");
                assert!(!output.contains("upstream_error"), "{output}");
                if mode.is_empty() {
                    let _: serde_json::Value = serde_json::from_str(&output).unwrap();
                }
            }
        }
    }
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn parallel_tool_deltas_all_pairs_and_incomplete_stream_errors() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"tools-map",r#"export default {async fetch(r,env){const q=new URL(r.url).searchParams;const bytes=new Uint8Array(await r.arrayBuffer());let i=0;const stream=new ReadableStream({pull(c){if(i===bytes.length){c.close();return;}const size=1+(i%17);c.enqueue(bytes.slice(i,i+size));i=Math.min(bytes.length,i+size);}});return new Response(env.IR.to(env.IR.from(new Response(stream,{headers:{'content-type':'text/event-stream'}}),q.get('from')),q.get('to')));}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let cases: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/upstream/protocol-tools-sse.json")).unwrap();
    for (from, events) in &cases {
        for to in cases.keys() {
            if from == to {
                continue;
            }
            let body = events
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    format!(
                        "data: {}\n\n",
                        e.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| e.to_string())
                    )
                })
                .collect::<String>();
            let text = client
                .post(format!(
                    "http://{}/tools-map/?from={from}&to={to}",
                    server.address
                ))
                .body(body)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert!(!text.contains("upstream_error"), "{from}->{to}: {text}");
            for word in ["alpha", "beta", "Paris"] {
                assert!(text.contains(word), "{from}->{to} lost {word}: {text}");
            }
        }
    }
    let body = "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"first\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":1,\"delta\":\"second\"}\n\n";
    let text = client
        .post(format!(
            "http://{}/tools-map/?from=openai.responses&to=anthropic.messages",
            server.address
        ))
        .body(body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        text.matches("event: content_block_start").count(),
        2,
        "{text}"
    );
    assert!(
        text.contains("upstream_error") && text.contains("before terminal"),
        "{text}"
    );
    server.shutdown().await.unwrap();
}
