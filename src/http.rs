use std::sync::{Arc, LazyLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper::{Method, Request, Uri};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

// Minimal HTTP/1.1 client: one connection per request, no pool.
//
// Every socket gets SO_LINGER=0. When a request times out (say the host lost
// its uplink), dropping the future closes the socket with RST and the kernel
// discards whatever was still queued. With a normal close the kernel keeps
// retransmitting that queue for minutes and the stale request reaches the
// server once the network is back — a Telegram alert would then arrive late,
// out of order, and on top of our own retry.
//
// Root certificates are compiled in (webpki-roots), so TLS does not depend on
// the host's CA bundle.

static TLS: LazyLock<TlsConnector> = LazyLock::new(|| {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
});

pub struct Response {
    pub status: u16,
    pub body: Bytes,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

pub async fn get(url: &str, timeout: Duration) -> Result<Response, String> {
    request(Method::GET, url, None, timeout).await
}

pub async fn post_json(url: &str, body: &serde_json::Value, timeout: Duration) -> Result<Response, String> {
    request(Method::POST, url, Some(body.to_string()), timeout).await
}

pub async fn request(method: Method, url: &str, body: Option<String>, timeout: Duration) -> Result<Response, String> {
    tokio::time::timeout(timeout, send(method, url, body)).await.map_err(|_| format!("timeout after {}s", timeout.as_secs_f64()))?
}

async fn send(method: Method, url: &str, body: Option<String>) -> Result<Response, String> {
    let uri: Uri = url.parse().map_err(|e| format!("bad url {url}: {e}"))?;
    let https = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        other => return Err(format!("unsupported scheme {other:?}")),
    };
    let host = uri.host().ok_or("url without host")?.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });

    let tcp = TcpStream::connect((host.as_str(), port)).await.map_err(|e| format!("connect {host}:{port}: {e}"))?;
    let _ = tcp.set_nodelay(true);
    let _ = socket2::SockRef::from(&tcp).set_linger(Some(Duration::ZERO));

    let host_header = match uri.port_u16() {
        Some(p) => format!("{host}:{p}"),
        None => host.clone(),
    };
    let mut req = Request::builder()
        .method(method)
        .uri(uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"))
        .header("host", host_header)
        .header("connection", "close");
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let req = req.body(Full::new(Bytes::from(body.unwrap_or_default()))).map_err(|e| e.to_string())?;

    if https {
        let name = ServerName::try_from(host.clone()).map_err(|e| format!("bad TLS name {host}: {e}"))?;
        let tls = TLS.connect(name, tcp).await.map_err(|e| format!("TLS {host}: {e}"))?;
        exchange(TokioIo::new(tls), req).await
    } else {
        exchange(TokioIo::new(tcp), req).await
    }
}

// Drives the connection inside this future (no spawned task), so a timeout
// that drops the future also drops the socket.
async fn exchange<T>(io: T, req: Request<Full<Bytes>>) -> Result<Response, String>
where
    T: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = http1::handshake(io).await.map_err(|e| e.to_string())?;
    let call = async {
        let resp = sender.send_request(req).await.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
        Ok(Response { status, body })
    };
    tokio::pin!(conn, call);
    tokio::select! {
        r = &mut call => r,
        // The server may close right after answering: the response is
        // already queued for `call`, so let it finish.
        r = &mut conn => match r {
            Ok(()) => call.await,
            Err(e) => Err(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn one_shot_server(reply: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/x?y=1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut buf = [0; 4096];
            // Read until the end of the body (the client sends content-length).
            while !String::from_utf8_lossy(&req).contains("\r\n\r\n") || !req.ends_with(b"}") {
                let n = s.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            s.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&req).into_owned()
        });
        (url, task)
    }

    #[tokio::test]
    async fn posts_json_and_reads_the_response() {
        let (url, server) = one_shot_server("HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\n{\"ok\":true}").await;
        let r = post_json(&url, &serde_json::json!({ "a": 1 }), Duration::from_secs(2)).await.unwrap();
        assert!(r.ok());
        assert_eq!(r.json().unwrap()["ok"], true);
        let req = server.await.unwrap();
        assert!(req.starts_with("POST /x?y=1 HTTP/1.1\r\n"), "{req}");
        assert!(req.contains("content-type: application/json"));
        assert!(req.ends_with("{\"a\":1}"));
    }

    #[tokio::test]
    async fn times_out_when_the_server_never_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let err = get(&url, Duration::from_millis(200)).await.err().unwrap();
        assert!(err.starts_with("timeout"), "{err}");
    }
}
