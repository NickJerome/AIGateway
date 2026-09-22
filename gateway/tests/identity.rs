mod support;
use gateway::{http::Server, identity::Auth, plugins::Manifest, storage::Database};
use reqwest::{Client, Response};
use serde_json::{Value, json};

const ADMIN: &str = "fixture-admin-token";
const PASSWORD: &str = "test-password-long-enough";

trait JsonRequest {
    fn json(self, value: &Value) -> Self;
}
impl JsonRequest for reqwest::RequestBuilder {
    fn json(self, value: &Value) -> Self {
        self.header("content-type", "application/json")
            .body(value.to_string())
    }
}

async fn value(response: Response, status: u16) -> Value {
    let actual = response.status().as_u16();
    let body = response.text().await.unwrap();
    assert_eq!(actual, status, "{body}");
    if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&body).unwrap()
    }
}
async fn user(client: &Client, base: &str, login: &str, administrator: bool) -> Value {
    value(
        client
            .post(format!("{base}/__gateway/users"))
            .bearer_auth(ADMIN)
            .json(&json!({"login":login,"password":PASSWORD,"administrator":administrator}))
            .send()
            .await
            .unwrap(),
        201,
    )
    .await
}
async fn login(client: &Client, base: &str, name: &str, password: &str) -> String {
    let response = client
        .post(format!("{base}/__gateway/auth/login"))
        .json(&json!({"login":name,"password":password}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    value(response, 200).await["token"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn own_key(client: &Client, base: &str, session: &str) -> Value {
    value(
        client
            .post(format!("{base}/__gateway/me/keys"))
            .bearer_auth(session)
            .json(&json!({"name":"IDE"}))
            .send()
            .await
            .unwrap(),
        201,
    )
    .await
}

#[tokio::test]
async fn sessions_keys_and_platform_permissions_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let mut process = support::Process::start(dir.path(), "");
    let c = Client::new();
    let base = &process.base;
    let alice = user(&c, base, "Alice", false).await;
    let bob = user(&c, base, "bob", false).await;
    assert_eq!(alice["login"], "alice");
    assert!(alice.get("password_hash").is_none());
    let session = login(&c, base, "ALICE", PASSWORD).await;
    let other_session = login(&c, base, "bob", PASSWORD).await;
    let key = own_key(&c, base, &session).await;
    assert_eq!(key["user_id"], alice["id"]);
    let token = key["key"].as_str().unwrap();
    value(
        c.get(format!("{base}/__gateway/me"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{base}/__gateway/users"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        403,
    )
    .await;
    value(
        c.get(format!("{base}/__gateway/plugins"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.get(format!("{base}/__gateway/me"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.post(format!("{base}/__gateway/me/keys"))
            .bearer_auth(&session)
            .json(&json!({"name":"bad","user_id":bob["id"]}))
            .send()
            .await
            .unwrap(),
        403,
    )
    .await;
    value(
        c.post(format!("{base}/__gateway/me/keys"))
            .bearer_auth(&session)
            .json(&json!({"name":"bad","administrator":true}))
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    let other_key = own_key(&c, base, &other_session).await;
    value(
        c.delete(format!(
            "{base}/__gateway/me/keys/{}",
            other_key["id"].as_str().unwrap()
        ))
        .bearer_auth(&session)
        .send()
        .await
        .unwrap(),
        404,
    )
    .await;
    let list = value(
        c.get(format!("{base}/__gateway/me/keys"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0].get("key").is_none());
    assert!(list[0].get("token_hash").is_none());
    value(
        c.post(format!("{base}/__gateway/auth/logout"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        204,
    )
    .await;
    value(
        c.get(format!("{base}/__gateway/me"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    let admin = user(&c, base, "owner", true).await;
    let admin_session = login(&c, base, "owner", PASSWORD).await;
    value(
        c.get(format!("{base}/__gateway/plugins"))
            .bearer_auth(&admin_session)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    let admin_key = own_key(&c, base, &admin_session).await;
    value(
        c.get(format!("{base}/__gateway/plugins"))
            .bearer_auth(admin_key["key"].as_str().unwrap())
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    // Demoting an administrator invalidates existing sessions immediately.
    value(
        c.put(format!(
            "{base}/__gateway/users/{}",
            admin["id"].as_str().unwrap()
        ))
        .bearer_auth(ADMIN)
        .json(&json!({"administrator":false}))
        .send()
        .await
        .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{base}/__gateway/plugins"))
            .bearer_auth(&admin_session)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    process.stop();
}

const WHO: &str = r#"export default {fetch(request,env){
  const identity=env.IDENTITY;
  let blocked=0;
  try{env.IDENTITY={userId:'spoof'}}catch{blocked++}
  try{Object.defineProperty(env,'IDENTITY',{value:{userId:'spoof'}})}catch{blocked++}
  if(identity){try{identity.userId='spoof'}catch{blocked++}}
  return Response.json({identity:env.IDENTITY,blocked,authorization:request.headers.get('authorization')});
}}"#;
const ROUTES: &str = r#"name='who'
main='index.js'
routes=[
 {pattern='/who/*',auth='key'},
 {pattern='/who/public',auth='public'},
 {pattern='/who/account',auth='user'},
 {pattern='/who/admin',auth='admin'}
]
"#;

#[tokio::test]
async fn route_auth_identity_revocation_expiry_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = support::Process::start(dir.path(), "");
    let c = Client::new();
    let archive = support::zip_files(&[
        ("wrangler.toml", ROUTES.as_bytes()),
        ("index.js", WHO.as_bytes()),
    ]);
    value(
        c.post(format!("{}/__gateway/plugins", p.base))
            .bearer_auth(ADMIN)
            .header("content-type", "application/zip")
            .body(archive)
            .send()
            .await
            .unwrap(),
        201,
    )
    .await;
    let u = user(&c, &p.base, "alice", false).await;
    let session = login(&c, &p.base, "alice", PASSWORD).await;
    let key = own_key(&c, &p.base, &session).await;
    let token = key["key"].as_str().unwrap();
    let service = value(
        c.post(format!("{}/__gateway/keys", p.base))
            .bearer_auth(ADMIN)
            .json(&json!({"name":"background"}))
            .send()
            .await
            .unwrap(),
        201,
    )
    .await;
    let service_token = service["key"].as_str().unwrap();
    for credential in [None, Some("bad"), Some(ADMIN), Some(session.as_str())] {
        let req = c.get(format!("{}/who/check", p.base));
        let req = if let Some(token) = credential {
            req.bearer_auth(token)
        } else {
            req
        };
        value(req.send().await.unwrap(), 401).await;
    }
    let identity = value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(token)
            .header("x-user-id", "spoof")
            .header("x-identity", "spoof")
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(identity["identity"]["userId"], u["id"]);
    assert_eq!(identity["identity"]["keyId"], key["id"]);
    assert_eq!(identity["identity"]["administrator"], false);
    assert_eq!(identity["blocked"], 3);
    assert!(identity["authorization"].is_null());
    let public = value(
        c.get(format!("{}/who/public", p.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert!(public["identity"].is_null());
    let account = value(
        c.get(format!("{}/who/account", p.base))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(account["identity"]["kind"], "session");
    value(
        c.get(format!("{}/who/admin", p.base))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        403,
    )
    .await;
    let platform = value(
        c.get(format!("{}/who/admin", p.base))
            .bearer_auth(ADMIN)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(platform["identity"]["kind"], "platform");
    assert!(platform["authorization"].is_null());
    // Duplicate Authorization headers must not select whichever credential happens to appear first.
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(token)
            .header("authorization", format!("Bearer {service_token}"))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    let responses = futures::future::join_all((0..16).map(|i| {
        let token = if i % 2 == 0 { token } else { service_token };
        let url = format!("{}/who/check", p.base);
        let c = c.clone();
        async move { value(c.get(url).bearer_auth(token).send().await.unwrap(), 200).await }
    }))
    .await;
    for (i, r) in responses.iter().enumerate() {
        assert_eq!(
            r["identity"]["keyId"],
            if i % 2 == 0 {
                key["id"].clone()
            } else {
                service["id"].clone()
            }
        );
    }
    p.stop();
    p = support::Process::start(dir.path(), "");
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{}/__gateway/me", p.base))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    // No raw credential or password is stored in identity tables.
    let db = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    let stored: String = db
        .query_row(
            "SELECT token_hash FROM identity_keys WHERE id=?1",
            [key["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(stored, token);
    assert_eq!(stored.len(), 64);
    let password: String = db
        .query_row(
            "SELECT password_hash FROM identity_users WHERE id=?1",
            [u["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(password, PASSWORD);
    value(
        c.put(format!(
            "{}/__gateway/users/{}",
            p.base,
            u["id"].as_str().unwrap()
        ))
        .bearer_auth(ADMIN)
        .json(&json!({"enabled":false}))
        .send()
        .await
        .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(service_token)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{}/__gateway/me", p.base))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.put(format!(
            "{}/__gateway/users/{}",
            p.base,
            u["id"].as_str().unwrap()
        ))
        .bearer_auth(ADMIN)
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    value(
        c.delete(format!(
            "{}/__gateway/keys/{}",
            p.base,
            key["id"].as_str().unwrap()
        ))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap(),
        204,
    )
    .await;
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    db.execute(
        "UPDATE identity_keys SET expires_at=1 WHERE id=?1",
        [service["id"].as_str().unwrap()],
    )
    .unwrap();
    value(
        c.get(format!("{}/who/check", p.base))
            .bearer_auth(service_token)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    p.stop();
}

#[tokio::test]
async fn password_reset_validation_and_last_administrator() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = support::Process::start(dir.path(), "");
    let c = Client::new();
    let u = user(&c, &p.base, "owner", true).await;
    let old_session = login(&c, &p.base, "owner", PASSWORD).await;
    value(
        c.post(format!("{}/__gateway/auth/login", p.base))
            .json(&json!({"login":"owner","password":"wrong"}))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.post(format!("{}/__gateway/auth/login", p.base))
            .json(&json!({"login":"missing","password":"wrong"}))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.post(format!("{}/__gateway/users", p.base))
            .bearer_auth(ADMIN)
            .json(&json!({"login":"OWNER","password":PASSWORD}))
            .send()
            .await
            .unwrap(),
        409,
    )
    .await;
    value(
        c.post(format!("{}/__gateway/users", p.base))
            .bearer_auth(ADMIN)
            .json(&json!({"login":"short","password":"short"}))
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    value(
        c.post(format!("{}/__gateway/keys", p.base))
            .bearer_auth(ADMIN)
            .json(&json!({"name":"expired","expires_at":1}))
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    value(
        c.post(format!("{}/__gateway/keys", p.base))
            .bearer_auth(ADMIN)
            .json(&json!({"name":"missing owner","user_id":"missing"}))
            .send()
            .await
            .unwrap(),
        404,
    )
    .await;
    value(
        c.put(format!(
            "{}/__gateway/users/{}",
            p.base,
            u["id"].as_str().unwrap()
        ))
        .bearer_auth(ADMIN)
        .json(&json!({"password":"new-test-password-long"}))
        .send()
        .await
        .unwrap(),
        200,
    )
    .await;
    value(
        c.get(format!("{}/__gateway/me", p.base))
            .bearer_auth(&old_session)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    value(
        c.post(format!("{}/__gateway/auth/login", p.base))
            .json(&json!({"login":"owner","password":PASSWORD}))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    let session = login(&c, &p.base, "owner", "new-test-password-long").await;
    p.stop();
    // Once an administrator exists, the bootstrap environment secret is optional.
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let base = format!("http://{}", server.address);
    value(
        c.get(format!("{base}/__gateway/plugins"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await;
    value(
        c.put(format!(
            "{base}/__gateway/users/{}",
            u["id"].as_str().unwrap()
        ))
        .bearer_auth(&session)
        .json(&json!({"administrator":false}))
        .send()
        .await
        .unwrap(),
        409,
    )
    .await;
    value(
        c.put(format!(
            "{base}/__gateway/users/{}",
            u["id"].as_str().unwrap()
        ))
        .bearer_auth(&session)
        .json(&json!({"enabled":false}))
        .send()
        .await
        .unwrap(),
        409,
    )
    .await;
    let db = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    db.execute("UPDATE identity_sessions SET expires_at=1", [])
        .unwrap();
    value(
        c.get(format!("{base}/__gateway/me"))
            .bearer_auth(&session)
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn reserved_identity_cannot_be_supplied_by_plugin_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager =
        gateway::plugins::PluginManager::open(dir.path().join("plugins"), db, Default::default())
            .await
            .unwrap();
    for extra in [
        "[vars]\nIDENTITY={userId='spoof'}",
        "[[kv_namespaces]]\nbinding='IDENTITY'\nid='spoof'",
        "[[unsafe.bindings]]\nname='IDENTITY'\ntype='usage'",
        "[[durable_objects.bindings]]\nname='IDENTITY'\nclass_name='Thing'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['Thing']",
    ] {
        assert!(
            manager
                .install(support::package("spoof", WHO, extra))
                .await
                .is_err(),
            "{extra}"
        );
    }
    manager
        .install(support::package("ok", WHO, ""))
        .await
        .unwrap();
    let mut config = gateway::plugins::Overrides::default();
    config
        .vars
        .insert("IDENTITY".into(), json!({"userId":"spoof"}));
    assert!(manager.configure("ok", config).await.is_err());
    assert!(
        Manifest::parse("name='bad'\nmain='x.js'\nroutes=[{pattern='/bad',auth='optional'}]")
            .is_err()
    );
    let legacy = Manifest::parse("name='old'\nmain='x.js'\nroutes=['/old']").unwrap();
    assert_eq!(legacy.routes[0].auth(), Auth::Public);
}

#[test]
fn schema_three_migrates_without_touching_plugin_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gateway.sqlite");
    {
        let db = Database::open(dir.path(), 1, 8).unwrap();
        futures::executor::block_on(async {
            let kv = db.namespace("keep").await.unwrap();
            kv.put("key".into(), b"value".to_vec(), None).await.unwrap();
        });
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("DROP TABLE identity_sessions; DROP TABLE identity_keys; DROP TABLE identity_users; PRAGMA user_version=3;").unwrap();
    }
    let db = Database::open(dir.path(), 1, 8).unwrap();
    futures::executor::block_on(async {
        assert_eq!(
            db.namespace("keep")
                .await
                .unwrap()
                .get("key".into())
                .await
                .unwrap(),
            Some(b"value".to_vec())
        );
    });
    let c = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}
