//! Local-only HTTP/TLS/CONNECT/SOCKS fixtures; no external service involved.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub struct Fixture {
    pub address: std::net::SocketAddr,
    pub certificate: Option<Vec<u8>>,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub connections: Arc<AtomicUsize>,
    pub socks_types: Arc<Mutex<Vec<u8>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone, Copy)]
pub enum Mode {
    Http,
    Proxy,
    Socks,
}
impl Fixture {
    pub async fn start(mode: Mode, tls: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let socks_types = Arc::new(Mutex::new(Vec::new()));
        let mut certificate = None;
        let acceptor = if tls {
            let generated =
                rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
                    .unwrap();
            let cert = generated.cert.der().clone();
            certificate = Some(cert.to_vec());
            let key = tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(
                generated.signing_key.serialize_der(),
            );
            let mut config = tokio_rustls::rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key.into())
                .unwrap();
            config.alpn_protocols = match mode {
                Mode::Http => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
                _ => vec![b"http/1.1".to_vec()],
            };
            Some(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
        } else {
            None
        };
        let reqs = requests.clone();
        let count = connections.clone();
        let types = socks_types.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let reqs = reqs.clone();
                let types = types.clone();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Some(acceptor) = acceptor {
                        if let Ok(socket) = acceptor.accept(socket).await {
                            match mode {
                                Mode::Http => http(socket, reqs).await,
                                Mode::Proxy => {
                                    let _ = proxy(socket, reqs).await;
                                }
                                Mode::Socks => {}
                            }
                        }
                    } else {
                        match mode {
                            Mode::Http => http(socket, reqs).await,
                            Mode::Proxy => {
                                let _ = proxy(socket, reqs).await;
                            }
                            Mode::Socks => {
                                let _ = socks(socket, types).await;
                            }
                        }
                    }
                });
            }
        });
        Self {
            address,
            certificate,
            requests,
            connections,
            socks_types,
            task,
        }
    }
}
async fn http<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    socket: T,
    requests: Arc<Mutex<Vec<String>>>,
) {
    use http_body_util::{BodyExt, Full};
    use hyper::{Request, Response, body::Incoming, service::service_fn};
    let service = service_fn(move |request: Request<Incoming>| {
        let requests = requests.clone();
        async move {
            let path = request.uri().path().to_owned();
            let method = request.method().to_string();
            let version = format!("{:?}", request.version());
            let auth = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            requests
                .lock()
                .unwrap()
                .push(format!("{method} {path} {version} auth={auth}"));
            let location = request
                .headers()
                .get("x-location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("/echo")
                .to_owned();
            let input = request.into_body().collect().await.unwrap().to_bytes();
            let mut response = Response::builder();
            let content = match path.as_str() {
                "/redirect" => {
                    response = response.status(307).header("location", location);
                    bytes::Bytes::new()
                }
                "/loop" => {
                    response = response.status(302).header("location", "/loop");
                    bytes::Bytes::new()
                }
                "/headers" => {
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                    bytes::Bytes::from_static(b"late")
                }
                "/echo" => input,
                _ => bytes::Bytes::from_static(b"fixture"),
            };
            Ok::<_, std::convert::Infallible>(response.body(Full::new(content)).unwrap())
        }
    });
    let builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    let _ = builder
        .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
        .await;
}
async fn proxy<T: AsyncRead + AsyncWrite + Unpin>(
    mut socket: T,
    requests: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    loop {
        let mut header = Vec::new();
        loop {
            let b = socket.read_u8().await?;
            header.push(b);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
            if header.len() > 65536 {
                return Ok(());
            }
        }
        let text = String::from_utf8_lossy(&header).into_owned();
        requests.lock().unwrap().push(text.clone());
        if !text
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic dxnlcjpwyxnz")
        {
            socket
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                )
                .await?;
            continue;
        }
        let target = text.split_whitespace().nth(1).unwrap();
        if text.starts_with("CONNECT ") {
            let mut target = tokio::net::TcpStream::connect(target).await?;
            socket
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            tokio::io::copy_bidirectional(&mut socket, &mut target).await?;
            return Ok(());
        } else {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nproxied")
                .await?;
        }
    }
}
async fn socks(
    mut socket: tokio::net::TcpStream,
    types: Arc<Mutex<Vec<u8>>>,
) -> std::io::Result<()> {
    let version = socket.read_u8().await?;
    if version != 5 {
        return Ok(());
    }
    let n = socket.read_u8().await?;
    let mut methods = vec![0; n as usize];
    socket.read_exact(&mut methods).await?;
    socket.write_all(&[5, 2]).await?;
    if socket.read_u8().await? != 1 {
        return Ok(());
    }
    let n = socket.read_u8().await?;
    let mut user = vec![0; n as usize];
    socket.read_exact(&mut user).await?;
    let n = socket.read_u8().await?;
    let mut pass = vec![0; n as usize];
    socket.read_exact(&mut pass).await?;
    if user != b"user" || pass != b"pass" {
        socket.write_all(&[1, 1]).await?;
        return Ok(());
    }
    socket.write_all(&[1, 0]).await?;
    let mut header = [0; 4];
    socket.read_exact(&mut header).await?;
    types.lock().unwrap().push(header[3]);
    let size = match header[3] {
        1 => 4,
        4 => 16,
        3 => socket.read_u8().await? as usize,
        _ => return Ok(()),
    };
    let mut address = vec![0; size];
    socket.read_exact(&mut address).await?;
    let port = socket.read_u16().await?;
    let mut target = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut target).await?;
    Ok(())
}
