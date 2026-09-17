use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

use base64::Engine;
use hmac::{Hmac, Mac};
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha2::Sha256;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

const LEAF_CONFIG_CERT_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_config.psynet.gg.crt");
const LEAF_CONFIG_KEY_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_config.psynet.gg.key");
const LEAF_WS_CERT_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_ws.rlpp.psynet.gg.crt");
const LEAF_WS_KEY_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_ws.rlpp.psynet.gg.key");
const CA_CERT_PEM: &[u8] = include_bytes!("../resources/certs/velocityrl_ca.crt");

const PSY_CDN_KEY: &[u8] = b"cqhyz50f3c3j2pxhwo6b1kypxikah0wh";
const PSY_RESP_KEY: &[u8] = b"3b932153785842ac927744b292e40e52";
const PSY_REQ_KEY: &[u8] = b"c338bd36fb8c42b1a431d30add939fc7";

static PROXY_RUNNING: AtomicBool = AtomicBool::new(false);
static PROXY_STOP_TX: std::sync::Mutex<Option<oneshot::Sender<()>>> = std::sync::Mutex::new(None);
static BROKER_STOP_TX: std::sync::Mutex<Option<oneshot::Sender<()>>> = std::sync::Mutex::new(None);
static SPOOF_CONFIG: std::sync::LazyLock<Arc<RwLock<Option<crate::psynet::SpoofPayload>>>> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(None)));

#[derive(Clone, Debug)]
struct AuthWSCreds {
    token: String,
    session_id: String,
    timestamp: std::time::Instant,
}
static LAST_AUTH_WS: std::sync::Mutex<Option<AuthWSCreds>> = std::sync::Mutex::new(None);
static LAST_GAME_BUILD_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn extract_build_id_from_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for window in segments.windows(2) {
        if window[0].eq_ignore_ascii_case("battlecars") {
            return Some(window[1].to_string());
        }
    }
    None
}

fn remember_auth_player_ws(body: &[u8]) {
    let token = extract_json_string_field_value(body, "PsyToken");
    let session = extract_json_string_field_value(body, "SessionID");
    if let (Some(t), Some(s)) = (token, session) {
        if !t.is_empty() && !s.is_empty() {
            crate::applog::event(&format!("broker: cached AuthPlayer PsyToken & session ({:.8}...) for WS", s));
            let mut lock = LAST_AUTH_WS.lock().unwrap();
            *lock = Some(AuthWSCreds {
                token: t,
                session_id: s,
                timestamp: std::time::Instant::now(),
            });
        }
    }
}

fn extract_json_string_field_value(body: &[u8], key: &str) -> Option<String> {
    let prefix = format!("\"{key}\":\"");
    let prefix_bytes = prefix.as_bytes();
    let i = twoway_search(body, prefix_bytes)?;
    let val_start = i + prefix_bytes.len();
    let j = json_string_end(body, val_start)?;
    String::from_utf8(body[val_start..j].to_vec()).ok()
}

/// Port where the plain-HTTP WS broker listens. Rocket League connects here
/// after we rewrite `PsyNetUrl.PerConURLv2` in the config response.
const WS_BROKER_PORT: u16 = 27505;

pub fn ca_cert_bytes() -> &'static [u8] {
    CA_CERT_PEM
}

pub fn is_proxy_running() -> bool {
    PROXY_RUNNING.load(Ordering::SeqCst)
}

pub async fn set_spoof_config(cfg: crate::psynet::SpoofPayload) {
    let mut lock = SPOOF_CONFIG.write().await;
    *lock = Some(cfg);
}

pub async fn get_spoof_config() -> Option<crate::psynet::SpoofPayload> {
    SPOOF_CONFIG.read().await.clone()
}

#[derive(Debug)]
struct SniCertResolver {
    config_key: Arc<CertifiedKey>,
    ws_key: Arc<CertifiedKey>,
}

impl ResolvesServerCert for SniCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        if let Some(sni) = client_hello.server_name() {
            if sni.contains("ws.rlpp.psynet.gg") {
                return Some(self.ws_key.clone());
            }
        }
        Some(self.config_key.clone())
    }
}

fn load_certified_key(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<CertifiedKey>, String> {
    let mut cert_reader = std::io::Cursor::new(cert_pem);
    let mut certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("invalid cert pem: {e}"))?;

    let mut ca_reader = std::io::Cursor::new(CA_CERT_PEM);
    if let Ok(ca_certs) = rustls_pemfile::certs(&mut ca_reader).collect::<Result<Vec<_>, _>>() {
        certs.extend(ca_certs);
    }

    let mut key_reader = std::io::Cursor::new(key_pem);
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| format!("invalid key pem read: {e}"))?
        .ok_or_else(|| "no private key found in pem".to_string())?;

    let signing_key = tokio_rustls::rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|e| format!("failed to parse signing key: {e}"))?;

    Ok(Arc::new(CertifiedKey::new(certs, signing_key)))
}

fn create_tls_acceptor() -> Result<TlsAcceptor, String> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let config_key = load_certified_key(LEAF_CONFIG_CERT_PEM, LEAF_CONFIG_KEY_PEM)?;
    let ws_key = load_certified_key(LEAF_WS_CERT_PEM, LEAF_WS_KEY_PEM)?;

    let mut server_config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("failed to set safe protocol versions: {e}"))?
    .with_no_client_auth()
    .with_cert_resolver(Arc::new(SniCertResolver { config_key, ws_key }));

    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

#[derive(Debug)]
struct NoCertVerifier;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        tokio_rustls::rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn create_upstream_tls_connector() -> tokio_rustls::TlsConnector {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let client_config = tokio_rustls::rustls::ClientConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("safe default protocols")
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
    .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(client_config))
}

type ResponseBoxBody = BoxBody<Bytes, std::convert::Infallible>;

fn full_body<T: Into<Bytes>>(chunk: T) -> ResponseBoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn empty_body() -> ResponseBoxBody {
    Empty::new()
        .map_err(|never| match never {})
        .boxed()
}

pub async fn start_native_proxy() -> Result<(), String> {
    if is_proxy_running() {
        crate::applog::event("proxy: already running");
        return Ok(());
    }

    let acceptor = create_tls_acceptor()?;

    let listener_v4 = match tokio::net::TcpListener::bind("127.0.0.1:443").await {
        Ok(l) => l,
        Err(e) => {
            let msg = format!("Failed to bind 127.0.0.1:443: {e}");
            crate::applog::event(&format!("proxy: {msg}"));
            return Err(msg);
        }
    };

    let listener_v6 = match tokio::net::TcpListener::bind("[::1]:443").await {
        Ok(l) => {
            crate::applog::event("proxy: listening on [::1]:443 (IPv6 HTTPS)");
            Some(l)
        }
        Err(e) => {
            crate::applog::event(&format!(
                "proxy: could not bind [::1]:443: {e} (continuing IPv4 only)"
            ));
            None
        }
    };

    crate::applog::event("proxy: listening on 127.0.0.1:443 (IPv4 HTTPS)");

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    *PROXY_STOP_TX.lock().unwrap() = Some(stop_tx);
    PROXY_RUNNING.store(true, Ordering::SeqCst);

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .resolve("config.psynet.gg", "34.160.180.65:443".parse().unwrap())
        .build()
        .map_err(|e| format!("failed to create reqwest client: {e}"))?;

    tokio::spawn(async move {
        loop {
            let conn = tokio::select! {
                _ = &mut stop_rx => {
                    crate::applog::event("proxy: stop signal received, shutting down listener");
                    None
                }
                res = listener_v4.accept() => match res {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log::debug!("proxy v4 accept error: {e}");
                        continue;
                    }
                },
                res = async {
                    match &listener_v6 {
                        Some(l) => l.accept().await,
                        None => std::future::pending().await,
                    }
                } => match res {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log::debug!("proxy v6 accept error: {e}");
                        continue;
                    }
                },
            };

            let Some((stream, peer_addr)) = conn else {
                break;
            };

            let acceptor = acceptor.clone();
            let client = client.clone();

            tokio::spawn(async move {
                let tls_stream = match acceptor.accept(stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        let err_str = e.to_string();
                        // Benign disconnections: TCP health checks, probes, or clients closing immediately.
                        let is_benign = err_str.contains("tls handshake eof")
                            || err_str.contains("unexpected EOF")
                            || err_str.contains("connection reset")
                            || err_str.contains("broken pipe");
                        if !is_benign {
                            crate::applog::event(&format!("proxy TLS handshake error from {peer_addr}: {e}"));
                        }
                        return;
                    }
                };

                let io = TokioIo::new(tls_stream);
                let service = service_fn(move |req: Request<Incoming>| {
                    let client = client.clone();
                    async move {
                        handle_request(req, client).await
                    }
                });

                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                {
                    let err_str = e.to_string();
                    let is_benign = err_str.contains("connection reset")
                        || err_str.contains("broken pipe")
                        || err_str.contains("unexpected EOF")
                        || err_str.contains("error shutting down connection");
                    if !is_benign {
                        crate::applog::event(&format!("proxy connection error from {peer_addr}: {e}"));
                    }
                }
            });
        }
        PROXY_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

pub fn stop_native_proxy(_revert_hosts_file: bool) {
    if let Some(tx) = PROXY_STOP_TX.lock().unwrap().take() {
        let _ = tx.send(());
    }
    if let Some(tx) = BROKER_STOP_TX.lock().unwrap().take() {
        let _ = tx.send(());
    }
    PROXY_RUNNING.store(false, Ordering::SeqCst);
    crate::applog::event("proxy: stopped");
}

/// Start a plain-HTTP server on `127.0.0.1:WS_BROKER_PORT`.
/// Rocket League connects here after `PsyNetUrl.PerConURLv2` is rewritten in
/// the config response to `ws://127.0.0.1:<port>/ws/gc2`.
/// The broker accepts WS upgrades and forwards them to the real
/// `wss://ws.rlpp.psynet.gg:443`, applying fake-rank patching on the way back.
pub async fn start_ws_broker() -> Result<(), String> {
    let addr = format!("127.0.0.1:{WS_BROKER_PORT}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            let msg = format!("broker: Failed to bind {addr}: {e}");
            crate::applog::event(&msg);
            return Err(msg);
        }
    };
    crate::applog::event(&format!("broker: listening on {addr} (plain HTTP/WS broker)"));

    let client = reqwest::Client::builder()
        .http1_only()
        .danger_accept_invalid_certs(true)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .resolve("api.rlpp.psynet.gg", "34.54.194.77:443".parse().unwrap())
        .build()
        .map_err(|e| format!("failed to create reqwest client for broker: {e}"))?;

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    *BROKER_STOP_TX.lock().unwrap() = Some(stop_tx);

    tokio::spawn(async move {
        loop {
            let conn = tokio::select! {
                _ = &mut stop_rx => {
                    crate::applog::event("broker: stop signal received");
                    None
                }
                res = listener.accept() => match res {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log::debug!("broker accept error: {e}");
                        continue;
                    }
                },
            };

            let Some((stream, peer_addr)) = conn else { break };

            let client = client.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req: Request<Incoming>| {
                    let client = client.clone();
                    async move {
                        handle_broker_request(req, client).await
                    }
                });
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                {
                    log::debug!("broker connection error from {peer_addr}: {e}");
                }
            });
        }
    });

    Ok(())
}

/// Handle incoming requests on the plain-HTTP broker port (27505).
/// Upgrades WebSocket connections to the game server, and proxies HTTP RPC/Services calls to api.rlpp.psynet.gg.
async fn handle_broker_request(
    req: Request<Incoming>,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let path_str = req.uri().path().to_ascii_lowercase();
    if is_upgrade || path_str.starts_with("/ws") {
        return handle_websocket(req).await;
    }

    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_string();
    let upstream_url = format!("https://api.rlpp.psynet.gg{path_and_query}");

    let method = req.method().clone();
    let req_headers = req.headers().clone();
    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes().to_vec(),
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full_body(format!("broker read body error: {e}")))
                .unwrap());
        }
    };

    let mut up_builder = client.request(method, &upstream_url);
    for (k, v) in req_headers.iter() {
        let k_str = k.as_str().to_ascii_lowercase();
        // Skip hop-by-hop headers and headers that reqwest manages
        if k_str != "host"
            && k_str != "content-length"
            && k_str != "accept-encoding"
            && k_str != "connection"
            && k_str != "keep-alive"
            && k_str != "proxy-authenticate"
            && k_str != "proxy-authorization"
            && k_str != "te"
            && k_str != "trailers"
            && k_str != "transfer-encoding"
            && k_str != "upgrade"
        {
            up_builder = up_builder.header(k.as_str(), v.as_bytes());
        }
    }
    up_builder = up_builder.header("Host", "api.rlpp.psynet.gg");
    if !body_bytes.is_empty() {
        up_builder = up_builder.body(body_bytes);
    }

    let up_resp = match up_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            crate::applog::event(&format!("broker upstream error for {upstream_url}: {e:?}"));
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("broker upstream error: {e}")))
                .unwrap());
        }
    };

    let status = up_resp.status();
    let resp_headers = up_resp.headers().clone();
    let resp_bytes = match up_resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("broker read resp error: {e}")))
                .unwrap());
        }
    };

    let mut out_body = resp_bytes;
    let mut patched = false;

    let path_lower = path_and_query.to_ascii_lowercase();
    if path_lower.contains("authplayer") {
        remember_auth_player_ws(&out_body);
        let local_ws_v2 = format!("ws://127.0.0.1:{WS_BROKER_PORT}/ws/gc2");
        let local_ws_v1 = format!("ws://127.0.0.1:{WS_BROKER_PORT}/ws/gc?PsyConnectionType=Player");
        if let Some(next) = replace_json_string_field(&out_body, "PerConURLv2", &local_ws_v2) {
            out_body = next;
            patched = true;
        }
        if let Some(next) = replace_json_string_field(&out_body, "PerConURL", &local_ws_v1) {
            out_body = next;
            patched = true;
        }
        if patched {
            crate::applog::event("broker: AuthPlayer WS URL rewritten to local broker (/ws/gc2)");
        }
    }

    let mut resp_builder = Response::builder().status(status.as_u16());
    let mut psy_time = String::new();
    for (k, v) in resp_headers.iter() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower == "psytime" {
            if let Ok(s) = v.to_str() {
                psy_time = s.to_string();
            }
        }
        if k_lower != "content-length"
            && k_lower != "transfer-encoding"
            && k_lower != "content-encoding"
            && (!patched || (k_lower != "psysig" && k_lower != "psysignature"))
        {
            resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
        }
    }

    if patched {
        let sig = resign_rpc_response(&psy_time, &out_body);
        resp_builder = resp_builder.header("PsySig", sig);
    }

    resp_builder = resp_builder.header("Content-Length", out_body.len().to_string());
    Ok(resp_builder.body(full_body(out_body)).unwrap())
}

async fn handle_request(
    req: Request<Incoming>,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let host_hdr = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    if host_hdr.contains("ws.rlpp.psynet.gg") || is_upgrade {
        return handle_websocket(req).await;
    }

    handle_http_config(req, client).await
}

async fn handle_websocket(
    req: Request<Incoming>,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let uri = req.uri();
    let sec_key = match req.headers().get("sec-websocket-key").and_then(|v| v.to_str().ok()) {
        Some(k) => k.to_string(),
        None => {
            let resp = Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full_body("Missing Sec-WebSocket-Key"))
                .unwrap();
            return Ok(resp);
        }
    };

    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(sec_key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept_hash = hasher.finalize();
    let accept_val = base64::engine::general_purpose::STANDARD.encode(accept_hash);

    let mut path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/ws/gc2").to_string();
    if path_and_query.is_empty() || path_and_query == "/" || path_and_query == "/ws" {
        path_and_query = "/ws/gc2".to_string();
    } else if !path_and_query.starts_with('/') {
        path_and_query = format!("/{path_and_query}");
    }

    let upstream_url = format!("wss://ws.rlpp.psynet.gg{}", path_and_query);
    let mut up_builder = match tokio_tungstenite::tungstenite::handshake::client::Request::builder()
        .uri(&upstream_url)
        .header("Host", "ws.rlpp.psynet.gg")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())
    {
        Ok(r) => r,
        Err(e) => {
            crate::applog::event(&format!("proxy: failed to build upstream ws req: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("build ws req error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    let mut has_psy_token = false;
    let mut has_psy_session = false;
    let mut has_psy_build = false;
    let mut has_psy_env = false;

    for (k, v) in req.headers() {
        let name = k.as_str();
        if !name.eq_ignore_ascii_case("host")
            && !name.eq_ignore_ascii_case("connection")
            && !name.eq_ignore_ascii_case("upgrade")
            && !name.eq_ignore_ascii_case("sec-websocket-key")
            && !name.eq_ignore_ascii_case("sec-websocket-version")
            && !name.eq_ignore_ascii_case("sec-websocket-extensions")
            && !name.eq_ignore_ascii_case("origin")
        {
            if name.eq_ignore_ascii_case("psytoken") {
                has_psy_token = true;
            } else if name.eq_ignore_ascii_case("psysessionid") {
                has_psy_session = true;
            } else if name.eq_ignore_ascii_case("psybuildid") {
                has_psy_build = true;
            } else if name.eq_ignore_ascii_case("psyenvironment") {
                has_psy_env = true;
            }
            up_builder.headers_mut().insert(k.clone(), v.clone());
        }
    }

    if !has_psy_token || !has_psy_session {
        if let Some(creds) = LAST_AUTH_WS.lock().unwrap().clone() {
            if creds.timestamp.elapsed().as_secs() < 300 {
                if !has_psy_token {
                    if let Ok(val) = hyper::header::HeaderValue::from_str(&creds.token) {
                        up_builder.headers_mut().insert("PsyToken", val);
                    }
                }
                if !has_psy_session {
                    if let Ok(val) = hyper::header::HeaderValue::from_str(&creds.session_id) {
                        up_builder.headers_mut().insert("PsySessionID", val);
                    }
                }
                crate::applog::event("proxy: injected cached AuthPlayer PsyToken/PsySessionID into upstream WS");
            }
        }
    }

    if !has_psy_build {
        let fallback_build = LAST_GAME_BUILD_ID
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "-1887694083".to_string());
        if let Ok(val) = hyper::header::HeaderValue::from_str(&fallback_build) {
            up_builder.headers_mut().insert("PsyBuildID", val);
            crate::applog::event(&format!("proxy: set WebSocket PsyBuildID: {fallback_build}"));
        }
    }
    if !has_psy_env {
        up_builder.headers_mut().insert("PsyEnvironment", hyper::header::HeaderValue::from_static("Prod"));
    }

    up_builder.headers_mut().insert(
        hyper::header::ORIGIN,
        hyper::header::HeaderValue::from_static("https://ws.rlpp.psynet.gg"),
    );

    let connector = create_upstream_tls_connector();
    let tcp_conn = match tokio::net::TcpStream::connect("34.149.116.40:443").await {
        Ok(t) => t,
        Err(e) => {
            crate::applog::event(&format!("proxy: direct connect to 34.149.116.40:443 failed ({e}), trying DNS ws.rlpp.psynet.gg:443"));
            match tokio::net::TcpStream::connect("ws.rlpp.psynet.gg:443").await {
                Ok(t) => t,
                Err(e2) => {
                    crate::applog::event(&format!("proxy: failed to connect to upstream ws: {e2}"));
                    let resp = Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .body(full_body(format!("connect ws upstream error: {e2}")))
                        .unwrap();
                    return Ok(resp);
                }
            }
        }
    };

    let server_name = match tokio_rustls::rustls::pki_types::ServerName::try_from("ws.rlpp.psynet.gg".to_string()) {
        Ok(sn) => sn,
        Err(e) => {
            crate::applog::event(&format!("proxy: invalid ServerName: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body("invalid server name"))
                .unwrap();
            return Ok(resp);
        }
    };

    let tls_upstream = match connector.connect(server_name, tcp_conn).await {
        Ok(s) => s,
        Err(e) => {
            crate::applog::event(&format!("proxy: upstream ws tls handshake error: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream ws tls error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    let (upstream_ws, _) = match tokio_tungstenite::client_async(up_builder, tls_upstream).await {
        Ok(pair) => pair,
        Err(e) => {
            crate::applog::event(&format!("proxy: upstream ws handshake failed: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream ws handshake error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    crate::applog::event("proxy: WebSocket upstream connected to ws.rlpp.psynet.gg");

    tokio::spawn(async move {
        let upgraded = match hyper::upgrade::on(req).await {
            Ok(u) => u,
            Err(e) => {
                crate::applog::event(&format!("proxy: client ws upgrade error: {e}"));
                return;
            }
        };

        let client_ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        ).await;

        crate::applog::event("proxy: WebSocket client tunnel established");
        tunnel_websocket(client_ws, upstream_ws).await;
    });

    let resp = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Accept", accept_val)
        .body(empty_body())
        .unwrap();

    Ok(resp)
}

async fn tunnel_websocket<S1, S2>(
    client_ws: tokio_tungstenite::WebSocketStream<S1>,
    upstream_ws: tokio_tungstenite::WebSocketStream<S2>,
) where
    S1: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    S2: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use futures_util::{SinkExt, StreamExt};
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut up_tx, mut up_rx) = upstream_ws.split();

    let c2u = tokio::spawn(async move {
        while let Some(msg_res) = client_rx.next().await {
            match msg_res {
                Ok(msg) => {
                    if up_tx.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let u2c = tokio::spawn(async move {
        while let Some(msg_res) = up_rx.next().await {
            match msg_res {
                Ok(msg) => {
                    let out_msg = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => {
                            let (patched_text, _) = patch_ws_frame_text(&t).await;
                            tokio_tungstenite::tungstenite::Message::Text(patched_text.into())
                        }
                        tokio_tungstenite::tungstenite::Message::Binary(b) => {
                            let (patched_b, _) = patch_ws_frame_binary(&b).await;
                            tokio_tungstenite::tungstenite::Message::Binary(patched_b.into())
                        }
                        other => other,
                    };
                    if client_tx.send(out_msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = c2u => {},
        _ = u2c => {},
    }
    crate::applog::event("proxy: WebSocket tunnel closed");
}

async fn patch_ws_frame_text(text: &str) -> (String, bool) {
    let (patched_bytes, changed) = patch_ws_frame_binary(text.as_bytes()).await;
    if changed {
        if let Ok(s) = String::from_utf8(patched_bytes) {
            return (s, true);
        }
    }
    (text.to_string(), false)
}

async fn patch_ws_frame_binary(frame: &[u8]) -> (Vec<u8>, bool) {
    let Some(hdr_end) = twoway_search(frame, b"\r\n\r\n") else {
        return (frame.to_vec(), false);
    };

    let headers_part = &frame[..hdr_end];
    let body_part = &frame[hdr_end + 4..];

    let svc = get_ws_header_value(headers_part, "PsyService");
    let is_skill = svc.contains("skills/getplayerskill")
        || svc.contains("skills/getplayersskills")
        || (twoway_search(body_part, b"\"Skills\"").is_some()
            && (twoway_search(body_part, b"\"Mu\"").is_some()
                || twoway_search(body_part, b"\"Tier\"").is_some()
                || twoway_search(body_part, b"\"Playlist\"").is_some()));
    let is_leaderboard = svc.contains("skills/getskillleaderboardvalueforuser")
        || (twoway_search(body_part, b"\"LeaderboardID\"").is_some()
            && (twoway_search(body_part, b"\"bHasSkill\"").is_some()
                || twoway_search(body_part, b"\"MMR\"").is_some()
                || twoway_search(body_part, b"\"Value\"").is_some()));

    if !is_skill && !is_leaderboard {
        return (frame.to_vec(), false);
    }

    if is_skill {
        extract_and_save_real_skills(body_part);
    }

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };

    let Some(cfg) = cfg_opt else {
        return (frame.to_vec(), false);
    };

    let Some(fake_ranks) = &cfg.fake_ranks else {
        return (frame.to_vec(), false);
    };

    let features = crate::features::get_cached_features();
    if !fake_ranks.enabled || !features.flags.fake_ranks || !crate::features::is_build_supported() {
        return (frame.to_vec(), false);
    }

    let (new_body, changed) = if is_skill {
        patch_get_player_skill_json(body_part, fake_ranks)
    } else if is_leaderboard {
        patch_leaderboard_value_json(body_part, fake_ranks)
    } else {
        (body_part.to_vec(), false)
    };

    if !changed {
        return (frame.to_vec(), false);
    }

    let new_headers = resign_ws_headers(headers_part, &new_body);
    let mut out = Vec::with_capacity(new_headers.len() + 4 + new_body.len());
    out.extend_from_slice(&new_headers);
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(&new_body);

    crate::applog::event(&format!(
        "proxy: patched fake ranks ws frame ({} -> {} bytes)",
        frame.len(),
        out.len()
    ));

    (out, true)
}

fn get_ws_header_value(headers: &[u8], key: &str) -> String {
    let text = String::from_utf8_lossy(headers);
    let key_lower = key.to_ascii_lowercase();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(&key_lower) {
                return v.trim().to_string();
            }
        }
    }
    String::new()
}

fn replace_ws_header_value(headers: &[u8], key: &str, new_val: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(headers);
    let mut out_lines = Vec::new();
    let key_lower = key.to_ascii_lowercase();
    let mut replaced = false;

    for line in text.lines() {
        if let Some((k, _)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(&key_lower) {
                out_lines.push(format!("{key}: {new_val}"));
                replaced = true;
                continue;
            }
        }
        out_lines.push(line.to_string());
    }

    if !replaced {
        out_lines.push(format!("{key}: {new_val}"));
    }

    out_lines.join("\r\n").into_bytes()
}

fn resign_ws_headers(headers: &[u8], body: &[u8]) -> Vec<u8> {
    let psy_time = get_ws_header_value(headers, "PsyTime");
    let has_psysig = twoway_search(headers, b"PsySig:").is_some() || twoway_search(headers, b"psysig:").is_some();
    let has_psysignature = twoway_search(headers, b"Psysignature:").is_some() || twoway_search(headers, b"psysignature:").is_some();

    if !has_psysig && !has_psysignature {
        return headers.to_vec();
    }

    let sig = if !psy_time.is_empty() {
        let mut m = Hmac::<Sha256>::new_from_slice(PSY_RESP_KEY).expect("valid hmac key");
        m.update(format!("{psy_time}-").as_bytes());
        m.update(body);
        base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
    } else {
        let mut m = Hmac::<Sha256>::new_from_slice(PSY_REQ_KEY).expect("valid hmac key");
        m.update(b"-");
        m.update(body);
        base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
    };

    let key_to_replace = if has_psysig { "PsySig" } else { "Psysignature" };
    replace_ws_header_value(headers, key_to_replace, &sig)
}

fn patch_get_player_skill_json(
    body: &[u8],
    fake_ranks: &crate::psynet::FakeRanksPayload,
) -> (Vec<u8>, bool) {
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let result_obj = if let Some(r) = root.get_mut("Result") {
        r
    } else {
        &mut root
    };

    let mut changed = false;

    if let Some(skills_arr) = result_obj.get_mut("Skills").and_then(|s| s.as_array_mut()) {
        for skill_val in skills_arr {
            let pl = get_playlist_id(skill_val);
            if let Some(ov) = get_playlist_override(fake_ranks, pl) {
                if apply_rank_override(skill_val, ov) {
                    changed = true;
                }
            }
        }
    } else if let Some(players_arr) = result_obj.get_mut("Players").and_then(|p| p.as_array_mut()) {
        for player_val in players_arr {
            if let Some(skills_arr) = player_val.get_mut("Skills").and_then(|s| s.as_array_mut()) {
                for skill_val in skills_arr {
                    let pl = get_playlist_id(skill_val);
                    if let Some(ov) = get_playlist_override(fake_ranks, pl) {
                        if apply_rank_override(skill_val, ov) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    if let Some(rl) = &fake_ranks.reward_levels {
        if let Some(reward_obj) = result_obj.get_mut("RewardLevels") {
            if let Some(lvl) = rl.season_level {
                reward_obj["SeasonLevel"] = serde_json::json!(lvl);
                changed = true;
            }
            if let Some(wins) = rl.season_level_wins {
                reward_obj["SeasonLevelWins"] = serde_json::json!(wins);
                changed = true;
            }
        }
    }

    if !changed {
        return (body.to_vec(), false);
    }

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn patch_leaderboard_value_json(
    body: &[u8],
    fake_ranks: &crate::psynet::FakeRanksPayload,
) -> (Vec<u8>, bool) {
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let pl = root.get("LeaderboardID").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let Some(ov) = get_playlist_override(fake_ranks, pl) else {
        return (body.to_vec(), false);
    };

    let mu_opt = if let Some(disp) = ov.display_mmr {
        Some(mu_from_display(disp))
    } else {
        ov.mu
    };

    let Some(mu) = mu_opt else {
        return (body.to_vec(), false);
    };

    let disp = (mu * 20.0 + 100.0).round() as i64;
    root["Value"] = serde_json::json!(disp);
    root["MMR"] = serde_json::json!(mu);
    root["bHasSkill"] = serde_json::json!(true);

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn get_playlist_id(skill: &serde_json::Value) -> i32 {
    skill.get("Playlist").and_then(|v| {
        v.as_i64().map(|n| n as i32).or_else(|| {
            v.as_str().and_then(|s| s.parse::<i32>().ok())
        })
    }).unwrap_or(0)
}

fn get_playlist_override<'a>(
    fake_ranks: &'a crate::psynet::FakeRanksPayload,
    playlist: i32,
) -> Option<&'a crate::psynet::FakeRankOverridePayload> {
    if let Some(playlists) = &fake_ranks.playlists {
        let pl_str = playlist.to_string();
        if let Some(ov) = playlists.get(&pl_str) {
            return Some(ov);
        }
    }
    fake_ranks.default.as_ref()
}

fn apply_rank_override(
    skill: &mut serde_json::Value,
    ov: &crate::psynet::FakeRankOverridePayload,
) -> bool {
    let mut modified = false;
    if let Some(disp) = ov.display_mmr {
        let mu = mu_from_display(disp);
        skill["Mu"] = serde_json::json!(mu);
        skill["MMR"] = serde_json::json!(mu);
        modified = true;
    } else if let Some(mu) = ov.mu {
        let clamped_mu = mu_from_display(mu * 20.0 + 100.0);
        skill["Mu"] = serde_json::json!(clamped_mu);
        skill["MMR"] = serde_json::json!(clamped_mu);
        modified = true;
    }

    if let Some(sigma) = ov.sigma {
        skill["Sigma"] = serde_json::json!(sigma);
        modified = true;
    }
    if let Some(tier) = ov.tier {
        skill["Tier"] = serde_json::json!(tier);
        modified = true;
    }
    if let Some(div) = ov.division {
        skill["Division"] = serde_json::json!(div);
        modified = true;
    }
    if let Some(ws) = ov.win_streak {
        skill["WinStreak"] = serde_json::json!(ws);
        modified = true;
    }

    modified
}

fn mu_from_display(disp: f64) -> f64 {
    (disp.max(0.0) - 100.0) / 20.0
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RealSkillEntry {
    playlist: i32,
    mu: f64,
    sigma: f64,
    display_mmr: i32,
    tier: i32,
    division: i32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RealSkillFile {
    skills: Vec<RealSkillEntry>,
}

fn extract_and_save_real_skills(body: &[u8]) {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return;
    };
    let result_obj = root.get("Result").unwrap_or(&root);
    let skills_opt = result_obj.get("Skills").and_then(|s| s.as_array())
        .or_else(|| {
            result_obj.get("Players")
                .and_then(|p| p.as_array())
                .and_then(|arr| arr.first())
                .and_then(|p0| p0.get("Skills"))
                .and_then(|s| s.as_array())
        });

    let Some(skills_arr) = skills_opt else {
        return;
    };

    let mut entries = Vec::new();
    for s in skills_arr {
        let pl = get_playlist_id(s);
        let mu = s.get("Mu").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sigma = s.get("Sigma").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let disp = (mu * 20.0 + 100.0).max(0.0);
        let tier = s.get("Tier").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let div = s.get("Division").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        entries.push(RealSkillEntry {
            playlist: pl,
            mu: (mu * 10000.0).round() / 10000.0,
            sigma: (sigma * 10000.0).round() / 10000.0,
            display_mmr: disp.round() as i32,
            tier,
            division: div,
        });
    }

    if entries.is_empty() {
        return;
    }

    let file_data = RealSkillFile { skills: entries };
    let Ok(json_str) = serde_json::to_string_pretty(&file_data) else {
        return;
    };

    let dir = crate::psynet::config_dir();
    let path = dir.join("real_skill.json");
    let _ = std::fs::write(&path, json_str);
    crate::applog::event(&format!(
        "proxy: saved {} playlists to real_skill.json",
        file_data.skills.len()
    ));
}

async fn handle_http_config(
    req: Request<Incoming>,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let uri = req.uri();
    let path = uri.path();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("https://config.psynet.gg{path}{query}");

    if let Some(bid) = extract_build_id_from_path(path) {
        let mut lock = LAST_GAME_BUILD_ID.lock().unwrap();
        if lock.as_deref() != Some(&bid) {
            crate::applog::event(&format!("proxy: detected game build ID: {bid}"));
            *lock = Some(bid);
        }
    }

    let is_battlecars = path.to_ascii_lowercase().contains("/config/battlecars/");
    crate::applog::event(&format!(
        "proxy: >>> {} {} (battlecars={})",
        req.method(),
        path,
        is_battlecars
    ));

    let mut up_builder = client.request(req.method().clone(), &upstream_url);
    for (k, v) in req.headers() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower != "host"
            && k_lower != "accept-encoding"
            && k_lower != "if-none-match"
            && k_lower != "if-modified-since"
            && k_lower != "if-match"
            && k_lower != "if-unmodified-since"
            && k_lower != "if-range"
        {
            up_builder = up_builder.header(k.as_str(), v.as_bytes());
        }
    }

    let up_resp = match up_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            crate::applog::event(&format!("proxy: upstream error: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    let status = up_resp.status();
    let headers = up_resp.headers().clone();
    let body_bytes = match up_resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("read body error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    let mut out_body = body_bytes;
    let mut patched = false;

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };
    if let Some(cfg) = cfg_opt {
        if crate::features::is_build_supported() {
            let (next_body, changed) = patch_config(&out_body, &cfg);
            if changed {
                out_body = next_body;
                patched = true;
                crate::applog::event(&format!(
                    "proxy: patched config ({} bytes)",
                    out_body.len()
                ));
            }
        }
    }

    if !patched {
        crate::applog::event(&format!(
            "proxy: unpatched config ({} bytes, Psysignature generated)",
            out_body.len()
        ));
    }

    let mut resp_builder = Response::builder().status(status.as_u16());
    for (k, v) in headers.iter() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower != "content-length"
            && k_lower != "transfer-encoding"
            && k_lower != "content-encoding"
            && k_lower != "psysignature"
            && k_lower != "psysig"
            && k_lower != "etag"
            && k_lower != "last-modified"
        {
            resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
        }
    }

    // Prevent client-side caching of config responses
    resp_builder = resp_builder.header("Cache-Control", "no-cache, no-store, must-revalidate");
    resp_builder = resp_builder.header("Pragma", "no-cache");
    resp_builder = resp_builder.header("Expires", "0");

    // Always provide a valid Psysignature (signed with PSY_CDN_KEY) so Rocket League always accepts the config
    let sig = resign_config_cdn(&out_body);
    resp_builder = resp_builder.header("Psysignature", sig);

    resp_builder = resp_builder.header("Content-Length", out_body.len().to_string());
    let resp = resp_builder.body(full_body(out_body)).unwrap();
    Ok(resp)
}

fn resign_config_cdn(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(PSY_CDN_KEY).expect("valid HMAC key");
    mac.update(body);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn resign_rpc_response(psy_time: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(PSY_RESP_KEY).expect("valid HMAC key");
    mac.update(format!("{psy_time}-").as_bytes());
    mac.update(body);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// Rewrite `PsyNetUrl.URL` and `PsyNetUrl.URLv2` in the battlecars config body
/// to route WebSocket connections and RPC requests through our local broker.
fn patch_psynet_url(body: &[u8]) -> Option<Vec<u8>> {
    let (obj_start, obj_end) = find_named_object(body, "PsyNetUrl")?;
    let obj = body[obj_start..obj_end].to_vec();

    let local_services = format!("http://127.0.0.1:{WS_BROKER_PORT}/Services");
    let local_rpc = format!("http://127.0.0.1:{WS_BROKER_PORT}/rpc");

    let mut patched_obj = obj;
    let mut changed = false;

    if let Some(next) = replace_json_string_field(&patched_obj, "URLv2", &local_rpc) {
        patched_obj = next;
        changed = true;
    }

    if let Some(next) = replace_json_string_field(&patched_obj, "URL", &local_services) {
        patched_obj = next;
        changed = true;
    }

    if !changed {
        return None;
    }

    let mut out = Vec::with_capacity(body.len() + 64);
    out.extend_from_slice(&body[..obj_start]);
    out.extend_from_slice(&patched_obj);
    out.extend_from_slice(&body[obj_end..]);
    Some(out)
}

/// Replace the value of a JSON string field `"key":"<old_value>"` inside `body`.
/// Returns `Some(new_body)` if the field was found and the value differed, `None` otherwise.
fn replace_json_string_field(body: &[u8], key: &str, new_value: &str) -> Option<Vec<u8>> {
    // Encode new_value as a JSON string (without surrounding quotes).
    let encoded_json = serde_json::to_string(new_value).ok()?;
    if encoded_json.len() < 2 {
        return None;
    }
    let encoded = &encoded_json.as_bytes()[1..encoded_json.len() - 1];

    let prefix = format!("\"{key}\":\"");
    let prefix_bytes = prefix.as_bytes();
    let i = twoway_search(body, prefix_bytes)?;
    let val_start = i + prefix_bytes.len();
    let j = json_string_end(body, val_start)?;

    if &body[val_start..j] == encoded {
        return None; // already set to the desired value
    }

    let mut out = Vec::with_capacity(body.len() + encoded.len());
    out.extend_from_slice(&body[..val_start]);
    out.extend_from_slice(encoded);
    out.extend_from_slice(&body[j..]);
    Some(out)
}

fn scan_object_end(body: &[u8], start: usize) -> Option<usize> {
    let mut in_str = false;
    let mut esc = false;
    let mut depth = 0;
    for (i, &c) in body[start..].iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
                continue;
            }
            if c == b'\\' {
                esc = true;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn json_string_end(body: &[u8], val_start: usize) -> Option<usize> {
    let mut esc = false;
    for (j, &c) in body[val_start..].iter().enumerate() {
        if esc {
            esc = false;
            continue;
        }
        if c == b'\\' {
            esc = true;
            continue;
        }
        if c == b'"' {
            return Some(val_start + j);
        }
    }
    None
}

fn find_named_object(body: &[u8], name: &str) -> Option<(usize, usize)> {
    let key = format!("\"{name}\"");
    let key_bytes = key.as_bytes();
    let at = twoway_search(body, key_bytes)?;
    let mut i = at + key_bytes.len();
    while i < body.len() && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= body.len() || body[i] != b':' {
        return None;
    }
    i += 1;
    while i < body.len() && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= body.len() || body[i] != b'{' {
        return None;
    }
    let close = scan_object_end(body, i)?;
    Some((i, close + 1))
}

fn twoway_search(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn replace_equip_text(body: &[u8], equip_id: &str, new_text: &str) -> Option<Vec<u8>> {
    let encoded_json = serde_json::to_string(new_text).ok()?;
    if encoded_json.len() < 2 {
        return None;
    }
    let encoded = &encoded_json.as_bytes()[1..encoded_json.len() - 1];

    let prefix = format!("\"ID\":\"{equip_id}\",\"Text\":\"");
    let val_start = if let Some(i) = twoway_search(body, prefix.as_bytes()) {
        i + prefix.len()
    } else {
        let id_pat = format!("\"ID\":\"{equip_id}\"");
        let id_at = twoway_search(body, id_pat.as_bytes())?;
        let mut start = id_at;
        while start > 0 && body[start] != b'{' {
            start -= 1;
        }
        let end = scan_object_end(body, start)? + 1;
        let obj = &body[start..end];
        let tkey = b"\"Text\":\"";
        let k = twoway_search(obj, tkey)?;
        start + k + tkey.len()
    };

    let j = json_string_end(body, val_start)?;
    if &body[val_start..j] == encoded {
        return None;
    }

    let mut out = Vec::with_capacity(body.len() + encoded.len());
    out.extend_from_slice(&body[..val_start]);
    out.extend_from_slice(encoded);
    out.extend_from_slice(&body[j..]);
    Some(out)
}

fn replace_equip_category(body: &[u8], equip_id: &str, new_cat: &str) -> Option<Vec<u8>> {
    let id_pat = format!("\"ID\":\"{equip_id}\"");
    let id_at = twoway_search(body, id_pat.as_bytes())?;
    let mut start = id_at;
    while start > 0 && body[start] != b'{' {
        start -= 1;
    }
    let end = scan_object_end(body, start)? + 1;
    let obj = &body[start..end];
    let ckey = b"\"Category\":\"";

    if let Some(k) = twoway_search(obj, ckey) {
        let val_start = start + k + ckey.len();
        let j = json_string_end(body, val_start)?;
        if &body[val_start..j] == new_cat.as_bytes() {
            return None;
        }
        let mut out = Vec::with_capacity(body.len() + new_cat.len());
        out.extend_from_slice(&body[..val_start]);
        out.extend_from_slice(new_cat.as_bytes());
        out.extend_from_slice(&body[j..]);
        Some(out)
    } else {
        let insert_at = start + id_at - start + id_pat.len();
        let frag = format!(",\"Category\":\"{new_cat}\"");
        let mut out = Vec::with_capacity(body.len() + frag.len());
        out.extend_from_slice(&body[..insert_at]);
        out.extend_from_slice(frag.as_bytes());
        out.extend_from_slice(&body[insert_at..]);
        Some(out)
    }
}

pub fn is_hex6(s: &str) -> bool {
    s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn upsert_title_category(body: &[u8], cat_id: &str, color: &str, glow_color: &str) -> (Vec<u8>, bool) {
    let Some((ptc_start, ptc_end)) = find_named_object(body, "PlayerTitleConfig") else {
        return (body.to_vec(), false);
    };

    let def = format!(
        "{{\"ID\":\"{cat_id}\",\"Color\":\"{color}\",\"GlowColor\":\"{glow_color}\"}}"
    );

    let ptc_obj = &body[ptc_start..ptc_end];
    let key = b"\"Categories\"";
    let Some(k) = twoway_search(ptc_obj, key) else {
        return (body.to_vec(), false);
    };

    let mut i = k + key.len();
    while i < ptc_obj.len() && ptc_obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= ptc_obj.len() || ptc_obj[i] != b':' {
        return (body.to_vec(), false);
    }
    i += 1;
    while i < ptc_obj.len() && ptc_obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= ptc_obj.len() || ptc_obj[i] != b'[' {
        return (body.to_vec(), false);
    }

    let arr_start = ptc_start + i;
    let Some(arr_end) = scan_array_end(body, arr_start) else {
        return (body.to_vec(), false);
    };

    // Check if category already exists in Categories array
    let id_needle = format!("\"ID\":\"{cat_id}\"");
    let arr_slice = &body[arr_start..=arr_end];
    if let Some(id_at) = twoway_search(arr_slice, id_needle.as_bytes()) {
        let abs_id = arr_start + id_at;
        let mut obj_s = abs_id;
        while obj_s > arr_start && body[obj_s] != b'{' {
            obj_s -= 1;
        }
        if let Some(obj_e) = scan_object_end(body, obj_s) {
            if &body[obj_s..=obj_e] == def.as_bytes() {
                return (body.to_vec(), false);
            }
            let mut out = Vec::with_capacity(body.len() + def.len());
            out.extend_from_slice(&body[..obj_s]);
            out.extend_from_slice(def.as_bytes());
            out.extend_from_slice(&body[obj_e + 1..]);
            return (out, true);
        }
    }

    // Insert at beginning of array [ {def}, ... ]
    let insert_at = arr_start + 1;
    let inner_is_empty = body[arr_start + 1..arr_end].iter().all(|c| c.is_ascii_whitespace());
    let frag = if inner_is_empty {
        def
    } else {
        format!("{def},")
    };

    let mut out = Vec::with_capacity(body.len() + frag.len());
    out.extend_from_slice(&body[..insert_at]);
    out.extend_from_slice(frag.as_bytes());
    out.extend_from_slice(&body[insert_at..]);
    (out, true)
}

pub fn sanitize_category_part(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "title".to_string()
    } else {
        out
    }
}

pub fn patch_config(body: &[u8], cfg: &crate::psynet::SpoofPayload) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let mut any_change = false;

    if cfg.enabled {
        let equip_id = cfg.equip_title_id.trim();
        let display_id = cfg.display_title_id.trim();
        let mut custom_text = cfg.custom_text.trim();
        if custom_text.is_empty() {
            custom_text = cfg.custom_name.trim();
        }

        let cat = cfg.category.trim();
        let clean_cat = if !cat.is_empty() {
            sanitize_category_part(cat)
        } else {
            String::new()
        };

        let mut registered_category_colors: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();

        if !equip_id.is_empty() {
            if !custom_text.is_empty() {
                if let Some(next) = replace_equip_text(&out, equip_id, custom_text) {
                    out = next;
                    any_change = true;
                }
            }

            if let Some(tc) = &cfg.title_color {
                if is_hex6(&tc.color) {
                    let glow = if is_hex6(&tc.glow_color) { &tc.glow_color } else { &tc.color };
                    let custom_cat = format!("RLItemMod_{}", sanitize_category_part(equip_id));
                    let color_up = tc.color.to_ascii_uppercase();
                    let glow_up = glow.to_ascii_uppercase();

                    let mut can_apply = true;
                    if let Some((existing_c, existing_g)) = registered_category_colors.get(&custom_cat) {
                        if existing_c != &color_up || existing_g != &glow_up {
                            crate::applog::event(&format!(
                                "proxy: WARNING: Category '{}' already has custom color (#{}/#{}); ignoring conflicting color (#{}/#{}) for title '{}'",
                                custom_cat, existing_c, existing_g, color_up, glow_up, equip_id
                            ));
                            can_apply = false;
                        }
                    } else {
                        registered_category_colors.insert(custom_cat.clone(), (color_up, glow_up));
                    }

                    if can_apply {
                        let (next, did) = upsert_title_category(&out, &custom_cat, &tc.color, glow);
                        if did {
                            out = next;
                            any_change = true;
                        }
                        if let Some(next) = replace_equip_category(&out, equip_id, &custom_cat) {
                            out = next;
                            any_change = true;
                        }
                    }
                } else if !clean_cat.is_empty() {
                    if let Some(next) = replace_equip_category(&out, equip_id, &clean_cat) {
                        out = next;
                        any_change = true;
                    }
                }
            } else if !clean_cat.is_empty() {
                if let Some(next) = replace_equip_category(&out, equip_id, &clean_cat) {
                    out = next;
                    any_change = true;
                }
            }
        }

        if let Some(swaps) = &cfg.swaps {
            for sw in swaps {
                let target_id = if !sw.equip_title_id.trim().is_empty() {
                    sw.equip_title_id.trim()
                } else if !sw.display_title_id.trim().is_empty() {
                    sw.display_title_id.trim()
                } else {
                    ""
                };

                if target_id.is_empty() {
                    continue;
                }

                let sw_text = sw.custom_text.trim();
                if !sw_text.is_empty() {
                    if let Some(next) = replace_equip_text(&out, target_id, sw_text) {
                        out = next;
                        any_change = true;
                    }
                } else if !sw.display_title_id.trim().is_empty()
                    && sw.display_title_id.trim() != target_id
                    && sw.display_title_id.trim() != "custom"
                {
                    let disp = sw.display_title_id.trim();
                    let src_pat = format!("\"ID\":\"{disp}\"");
                    if let Some(src_at) = twoway_search(&out, src_pat.as_bytes()) {
                        let mut start = src_at;
                        while start > 0 && out[start] != b'{' {
                            start -= 1;
                        }
                        if let Some(end) = scan_object_end(&out, start) {
                            let src_obj = &out[start..end + 1];
                            let tkey = b"\"Text\":\"";
                            if let Some(k) = twoway_search(src_obj, tkey) {
                                let val_start = start + k + tkey.len();
                                if let Some(j) = json_string_end(&out, val_start) {
                                    let text_val = String::from_utf8_lossy(&out[val_start..j]).to_string();
                                    if !text_val.is_empty() {
                                        if let Some(next) = replace_equip_text(&out, target_id, &text_val) {
                                            out = next;
                                            any_change = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let sw_cat = sw.category.trim();
                let clean = if !sw_cat.is_empty() {
                    sanitize_category_part(sw_cat)
                } else {
                    String::new()
                };

                if let Some(tc) = &sw.title_color {
                    if is_hex6(&tc.color) {
                        let glow = if is_hex6(&tc.glow_color) { &tc.glow_color } else { &tc.color };
                        let custom_cat = format!("RLItemMod_{}", sanitize_category_part(target_id));
                        let color_up = tc.color.to_ascii_uppercase();
                        let glow_up = glow.to_ascii_uppercase();

                        let mut can_apply = true;
                        if let Some((existing_c, existing_g)) = registered_category_colors.get(&custom_cat) {
                            if existing_c != &color_up || existing_g != &glow_up {
                                crate::applog::event(&format!(
                                    "proxy: WARNING: Category '{}' already has custom color (#{}/#{}); ignoring conflicting color (#{}/#{}) for title '{}'",
                                    custom_cat, existing_c, existing_g, color_up, glow_up, target_id
                                ));
                                can_apply = false;
                            }
                        } else {
                            registered_category_colors.insert(custom_cat.clone(), (color_up, glow_up));
                        }

                        if can_apply {
                            let (next, did) = upsert_title_category(&out, &custom_cat, &tc.color, glow);
                            if did {
                                out = next;
                                any_change = true;
                            }
                            if let Some(next) = replace_equip_category(&out, target_id, &custom_cat) {
                                out = next;
                                any_change = true;
                            }
                        }
                    } else if !clean.is_empty() {
                        if let Some(next) = replace_equip_category(&out, target_id, &clean) {
                            out = next;
                            any_change = true;
                        }
                    }
                } else if !clean.is_empty() {
                    if let Some(next) = replace_equip_category(&out, target_id, &clean) {
                        out = next;
                        any_change = true;
                    }
                }
            }
        }

        if custom_text.is_empty() && !display_id.is_empty() && display_id != equip_id && display_id != "custom" {
            let src_pat = format!("\"ID\":\"{display_id}\"");
            if let Some(src_at) = twoway_search(&out, src_pat.as_bytes()) {
                let mut start = src_at;
                while start > 0 && out[start] != b'{' {
                    start -= 1;
                }
                if let Some(end) = scan_object_end(&out, start) {
                    let src_obj = &out[start..end + 1];
                    let tkey = b"\"Text\":\"";
                    if let Some(k) = twoway_search(src_obj, tkey) {
                        let val_start = start + k + tkey.len();
                        if let Some(j) = json_string_end(&out, val_start) {
                            let text_val = String::from_utf8_lossy(&out[val_start..j]).to_string();
                            if !text_val.is_empty() && !equip_id.is_empty() {
                                if let Some(next) = replace_equip_text(&out, equip_id, &text_val) {
                                    out = next;
                                    any_change = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let features = crate::features::get_cached_features();

    if features.flags.camera_spoof {
        if let Some(cam) = &cfg.camera_spoof {
            if cam.enabled {
                let (next, changed) = patch_camera_class_properties(&out, cam);
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.rich_palette {
        if let Some(palette) = &cfg.palette_spoof {
            if palette.enabled {
                let (next, changed) = patch_palette_class_properties(&out);
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.dynamic_logos {
        if let Some(logo) = &cfg.logo_spoof {
            if logo.enabled && !logo.logo_url.trim().is_empty() {
                let (next, changed) = patch_dynamic_logos_config(&out, logo.logo_url.trim());
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.blog_motd {
        if let Some(blog) = &cfg.blog_spoof {
            if blog.enabled && !blog.motd.trim().is_empty() {
                let (next, changed) = patch_blog_config(&out, blog.motd.trim());
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.fake_ranks {
        if let Some(fr) = &cfg.fake_ranks {
            if fr.enabled {
                if let Some(next) = patch_psynet_url(&out) {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    (out, any_change)
}

fn patch_dynamic_logos_config(body: &[u8], url: &str) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let has_escaped_slash = twoway_search(body, b"\\/").is_some();
    let encoded_url = if has_escaped_slash {
        url.replace('/', "\\/")
    } else {
        url.to_string()
    };

    let Some((start, end)) = find_named_object(&out, "DynamicLogosConfig") else {
        // Inject DynamicLogosConfig before the last '}'
        let Some(close_idx) = out.iter().rposition(|&c| c == b'}') else {
            return (out, false);
        };
        let block = format!(
            ",\"DynamicLogosConfig\":{{\"Class\":\"DynamicLogosConfig_X\",\"bUseDynamicLogos\":true,\"LogoURL\":\"{encoded_url}\"}}"
        );
        let mut res = Vec::with_capacity(out.len() + block.len());
        res.extend_from_slice(&out[..close_idx]);
        res.extend_from_slice(block.as_bytes());
        res.extend_from_slice(&out[close_idx..]);
        return (res, true);
    };

    let mut changed = false;

    // Force bUseDynamicLogos: true
    let obj = out[start..end].to_vec();
    if let Some(b_at) = twoway_search(&obj, b"\"bUseDynamicLogos\":") {
        let val_start = start + b_at + b"\"bUseDynamicLogos\":".len();
        let mut val_end = val_start;
        while val_end < out.len() && out[val_end].is_ascii_alphanumeric() {
            val_end += 1;
        }
        if &out[val_start..val_end] != b"true" {
            let mut next = Vec::with_capacity(out.len() + 4);
            next.extend_from_slice(&out[..val_start]);
            next.extend_from_slice(b"true");
            next.extend_from_slice(&out[val_end..]);
            out = next;
            changed = true;
        }
    }

    // Replace LogoURL
    let (cur_start, cur_end) = match find_named_object(&out, "DynamicLogosConfig") {
        Some(b) => b,
        None => return (out, changed),
    };
    let cur_obj = out[cur_start..cur_end].to_vec();
    let mut found_url = false;
    for key in &["LogoURL", "LogoUrl", "SeasonLogo", "SeasonLogoURL", "LogoImageURL", "DynamicLogoURL"] {
        let pat = format!("\"{key}\":\"");
        if let Some(k_at) = twoway_search(&cur_obj, pat.as_bytes()) {
            found_url = true;
            let val_start = cur_start + k_at + pat.len();
            if let Some(val_end) = json_string_end(&out, val_start) {
                if &out[val_start..val_end] != encoded_url.as_bytes() {
                    let mut next = Vec::with_capacity(out.len() + encoded_url.len());
                    next.extend_from_slice(&out[..val_start]);
                    next.extend_from_slice(encoded_url.as_bytes());
                    next.extend_from_slice(&out[val_end..]);
                    out = next;
                    changed = true;
                    break;
                }
            }
        }
    }

    if !found_url {
        let close_obj = cur_end - 1;
        if out[close_obj] == b'}' {
            let snippet = format!(",\"LogoURL\":\"{encoded_url}\"");
            let mut next = Vec::with_capacity(out.len() + snippet.len());
            next.extend_from_slice(&out[..close_obj]);
            next.extend_from_slice(snippet.as_bytes());
            next.extend_from_slice(&out[close_obj..]);
            out = next;
            changed = true;
        }
    }

    (out, changed)
}

fn json_string_contents(s: &str) -> Option<Vec<u8>> {
    let serialized = serde_json::to_string(s).ok()?;
    if serialized.len() >= 2 && serialized.starts_with('"') && serialized.ends_with('"') {
        Some(serialized[1..serialized.len() - 1].as_bytes().to_vec())
    } else {
        None
    }
}

fn patch_blog_config(body: &[u8], motd: &str) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let Some(encoded_motd) = json_string_contents(motd) else {
        return (out, false);
    };
    let Some((start, end)) = find_named_object(&out, "BlogConfig") else {
        // Inject BlogConfig before the last '}'
        let Some(close_idx) = out.iter().rposition(|&c| c == b'}') else {
            return (out, false);
        };
        let mut block = Vec::new();
        block.extend_from_slice(b",\"BlogConfig\":{\"Class\":\"BlogConfig_X\",\"MotD\":\"");
        block.extend_from_slice(&encoded_motd);
        block.extend_from_slice(b"\"}");
        let mut res = Vec::with_capacity(out.len() + block.len());
        res.extend_from_slice(&out[..close_idx]);
        res.extend_from_slice(&block);
        res.extend_from_slice(&out[close_idx..]);
        return (res, true);
    };

    let obj = out[start..end].to_vec();
    let mut changed = false;

    for key in &["MotD", "Motd", "MOTD", "NewsText"] {
        let pat = format!("\"{key}\":\"");
        if let Some(k_at) = twoway_search(&obj, pat.as_bytes()) {
            let val_start = start + k_at + pat.len();
            if let Some(val_end) = json_string_end(&out, val_start) {
                if &out[val_start..val_end] != encoded_motd.as_slice() {
                    let mut next = Vec::with_capacity(out.len() + encoded_motd.len());
                    next.extend_from_slice(&out[..val_start]);
                    next.extend_from_slice(&encoded_motd);
                    next.extend_from_slice(&out[val_end..]);
                    out = next;
                    changed = true;
                    break;
                }
            }
        }
    }

    (out, changed)
}

fn format_camera_limit(min: f64, max: f64, interval: f64, def_min: f64, def_max: f64, def_interval: f64) -> String {
    let mut actual_min = min;
    let mut actual_max = max;
    let mut actual_interval = interval;
    if actual_max <= 0.0 && actual_min <= 0.0 {
        actual_min = def_min;
        actual_max = def_max;
    }
    if actual_interval <= 0.0 {
        actual_interval = def_interval;
    }
    if actual_max < actual_min {
        actual_max = actual_min;
    }
    format!("(Min={:.6},Max={:.6},interval={:.6})", actual_min, actual_max, actual_interval)
}

fn patch_camera_class_properties(body: &[u8], cam: &crate::psynet::CameraSpoofPayload) -> (Vec<u8>, bool) {
    let fov_str = format_camera_limit(cam.fov.min, cam.fov.max, cam.fov.interval, 60.0, 1000.0, 1.0);
    let height_str = format_camera_limit(cam.height.min, cam.height.max, cam.height.interval, 40.0, 1000.0, 1.0);
    let dist_str = format_camera_limit(cam.distance.min, cam.distance.max, cam.distance.interval, 100.0, 1000.0, 1.0);

    let targets = [
        ("Camera_TA", "FOVLimits", fov_str),
        ("Camera_TA", "HeightLimits", height_str),
        ("Camera_TA", "DistanceLimits", dist_str),
    ];

    let mut out = body.to_vec();
    let mut changed = false;

    for (class_name, prop_name, val_str) in &targets {
        let (next, did) = upsert_class_property_override(&out, class_name, prop_name, val_str);
        if did {
            out = next;
            changed = true;
        }
    }

    (out, changed)
}

fn patch_palette_class_properties(body: &[u8]) -> (Vec<u8>, bool) {
    let val_str = "CarColorSet_TA'CarColors.OrangeTeamV2'";
    upsert_class_property_override(body, "Team_Soccar_TA", "CarColorSet", val_str)
}

fn find_class_property_config(body: &[u8]) -> Option<(usize, usize)> {
    let key = b"\"ClassPropertyConfig\"";
    let at = twoway_search(body, key)?;
    let mut i = at + key.len();
    while i < body.len() && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= body.len() || body[i] != b':' {
        return None;
    }
    i += 1;
    while i < body.len() && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= body.len() || body[i] != b'{' {
        return None;
    }
    let close = scan_object_end(body, i)?;
    Some((i, close + 1))
}

fn find_overrides_array(body: &[u8], obj_start: usize, obj_end: usize) -> Option<(usize, usize)> {
    let obj = &body[obj_start..obj_end];
    let key = b"\"Overrides\"";
    let at = twoway_search(obj, key)?;
    let mut i = at + key.len();
    while i < obj.len() && obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= obj.len() || obj[i] != b':' {
        return None;
    }
    i += 1;
    while i < obj.len() && obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= obj.len() || obj[i] != b'[' {
        return None;
    }
    let arr_start = obj_start + i;
    let close = scan_array_end(body, arr_start)?;
    Some((arr_start, close + 1))
}

fn scan_array_end(body: &[u8], start: usize) -> Option<usize> {
    let mut in_str = false;
    let mut esc = false;
    let mut depth = 0;
    for (i, &c) in body[start..].iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
                continue;
            }
            if c == b'\\' {
                esc = true;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    if c == b']' {
                        return Some(start + i);
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn upsert_class_property_override(body: &[u8], class_name: &str, prop_name: &str, new_value: &str) -> (Vec<u8>, bool) {
    let (obj_start, obj_end) = match find_class_property_config(body) {
        Some(bounds) => bounds,
        None => {
            // Inject ClassPropertyConfig before the last '}'
            let Some(close_idx) = body.iter().rposition(|&c| c == b'}') else {
                return (body.to_vec(), false);
            };
            let block = format!(
                ",\"ClassPropertyConfig\":{{\"Class\":\"ClassPropertyConfig_X\",\"Overrides\":[{{\"Class\":\"{class_name}\",\"Property\":\"{prop_name}\",\"Value\":\"{new_value}\"}}]}}"
            );
            let mut out = Vec::with_capacity(body.len() + block.len());
            out.extend_from_slice(&body[..close_idx]);
            out.extend_from_slice(block.as_bytes());
            out.extend_from_slice(&body[close_idx..]);
            return (out, true);
        }
    };

    let (arr_start, arr_end) = match find_overrides_array(body, obj_start, obj_end) {
        Some(bounds) => bounds,
        None => return (body.to_vec(), false),
    };

    let inner_start = arr_start + 1;
    let inner_end = arr_end - 1;
    if inner_start > inner_end {
        return (body.to_vec(), false);
    }

    // Search for existing entry with Class and Property matching
    let mut search_from = inner_start;
    while search_from < inner_end {
        let Some(rel_class) = twoway_search(&body[search_from..inner_end], b"\"Class\"") else {
            break;
        };
        let abs_class_key = search_from + rel_class;
        let mut obj_s = abs_class_key;
        while obj_s > arr_start && body[obj_s] != b'{' {
            obj_s -= 1;
        }
        if body[obj_s] != b'{' {
            search_from = abs_class_key + 1;
            continue;
        }
        let Some(obj_e) = scan_object_end(body, obj_s) else {
            search_from = abs_class_key + 1;
            continue;
        };
        let entry_obj = &body[obj_s..=obj_e];

        // Check if Class matches
        let class_pat = format!("\"Class\":\"{class_name}\"");
        let prop_pat = format!("\"Property\":\"{prop_name}\"");

        if twoway_search(entry_obj, class_pat.as_bytes()).is_some() && twoway_search(entry_obj, prop_pat.as_bytes()).is_some() {
            // Found existing entry! Update its "Value" field
            let val_key = b"\"Value\":\"";
            let Some(vk) = twoway_search(entry_obj, val_key) else {
                return (body.to_vec(), false);
            };
            let val_start = obj_s + vk + val_key.len();
            let Some(val_end) = json_string_end(body, val_start) else {
                return (body.to_vec(), false);
            };

            if &body[val_start..val_end] == new_value.as_bytes() {
                return (body.to_vec(), false); // Already equal
            }

            let mut out = Vec::with_capacity(body.len() + new_value.len());
            out.extend_from_slice(&body[..val_start]);
            out.extend_from_slice(new_value.as_bytes());
            out.extend_from_slice(&body[val_end..]);
            return (out, true);
        }

        search_from = obj_e + 1;
    }

    // Not found in existing Overrides array — append new entry into array
    let new_entry = format!("{{\"Class\":\"{class_name}\",\"Property\":\"{prop_name}\",\"Value\":\"{new_value}\"}}");
    let inner = &body[inner_start..inner_end];
    let is_empty = inner.iter().all(|c| c.is_ascii_whitespace());

    let insert_str = if is_empty {
        new_entry
    } else {
        format!(",{new_entry}")
    };

    let mut out = Vec::with_capacity(body.len() + insert_str.len());
    out.extend_from_slice(&body[..inner_end]);
    out.extend_from_slice(insert_str.as_bytes());
    out.extend_from_slice(&body[inner_end..]);
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_palette_class_properties() {
        let input = br#"{"ClassPropertyConfig":{"Class":"ClassPropertyConfig_X","Overrides":[{"Class":"GFxData_MusicPlayer_TA","Property":"bDebugMusicPlayer","Value":"true"},{"Class":"Camera_TA","Property":"FOVLimits","Value":"(Min=1.000000,Max=1000.000000,interval=1.000000)"}]}}"#;
        let (patched, changed) = patch_palette_class_properties(input);
        assert!(changed);
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"Class\":\"Team_Soccar_TA\""));
        assert!(s.contains("\"Property\":\"CarColorSet\""));
        assert!(s.contains("\"Value\":\"CarColorSet_TA'CarColors.OrangeTeamV2'\""));
    }

    #[test]
    fn test_extract_build_id_from_path() {
        assert_eq!(
            extract_build_id_from_path("/v2/Config/BattleCars/-1887694083/Prod/Epic/INT/"),
            Some("-1887694083".to_string())
        );
        assert_eq!(
            extract_build_id_from_path("/Config/BattleCars/99999/"),
            Some("99999".to_string())
        );
        assert_eq!(extract_build_id_from_path("/favicon.ico"), None);
    }

    #[test]
    fn test_resign_config_cdn() {
        let body = b"test payload";
        let sig = resign_config_cdn(body);
        assert!(!sig.is_empty());
        // Verify deterministic output
        assert_eq!(sig, resign_config_cdn(body));
    }
}

