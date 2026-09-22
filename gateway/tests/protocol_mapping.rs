mod support;
use gateway::http::Server;
#[tokio::test]
async fn twelve_directed_request_pairs_and_opaque_state_rejection() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"map",r#"export default {async fetch(r,env){try{const u=new URL(r.url);return new Response(env.IR.to(env.IR.from(r,u.searchParams.get('from')),u.searchParams.get('to')));}catch(e){return new Response(e.message,{status:422});}}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let cases = [
        (
            "anthropic.messages",
            serde_json::json!({"model":"m","system":"system","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"tools":[{"name":"run","input_schema":{"type":"object"}}],"thinking":{"type":"enabled","budget_tokens":8192}}),
        ),
        (
            "openai.responses",
            serde_json::json!({"model":"m","instructions":"system","input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"tools":[{"type":"function","name":"run","parameters":{"type":"object"}}],"reasoning":{"effort":"medium"}}),
        ),
        (
            "openai.chat",
            serde_json::json!({"model":"m","messages":[{"role":"system","content":"system"},{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"run","parameters":{"type":"object"}}}],"reasoning_effort":"medium"}),
        ),
        (
            "gemini.generate",
            serde_json::json!({"model":"m","systemInstruction":{"parts":[{"text":"system"}]},"contents":[{"role":"user","parts":[{"text":"hello"}]}],"tools":[{"functionDeclarations":[{"name":"run","parametersJsonSchema":{"type":"object"}}]}],"generationConfig":{"thinkingConfig":{"thinkingBudget":8192}}}),
        ),
    ];
    for (from, body) in &cases {
        for (to, _) in &cases {
            if from == to {
                continue;
            }
            let response = client
                .post(format!(
                    "http://{}/map/?from={from}&to={to}",
                    server.address
                ))
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{from} -> {to}: {text}");
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["model"], "m");
            assert!(text.contains("hello") && text.contains("system") && text.contains("run"));
        }
    }
    let response = client
        .post(format!(
            "http://{}/map/?from=openai.responses&to=anthropic.messages",
            server.address
        ))
        .body(r#"{"input":"hello","previous_response_id":"opaque"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn multimodal_parallel_tools_results_and_schema_all_pairs() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"rich",r#"export default {fetch(r,env){try{const q=new URL(r.url).searchParams;return new Response(env.IR.to(env.IR.from(r,q.get('from')),q.get('to')));}catch(e){return new Response(e.message,{status:422});}}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    let cases: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/upstream/protocol-requests.json")).unwrap();
    for (from, body) in &cases {
        for to in cases.keys() {
            if from == to {
                continue;
            }
            let response = client
                .post(format!(
                    "http://{}/rich/?from={from}&to={to}",
                    server.address
                ))
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{from}->{to}: {text}");
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            for word in [
                "system", "hello", "aGVsbG8=", "alpha", "beta", "Paris", "Rome", "one", "two",
                "required",
            ] {
                assert!(text.contains(word), "{from}->{to} lost {word}: {text}");
            }
            if to == "openai.chat" {
                let tools: Vec<_> = v["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["role"] == "tool")
                    .collect();
                assert_eq!(tools.len(), 2);
                assert_eq!(tools[0]["tool_call_id"], "a");
                assert_eq!(tools[1]["tool_call_id"], "b");
            }
        }
    }
    for body in [
        r#"{"input":"x","tools":[{"type":"function","name":"x","parameters":{"type":"string","pattern":".*"}}]}"#,
        r#"{"input":"x","reasoning":{"effort":"low","future_option":true}}"#,
        r#"{"input":[{"type":"reasoning","encrypted_content":"opaque"}]}"#,
    ] {
        let response = client
            .post(format!(
                "http://{}/rich/?from=openai.responses&to=anthropic.messages",
                server.address
            ))
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }
    server.shutdown().await.unwrap();
}
