use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::{Body, Frame, Incoming};
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use indexmap::IndexMap;
use tokio::net::TcpListener;

use crate::format::num;

// Minimal built-in OTLP/gRPC metrics receiver, so monadar works without an
// OpenTelemetry Collector. monad-bft and monad-rpc push metrics to
// --otel-endpoint (http://127.0.0.1:4317 by default); we accept
// MetricsService/Export, decode the protobuf by hand (only the fields we need)
// and keep the latest value of every gauge/sum series. text() renders them in
// Prometheus exposition format so metric_snapshot.rs works unchanged.

const EXPORT_PATH: &str = "/opentelemetry.proto.collector.metrics.v1.MetricsService/Export";
const SERIES_TTL_MS: i64 = 5 * 60_000;

pub type Labels = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq)]
pub struct OtlpPoint {
    pub name: String,
    pub labels: Labels,
    pub value: f64,
    pub delta: bool,
}

// --- protobuf wire format ---------------------------------------------------

struct Field<'a> {
    no: u64,
    wire: u64,
    varint: u64,
    bytes: &'a [u8],
}

impl Field<'_> {
    fn f64(&self) -> f64 {
        f64::from_le_bytes(self.bytes.try_into().unwrap())
    }

    fn i64(&self) -> i64 {
        i64::from_le_bytes(self.bytes.try_into().unwrap())
    }

    fn str(&self) -> String {
        String::from_utf8_lossy(self.bytes).into_owned()
    }
}

fn fields(buf: &[u8]) -> Result<Vec<Field<'_>>, String> {
    let mut pos = 0;
    let varint = |pos: &mut usize| -> Result<u64, String> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            let b = *buf.get(*pos).ok_or("protobuf: truncated varint")?;
            *pos += 1;
            if shift >= 64 {
                return Err("protobuf: varint too long".into());
            }
            result |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    };
    let mut out = Vec::new();
    while pos < buf.len() {
        let key = varint(&mut pos)?;
        let mut f = Field { no: key >> 3, wire: key & 7, varint: 0, bytes: &[] };
        let len = match f.wire {
            0 => {
                f.varint = varint(&mut pos)?;
                0
            }
            1 => 8,
            2 => varint(&mut pos)? as usize,
            5 => 4,
            w => return Err(format!("protobuf: unsupported wire type {w}")),
        };
        if f.wire != 0 {
            let end = pos.checked_add(len).filter(|e| *e <= buf.len()).ok_or("protobuf: truncated field")?;
            f.bytes = &buf[pos..end];
            pos = end;
        }
        out.push(f);
    }
    Ok(out)
}

// --- OTLP messages (opentelemetry/proto/metrics/v1/metrics.proto) ------------

// AnyValue: string=1, bool=2, int=3, double=4. Arrays/kvlists/bytes are skipped.
fn any_value(buf: &[u8]) -> Result<Option<String>, String> {
    for f in fields(buf)? {
        match (f.no, f.wire) {
            (1, 2) => return Ok(Some(f.str())),
            (2, 0) => return Ok(Some((f.varint != 0).to_string())),
            (3, 0) => return Ok(Some((f.varint as i64).to_string())),
            (4, 1) => return Ok(Some(num(f.f64()))),
            _ => {}
        }
    }
    Ok(None)
}

// KeyValue: key=1, value=2
fn key_value(buf: &[u8], into: &mut Labels) -> Result<(), String> {
    let mut key = String::new();
    let mut value = None;
    for f in fields(buf)? {
        match (f.no, f.wire) {
            (1, 2) => key = f.str(),
            (2, 2) => value = any_value(f.bytes)?,
            _ => {}
        }
    }
    if let Some(v) = value.filter(|_| !key.is_empty()) {
        into.insert(sanitize(&key), v);
    }
    Ok(())
}

// NumberDataPoint: attributes=7, as_double=4 (double), as_int=6 (sfixed64)
fn data_point(buf: &[u8], labels: &Labels) -> Result<Option<(Labels, f64)>, String> {
    let mut own = labels.clone();
    let mut value = None;
    for f in fields(buf)? {
        match (f.no, f.wire) {
            (7, 2) => key_value(f.bytes, &mut own)?,
            (4, 1) => value = Some(f.f64()),
            (6, 1) => value = Some(f.i64() as f64),
            _ => {}
        }
    }
    Ok(value.map(|v| (own, v)))
}

// Metric: name=1, gauge=5, sum=7 (histograms etc. are ignored).
// Gauge/Sum: data_points=1; Sum: aggregation_temporality=2 (1 = DELTA), is_monotonic=3.
fn metric(buf: &[u8], labels: &Labels, out: &mut Vec<OtlpPoint>) -> Result<(), String> {
    let mut name = String::new();
    let mut data = None;
    let mut is_sum = false;
    for f in fields(buf)? {
        match (f.no, f.wire) {
            (1, 2) => name = sanitize(&f.str()),
            (5 | 7, 2) => {
                data = Some(f.bytes);
                is_sum = f.no == 7;
            }
            _ => {}
        }
    }
    let Some(data) = data.filter(|_| !name.is_empty()) else { return Ok(()) };

    let mut points = Vec::new();
    let mut delta_temporality = false;
    let mut monotonic = false;
    for f in fields(data)? {
        match (f.no, f.wire) {
            (1, 2) => points.push(f.bytes),
            (2, 0) if is_sum => delta_temporality = f.varint == 1,
            (3, 0) if is_sum => monotonic = f.varint != 0,
            _ => {}
        }
    }
    for p in points {
        if let Some((labels, value)) = data_point(p, labels)? {
            out.push(OtlpPoint { name: name.clone(), labels, value, delta: delta_temporality && monotonic });
        }
    }
    Ok(())
}

// ExportMetricsServiceRequest: resource_metrics=1
// ResourceMetrics: resource=1 (Resource: attributes=1), scope_metrics=2
// ScopeMetrics: metrics=2
pub fn decode_export_request(buf: &[u8]) -> Result<Vec<OtlpPoint>, String> {
    let mut out = Vec::new();
    for rm in fields(buf)? {
        if rm.no != 1 || rm.wire != 2 {
            continue;
        }
        let mut labels = Labels::new();
        let mut scopes = Vec::new();
        for f in fields(rm.bytes)? {
            match (f.no, f.wire) {
                (1, 2) => {
                    for a in fields(f.bytes)? {
                        if a.no == 1 && a.wire == 2 {
                            key_value(a.bytes, &mut labels)?;
                        }
                    }
                }
                (2, 2) => scopes.push(f.bytes),
                _ => {}
            }
        }
        // Match the collector's Prometheus exporter: service.name is also exposed as job.
        if let Some(service) = labels.get("service_name").cloned() {
            labels.entry("job".into()).or_insert(service);
        }
        for s in scopes {
            for f in fields(s)? {
                if f.no == 2 && f.wire == 2 {
                    metric(f.bytes, &labels, &mut out)?;
                }
            }
        }
    }
    Ok(out)
}

// Same normalization as the collector's Prometheus exporter: `monad.peer_disc.num_peers` → `monad_peer_disc_num_peers`.
fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

// --- series store -----------------------------------------------------------

fn escape_label(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

struct Series {
    name: String,
    labels: Labels,
    value: f64,
    updated: i64,
}

pub struct SeriesStore {
    extra_labels: Labels,
    series: IndexMap<String, Series>,
    last_export: Option<i64>,
}

impl SeriesStore {
    pub fn new(extra_labels: Labels) -> SeriesStore {
        SeriesStore { extra_labels, series: IndexMap::new(), last_export: None }
    }

    pub fn ingest(&mut self, points: Vec<OtlpPoint>, now_ms: i64) {
        for p in points {
            let mut labels = self.extra_labels.clone();
            labels.extend(p.labels);
            let key = format!("{}{:?}", p.name, labels);
            let value = match self.series.get(&key) {
                Some(prev) if p.delta => prev.value + p.value,
                _ => p.value,
            };
            self.series.insert(key, Series { name: p.name, labels, value, updated: now_ms });
        }
        self.last_export = Some(now_ms);
    }

    pub fn text(&mut self, now_ms: i64) -> String {
        self.series.retain(|_, s| now_ms - s.updated <= SERIES_TTL_MS);
        let mut out = String::new();
        for s in self.series.values() {
            let labels: Vec<String> = s.labels.iter().map(|(k, v)| format!("{k}=\"{}\"", escape_label(v))).collect();
            out += &format!("{}{{{}}} {}\n", s.name, labels.join(","), num(s.value));
        }
        out
    }

    pub fn last_export(&self) -> Option<i64> {
        self.last_export
    }
}

// --- gRPC server --------------------------------------------------------------

// gRPC message framing: 1 byte compressed flag + 4 bytes big-endian length.
pub fn unframe(body: &[u8], encoding: Option<&str>) -> Result<Vec<Vec<u8>>, String> {
    let mut msgs = Vec::new();
    let mut pos = 0;
    while pos + 5 <= body.len() {
        let compressed = body[pos] == 1;
        let len = u32::from_be_bytes(body[pos + 1..pos + 5].try_into().unwrap()) as usize;
        let end = (pos + 5).saturating_add(len).min(body.len());
        let msg = &body[pos + 5..end];
        pos = end;
        if compressed {
            if encoding != Some("gzip") {
                return Err(format!("unsupported grpc-encoding {}", encoding.unwrap_or("undefined")));
            }
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(msg).read_to_end(&mut out).map_err(|e| e.to_string())?;
            msgs.push(out);
        } else {
            msgs.push(msg.to_vec());
        }
    }
    Ok(msgs)
}

// A gRPC response body: optional message bytes, then the grpc-status trailers.
pub struct GrpcBody {
    data: Option<Bytes>,
    trailers: Option<HeaderMap>,
}

impl Body for GrpcBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if let Some(d) = self.data.take() {
            return Poll::Ready(Some(Ok(Frame::data(d))));
        }
        Poll::Ready(self.trailers.take().map(|t| Ok(Frame::trailers(t))))
    }
}

// encodeURIComponent, for grpc-message.
fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn grpc_reply(status: u32, message: Option<&str>) -> Response<GrpcBody> {
    let mut trailers = HeaderMap::new();
    trailers.insert("grpc-status", HeaderValue::from(status));
    if let Some(m) = message.and_then(|m| HeaderValue::from_str(&percent_encode(m)).ok()) {
        trailers.insert("grpc-message", m);
    }
    // Empty ExportMetricsServiceResponse on success; no body on error.
    let data = (status == 0).then(|| Bytes::from_static(&[0; 5]));
    Response::builder().status(200).header("content-type", "application/grpc").body(GrpcBody { data, trailers: Some(trailers) }).unwrap()
}

async fn handle(req: Request<Incoming>, store: Arc<Mutex<SeriesStore>>) -> Result<Response<GrpcBody>, Infallible> {
    let ok_path = req.method() == hyper::Method::POST && req.uri().path() == EXPORT_PATH;
    let encoding = req.headers().get("grpc-encoding").and_then(|v| v.to_str().ok()).map(String::from);
    let body = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => return Ok(grpc_reply(3, Some(&e.to_string()))),
    };
    // 12 = UNIMPLEMENTED (e.g. trace/log exporters), same as a metrics-only collector.
    if !ok_path {
        return Ok(grpc_reply(12, Some("only metrics are supported")));
    }
    let decoded =
        unframe(&body, encoding.as_deref()).and_then(|msgs| msgs.iter().map(|m| decode_export_request(m)).collect::<Result<Vec<_>, _>>());
    match decoded {
        Ok(batches) => {
            let now = chrono::Utc::now().timestamp_millis();
            let mut s = store.lock().unwrap();
            for points in batches {
                s.ingest(points, now);
            }
            Ok(grpc_reply(0, None))
        }
        Err(e) => Ok(grpc_reply(3, Some(&e))), // 3 = INVALID_ARGUMENT
    }
}

pub struct OtlpReceiver {
    store: Arc<Mutex<SeriesStore>>,
    server: tokio::task::JoinHandle<()>,
}

impl OtlpReceiver {
    pub fn text(&self) -> String {
        self.store.lock().unwrap().text(chrono::Utc::now().timestamp_millis())
    }

    pub fn last_export(&self) -> Option<i64> {
        self.store.lock().unwrap().last_export()
    }
}

impl Drop for OtlpReceiver {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn split_listen(listen: &str) -> (String, u16) {
    let (host, port) = listen.rsplit_once(':').unwrap_or(("", listen));
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (if host.is_empty() { "127.0.0.1".into() } else { host.into() }, port.parse().unwrap_or(0))
}

// None if the address is taken (e.g. an otelcol is still running), so the
// caller can fall back to scraping its Prometheus endpoint.
pub async fn start_otlp_receiver(listen: &str, extra_labels: Labels) -> Option<OtlpReceiver> {
    let (host, port) = split_listen(listen);
    let listener = TcpListener::bind((host.as_str(), port)).await.ok()?;
    let store = Arc::new(Mutex::new(SeriesStore::new(extra_labels)));
    let st = store.clone();
    // Exporters keep their HTTP/2 connection open; connections run as child
    // tasks of the accept loop's JoinSet, so dropping the receiver closes them too.
    let server = tokio::spawn(async move {
        let mut conns = tokio::task::JoinSet::new();
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let st = st.clone();
            conns.spawn(async move {
                let svc = hyper::service::service_fn(move |req| handle(req, st.clone()));
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(stream), svc).await;
            });
            while conns.try_join_next().is_some() {}
        }
    });
    Some(OtlpReceiver { store, server })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{metric_label, metric_value};
    use std::io::Write;

    // Tiny protobuf encoder, just enough to build OTLP requests for the tests.
    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v > 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                return out;
            }
        }
    }
    fn tag(no: u64, wire: u64) -> Vec<u8> {
        varint((no << 3) | wire)
    }
    fn len(no: u64, body: impl AsRef<[u8]>) -> Vec<u8> {
        let b = body.as_ref();
        [tag(no, 2), varint(b.len() as u64), b.to_vec()].concat()
    }
    fn vint(no: u64, n: u64) -> Vec<u8> {
        [tag(no, 0), varint(n)].concat()
    }
    fn f64f(no: u64, d: f64) -> Vec<u8> {
        [tag(no, 1), d.to_le_bytes().to_vec()].concat()
    }
    fn i64f(no: u64, n: i64) -> Vec<u8> {
        [tag(no, 1), n.to_le_bytes().to_vec()].concat()
    }
    fn key_value(k: &str, v: &str) -> Vec<u8> {
        [len(1, k), len(2, len(1, v))].concat()
    }
    fn kv(k: &str, v: &str) -> Vec<u8> {
        len(1, key_value(k, v)) // as Resource.attributes (field 1)
    }

    fn request() -> Vec<u8> {
        let resource = [kv("service.name", "mainnet_node"), kv("service.version", "0.16.5"), kv("network", "mainnet")].concat();
        let gauge = len(2, [len(1, "monad.peer_disc.num_peers"), len(5, len(1, i64f(6, 213)))].concat());
        let cumulative = len(
            2,
            [len(1, "monad.state.consensus_events.commit_block"), len(7, [len(1, f64f(4, 16461.0)), vint(2, 2), vint(3, 1)].concat())]
                .concat(),
        );
        let delta = len(
            2,
            [
                len(1, "monad.delta_counter"),
                len(7, [len(1, [len(7, key_value("kind", "x")), i64f(6, 5)].concat()), vint(2, 1), vint(3, 1)].concat()),
            ]
            .concat(),
        );
        let histogram = len(2, [len(1, "monad.some_histogram"), len(9, len(1, []))].concat());
        let scope = len(2, [gauge, cumulative, delta, histogram].concat());
        len(1, [len(1, resource), scope].concat())
    }

    fn frame(flag: u8, body: &[u8]) -> Vec<u8> {
        [vec![flag], (body.len() as u32).to_be_bytes().to_vec(), body.to_vec()].concat()
    }

    #[test]
    fn decodes_gauges_and_sums_with_resource_and_point_labels() {
        let points = decode_export_request(&request()).unwrap();
        let summary: Vec<_> = points.iter().map(|p| (p.name.as_str(), p.value, p.delta)).collect();
        assert_eq!(
            summary,
            vec![
                ("monad_peer_disc_num_peers", 213.0, false),
                ("monad_state_consensus_events_commit_block", 16461.0, false),
                ("monad_delta_counter", 5.0, true),
            ]
        );
        let want: Labels =
            [("service_name", "mainnet_node"), ("service_version", "0.16.5"), ("network", "mainnet"), ("job", "mainnet_node")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        assert_eq!(points[0].labels, want);
        assert_eq!(points[2].labels["kind"], "x");
    }

    #[test]
    fn series_store_renders_prometheus_text() {
        let mut store = SeriesStore::new([("secp_key".to_string(), "035c51".to_string())].into());
        store.ingest(decode_export_request(&request()).unwrap(), 0);
        store.ingest(decode_export_request(&request()).unwrap(), 1000);
        let text = store.text(1000);
        assert_eq!(metric_value(&text, "monad_peer_disc_num_peers"), Some(213.0));
        assert_eq!(metric_value(&text, "monad_state_consensus_events_commit_block"), Some(16461.0));
        assert_eq!(metric_value(&text, "monad_delta_counter"), Some(10.0), "delta sums accumulate");
        assert_eq!(metric_label(&text, "monad_peer_disc_num_peers", "secp_key"), "035c51");
        assert_eq!(metric_label(&text, "monad_peer_disc_num_peers", "service_version"), "0.16.5");
        assert_eq!(store.text(10 * 60_000), "", "stale series expire");
    }

    #[test]
    fn unframe_handles_plain_and_gzip_messages() {
        let msg = request();
        assert_eq!(unframe(&frame(0, &msg), None).unwrap(), vec![msg.clone()]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&msg).unwrap();
        assert_eq!(unframe(&frame(1, &gz.finish().unwrap()), Some("gzip")).unwrap(), vec![msg]);
    }

    #[test]
    fn rejects_truncated_protobuf() {
        let mut req = request();
        req.truncate(req.len() - 3);
        assert!(decode_export_request(&req).is_err());
    }

    #[tokio::test]
    async fn receiver_answers_grpc_export_and_yields_none_when_port_busy() {
        let a = start_otlp_receiver("127.0.0.1:47317", Labels::new()).await.expect("first bind");
        assert!(start_otlp_receiver("127.0.0.1:47317", Labels::new()).await.is_none());

        // Real gRPC call against the first receiver.
        let stream = tokio::net::TcpStream::connect("127.0.0.1:47317").await.unwrap();
        let (mut client, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream)).await.unwrap();
        tokio::spawn(conn);
        let req = Request::builder()
            .method("POST")
            .uri(format!("http://127.0.0.1:47317{EXPORT_PATH}"))
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(http_body_util::Full::new(Bytes::from(frame(0, &request()))))
            .unwrap();
        let resp = client.send_request(req).await.unwrap();
        let collected = resp.into_body().collect().await.unwrap();
        assert_eq!(collected.trailers().and_then(|t| t.get("grpc-status")).unwrap(), "0");
        assert_eq!(metric_value(&a.text(), "monad_peer_disc_num_peers"), Some(213.0));
        assert!(a.last_export().is_some());
    }
}
