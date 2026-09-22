mod support;
use gateway::http::Server;
#[tokio::test]
async fn raw_proxy_semantics_reasoning_last_write_and_output_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"views",r#"export default {async fetch(r,env){try{
 const ir=env.IR.from(r,'openai.responses'),raw=ir.raw,check=(v,m)=>{if(!v)throw new Error(m);};
 check(raw.model==='old','read');check('model'in raw,'in');check(!('missing'in raw),'missing');check(Object.keys(raw).includes('model'),'keys');check({...raw}.model==='old','spread');const {model}=raw;check(model==='old','destructure');check(Array.isArray(raw.input),'array');check([...raw.input].length===2,'iterate');check(raw.input.map(x=>x.role).join(',')==='user,assistant','map');check(JSON.parse(JSON.stringify(raw)).input.length===2,'stringify');
 raw.input.push({role:'user',content:'third'});check(raw.input.length===3,'push');raw.input.splice(1,1);check(raw.input.length===2&&raw.input[1].content==='third','splice');raw.input.unshift({role:'user',content:'first'});check(raw.input.shift().content==='first','shift');check(raw.input.pop().content==='third','pop');raw.input[0].content='changed';check(ir.messages[0].content[0].text==='changed','unified message');ir.messages[0].content[0].text='back';check(raw.input[0].content[0].text==='back','message write');
 ir.reasoning.level='high';check(raw.reasoning.effort==='high','reasoning write');raw.reasoning.effort='low';check(ir.reasoning.level==='low'&&ir.reasoning.budgetTokens===1024,'last raw write');
 raw.extra={a:[1,2]};raw.extra.a[0]=4;delete raw.extra.a[1];check(JSON.stringify(raw.extra)==='{"a":[4,null]}','nested edits');delete raw.extra;check(!('extra'in raw),'delete');raw.nonfinite=NaN;check(raw.nonfinite===null,'nan');raw.absent=undefined;check(!('absent'in raw),'undefined');
 let frozen=false,cloned=false,big=false;try{Object.freeze(raw)}catch(e){frozen=e instanceof TypeError;}try{structuredClone(raw)}catch(e){cloned=e.name==='DataCloneError';}try{raw.big=1n}catch(e){big=e instanceof TypeError;}check(frozen&&cloned&&big,'explicit unsupported operations');
 raw.obj={b:2,'10':'ten','2':'two',a:1};check(Object.keys(raw.obj).join(',')==='2,10,b,a','integer key order');check(Object.hasOwn(raw.obj,'a'),'hasOwn');check(Object.values(raw.obj).includes('two'),'values');check(Object.entries(raw.obj).some(([k,v])=>k==='a'&&v===1),'entries');
 Object.assign(raw.obj,{a:5,c:6});check(raw.obj.a===5&&raw.obj.c===6,'assign');check(Object.getOwnPropertyDescriptor(raw.obj,'a').writable,'descriptor');check(Object.getPrototypeOf(raw.obj)===Object.prototype,'object prototype');check(Object.getPrototypeOf(raw.input)===Array.prototype,'array prototype');check(raw.input['01']===undefined,'noncanonical index');
 raw.numbers=[3,1,2];check(raw.numbers.sort().join(',')==='1,2,3','sort');check(raw.numbers.reverse().join(',')==='3,2,1','reverse');raw.numbers.length=2;check(raw.numbers.length===2&&raw.numbers[2]===undefined,'truncate');raw.numbers.fill(7);check(raw.numbers.join(',')==='7,7','fill');raw.obj.a=null;check(raw.obj.a===null,'null');delete raw.obj;delete raw.numbers;
 const first=env.IR.to(ir);ir.model='new';const second=env.IR.to(ir);const a=await new Response(first).json(),b=await new Response(second).json();check(a.model==='old'&&b.model==='new','snapshot');return new Response('ok');
 }catch(e){return new Response(e.message,{status:500});}}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let response=reqwest::Client::new().post(format!("http://{}/views/",server.address)).body(r#"{"model":"old","input":[{"role":"user","content":"one"},{"role":"assistant","content":"two"}],"reasoning":{"effort":"medium"}}"#).send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, "ok");
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn unified_edits_preserve_opaque_blocks_and_response_item_shapes() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"signed",r#"export default {fetch(r,env){const d=new URL(r.url).searchParams.get('d');const ir=env.IR.from(r,d);ir.messages[0].content[0].text='changed';return new Response(env.IR.to(ir));}}"#,"[[unsafe.bindings]]\nname='IR'\ntype='ir'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let client = reqwest::Client::new();
    for (d, body) in [
        (
            "anthropic.messages",
            r#"{"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"old","signature":"signed","cache_control":{"type":"ephemeral"},"provider_extension":"retained"}]}]}"#,
        ),
        (
            "openai.responses",
            r#"{"input":[{"type":"reasoning","summary":[{"type":"summary_text","text":"old"}],"encrypted_content":"signed","provider_extension":"retained"}]}"#,
        ),
        (
            "gemini.generate",
            r#"{"contents":[{"role":"model","parts":[{"text":"old","thought":true,"thoughtSignature":"signed","provider_extension":"retained"}]}]}"#,
        ),
    ] {
        let response = client
            .post(format!("http://{}/signed/?d={d}", server.address))
            .body(body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, 200, "{text}");
        for word in ["changed", "signed", "retained"] {
            assert!(text.contains(word), "{d}: {text}");
        }
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        if d == "openai.responses" {
            assert_eq!(value["input"][0]["summary"][0]["text"], "changed");
            assert!(value["input"][0].get("content").is_none());
        }
        if d == "anthropic.messages" {
            assert_eq!(
                value["messages"][0]["content"][0]["cache_control"]["type"],
                "ephemeral"
            );
        }
    }
    server.shutdown().await.unwrap();
}
