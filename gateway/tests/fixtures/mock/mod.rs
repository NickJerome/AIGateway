#![allow(dead_code)]
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{convert::Infallible, future::Future, sync::Arc};
pub struct Mock {
    pub base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Mock {
    pub async fn start<F, Fut>(handler: F) -> Self
    where
        F: Fn(String, hyper::HeaderMap, Bytes) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (u16, Vec<(String, String)>, String)> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {accepted=listener.accept()=>{let Ok((socket,_))=accepted else{break;};let handler=handler.clone();children.spawn(async move{let service=service_fn(move|request:Request<Incoming>|{let handler=handler.clone();async move{let (parts,body)=request.into_parts();let body=body.collect().await.unwrap().to_bytes();let (status,headers,body)=handler(parts.uri.to_string(),parts.headers,body).await;let mut response=Response::builder().status(status);for(k,v)in headers{response=response.header(k,v);}Ok::<_,Infallible>(response.body(Full::new(Bytes::from(body))).unwrap())}});let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(socket),service).await;});},_=children.join_next(),if !children.is_empty()=>{}}
            }
        });
        Self { base, task }
    }
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
