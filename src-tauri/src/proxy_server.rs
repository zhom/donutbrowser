use crate::proxy_storage::ProxyConfig;
use crate::traffic_stats::{get_traffic_tracker, init_traffic_tracker, LiveTrafficTracker};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use regex_lite::Regex;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

/// Combined read+write trait for tunnel target streams, allowing
/// `handle_connect_from_buffer` to handle plain TCP, SOCKS, and
/// Shadowsocks through the same bidirectional-copy path.
pub(crate) trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}
pub(crate) type BoxedAsyncStream = Box<dyn AsyncStream>;
use url::Url;

enum CompiledRule {
  Regex(Regex),
  Exact(String),
}

#[derive(Clone)]
pub struct BypassMatcher {
  rules: Arc<Vec<CompiledRule>>,
}

impl BypassMatcher {
  pub fn new(rules: &[String]) -> Self {
    let compiled = rules
      .iter()
      .map(|rule| match Regex::new(rule) {
        Ok(re) => CompiledRule::Regex(re),
        Err(_) => CompiledRule::Exact(rule.clone()),
      })
      .collect();
    Self {
      rules: Arc::new(compiled),
    }
  }

  pub fn should_bypass(&self, host: &str) -> bool {
    self.rules.iter().any(|rule| match rule {
      CompiledRule::Regex(re) => re.is_match(host),
      CompiledRule::Exact(exact) => host == exact,
    })
  }
}

#[derive(Clone)]
pub struct BlocklistMatcher {
  domains: Arc<HashSet<String>>,
  /// When true the `domains` set is an ALLOW list: a host is blocked unless it
  /// (or a parent domain) is present. When false it's a block list (default).
  allowlist_mode: bool,
}

impl Default for BlocklistMatcher {
  fn default() -> Self {
    Self::new()
  }
}

impl BlocklistMatcher {
  pub fn new() -> Self {
    Self {
      domains: Arc::new(HashSet::new()),
      allowlist_mode: false,
    }
  }

  pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
    Self::from_file_with_mode(path, false)
  }

  pub fn from_file_with_mode(
    path: &str,
    allowlist_mode: bool,
  ) -> Result<Self, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let domains: HashSet<String> = content
      .lines()
      .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
      .map(|line| line.trim().to_lowercase())
      .collect();
    log::info!(
      "[blocklist] Loaded {} domains from {} (mode={})",
      domains.len(),
      path,
      if allowlist_mode { "allow" } else { "block" }
    );
    Ok(Self {
      domains: Arc::new(domains),
      allowlist_mode,
    })
  }

  /// True if `host` (or any parent domain) is in the set.
  fn set_contains(&self, host_lower: &str) -> bool {
    if self.domains.contains(host_lower) {
      return true;
    }
    // Suffix matching: check parent domains (like uBlock)
    let mut start = 0;
    while let Some(dot_pos) = host_lower[start..].find('.') {
      start += dot_pos + 1;
      if self.domains.contains(&host_lower[start..]) {
        return true;
      }
    }
    false
  }

  pub fn is_blocked(&self, host: &str) -> bool {
    // Empty set = no filtering in either mode. In allowlist mode an empty list
    // would otherwise block everything and brick the browser, so fail open.
    if self.domains.is_empty() {
      return false;
    }
    let host_lower = host.to_lowercase();
    let in_set = self.set_contains(&host_lower);
    if self.allowlist_mode {
      // Allow only listed domains; block everything else.
      !in_set
    } else {
      in_set
    }
  }
}

#[derive(Clone, Copy)]
enum TrafficDirection {
  Sent,
  Received,
}

/// Wrapper stream that counts bytes successfully relayed to its destination.
struct CountingStream<S> {
  inner: S,
  bytes_written: Arc<AtomicU64>,
  write_direction: TrafficDirection,
  // Resolved once per stream: the global tracker is fixed after init, so the
  // hot poll paths avoid taking the global RwLock on every packet
  tracker: Option<Arc<LiveTrafficTracker>>,
}

impl<S> CountingStream<S> {
  fn new(inner: S, write_direction: TrafficDirection) -> Self {
    Self {
      inner,
      bytes_written: Arc::new(AtomicU64::new(0)),
      write_direction,
      tracker: get_traffic_tracker(),
    }
  }
}

impl<S: AsyncRead + Unpin> AsyncRead for CountingStream<S> {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_read(cx, buf)
  }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CountingStream<S> {
  fn poll_write(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<io::Result<usize>> {
    let result = Pin::new(&mut self.inner).poll_write(cx, buf);
    if let Poll::Ready(Ok(n)) = &result {
      self.bytes_written.fetch_add(*n as u64, Ordering::Relaxed);
      if let Some(tracker) = &self.tracker {
        match self.write_direction {
          TrafficDirection::Sent => tracker.add_bytes_sent(*n as u64),
          TrafficDirection::Received => tracker.add_bytes_received(*n as u64),
        }
      }
    }
    result
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_flush(cx)
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_shutdown(cx)
  }
}

// Wrapper to prepend consumed bytes to a stream.
//
// Generic over the inner stream rather than fixed to `TcpStream`: the upstream
// hop is a bare socket for `http`/`https` but a `TlsStream<TcpStream>` for
// `httpstls`, and both need the same coalesced-payload replay.
struct PrependReader<S> {
  prepended: Vec<u8>,
  prepended_pos: usize,
  inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for PrependReader<S> {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    // First, read from prepended bytes if any
    if self.prepended_pos < self.prepended.len() {
      let available = self.prepended.len() - self.prepended_pos;
      let to_copy = available.min(buf.remaining());
      buf.put_slice(&self.prepended[self.prepended_pos..self.prepended_pos + to_copy]);
      self.prepended_pos += to_copy;
      return Poll::Ready(Ok(()));
    }

    // Then read from inner stream
    Pin::new(&mut self.inner).poll_read(cx, buf)
  }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrependReader<S> {
  fn poll_write(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<io::Result<usize>> {
    Pin::new(&mut self.inner).poll_write(cx, buf)
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_flush(cx)
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_shutdown(cx)
  }
}

async fn handle_request(
  req: Request<hyper::body::Incoming>,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) -> Result<Response<Full<Bytes>>, Infallible> {
  // CONNECT cannot be tunneled on the hyper path: hyper owns the connection
  // and would keep parsing the post-200 tunnel bytes (TLS) as HTTP. This is
  // only reachable when a kept-alive connection that started as plain HTTP
  // later sends CONNECT — refuse and close so the browser retries on a fresh
  // connection, which the peek path classifies as CONNECT and tunnels.
  if req.method() == Method::CONNECT {
    let mut response = Response::new(Full::new(Bytes::from(
      "CONNECT is not supported on a reused connection",
    )));
    *response.status_mut() = StatusCode::NOT_IMPLEMENTED;
    response.headers_mut().insert(
      hyper::header::CONNECTION,
      hyper::header::HeaderValue::from_static("close"),
    );
    return Ok(response);
  }

  // Handle regular HTTP requests
  handle_http(req, upstream_url, bypass_matcher, blocklist_matcher).await
}

/// Extract percent-decoded (username, password) from the upstream URL.
///
/// `url::Url::username()` / `Url::password()` return percent-encoded ASCII
/// strings per the WHATWG spec. `build_proxy_url` on the producer side
/// already percent-encodes the credentials with `urlencoding::encode`, so
/// we must decode here — otherwise the upstream SOCKS5 / HTTP CONNECT
/// receives `%40` instead of `@`, breaking RFC1929 user/password
/// authentication or HTTP Basic-Auth
fn upstream_userpass(upstream: &Url) -> (String, String) {
  let username = urlencoding::decode(upstream.username())
    .map(|cow| cow.into_owned())
    .unwrap_or_default();
  let password = urlencoding::decode(upstream.password().unwrap_or(""))
    .map(|cow| cow.into_owned())
    .unwrap_or_default();
  (username, password)
}

/// Transparent AsyncRead/AsyncWrite wrapper that logs every read/write
/// byte of the SOCKS5 handshake. Used only during the handshake — the
/// inner stream is taken back via `into_inner` once the handshake
/// completes, so the tunnel phase pays no overhead
struct SocksHandshakeLogger<S> {
  inner: S,
  label: String,
}

impl<S> SocksHandshakeLogger<S> {
  fn new(inner: S, label: String) -> Self {
    Self { inner, label }
  }

  fn into_inner(self) -> S {
    self.inner
  }
}

impl<S: AsyncRead + Unpin> AsyncRead for SocksHandshakeLogger<S> {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    let before = buf.filled().len();
    let result = Pin::new(&mut self.inner).poll_read(cx, buf);
    if let Poll::Ready(Ok(())) = &result {
      let after = buf.filled().len();
      if after > before {
        let bytes = &buf.filled()[before..after];
        log::trace!(
          "[socks-handshake:{}] <- {} byte(s): {:02x?}",
          self.label,
          bytes.len(),
          bytes
        );
      } else {
        log::trace!("[socks-handshake:{}] <- EOF (peer closed)", self.label);
      }
    }
    result
  }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for SocksHandshakeLogger<S> {
  fn poll_write(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<io::Result<usize>> {
    let result = Pin::new(&mut self.inner).poll_write(cx, buf);
    if let Poll::Ready(Ok(n)) = &result {
      log::trace!(
        "[socks-handshake:{}] -> {} byte(s): {:02x?}",
        self.label,
        n,
        &buf[..*n]
      );
    }
    result
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_flush(cx)
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Pin::new(&mut self.inner).poll_shutdown(cx)
  }
}

async fn connect_via_socks(
  socks_addr: &str,
  target_host: &str,
  target_port: u16,
  is_socks5: bool,
  auth: Option<(&str, &str)>,
) -> Result<TcpStream, Box<dyn std::error::Error>> {
  let stream = tokio::time::timeout(UPSTREAM_DIAL_TIMEOUT, TcpStream::connect(socks_addr))
    .await
    .map_err(|_| format!("SOCKS upstream connect to {socks_addr} timed out"))??;

  if is_socks5 {
    // SOCKS5 connection using async_socks5
    use async_socks5::{connect, AddrKind, Auth};

    let target = if let Ok(ip) = target_host.parse::<std::net::IpAddr>() {
      AddrKind::Ip(std::net::SocketAddr::new(ip, target_port))
    } else {
      AddrKind::Domain(target_host.to_string(), target_port)
    };

    let auth_info: Option<Auth> = auth.map(|(user, pass)| Auth {
      username: user.to_string(),
      password: pass.to_string(),
    });

    let has_auth = auth_info.is_some();
    log::trace!(
      "[socks-handshake] dialing {} (target={}:{}, has_auth={})",
      socks_addr,
      target_host,
      target_port,
      has_auth
    );

    // Disable Nagle so the kernel doesn't further delay/coalesce the
    // syscalls issued when BufStream flushes
    let _ = stream.set_nodelay(true);

    // BufStream wrapping is required: async_socks5 calls write_u8 for every
    // single-byte SOCKS5 / RFC1929 field, and on a raw TcpStream each call
    // becomes its own TCP segment. Some upstream SOCKS5 implementations
    // treat such a "fragmented auth submission" as a misbehaving client
    // and silently FIN instead of returning an RFC1929 status. BufStream
    // coalesces those small writes into one syscall on flush — this is
    // the usage pattern shown in the async_socks5 README
    let label = format!("{socks_addr}->{target_host}:{target_port}");
    let logged = SocksHandshakeLogger::new(stream, label);
    let mut buffered = tokio::io::BufStream::new(logged);
    let handshake = tokio::time::timeout(
      UPSTREAM_DIAL_TIMEOUT,
      connect(&mut buffered, target, auth_info),
    )
    .await;
    // Unwrap the layered stream: BufStream → SocksHandshakeLogger → TcpStream
    let stream = buffered.into_inner().into_inner();
    match handshake {
      Ok(Ok(_)) => {
        log::trace!("[socks-handshake] handshake completed ok");
        Ok(stream)
      }
      Ok(Err(e)) => {
        log::trace!("[socks-handshake] handshake failed: {:?}", e);
        Err(e.into())
      }
      Err(_) => {
        log::trace!("[socks-handshake] handshake timed out");
        Err("SOCKS5 upstream handshake timed out".into())
      }
    }
  } else {
    let mut stream = stream;
    // SOCKS4 - simplified implementation
    let ip: std::net::IpAddr = target_host.parse()?;

    let mut request = vec![0x04, 0x01]; // SOCKS4, CONNECT
    request.extend_from_slice(&target_port.to_be_bytes());
    match ip {
      std::net::IpAddr::V4(ipv4) => {
        request.extend_from_slice(&ipv4.octets());
      }
      std::net::IpAddr::V6(_) => {
        return Err("SOCKS4 does not support IPv6".into());
      }
    }
    request.push(0); // NULL terminator for userid

    stream.write_all(&request).await?;

    let mut response = [0u8; 8];
    stream.read_exact(&mut response).await?;

    if response[1] != 0x5A {
      return Err("SOCKS4 connection failed".into());
    }

    Ok(stream)
  }
}

/// How the body of a buffered response is framed on the wire.
enum BufferedBody {
  /// The body follows the header block in `bytes` exactly as the upstream sent
  /// it.
  AsSent,
  /// The upstream used `Transfer-Encoding: chunked`; this is the de-framed body.
  Dechunked(Vec<u8>),
  /// The upstream declared chunked but the framing never completed. Nothing can
  /// be forwarded: the chunk-size lines are not body bytes, and a half-decoded
  /// body reaches the browser as a complete-looking short one.
  BrokenChunks,
}

/// A buffered HTTP response read off a raw upstream stream.
struct BufferedHttpResponse {
  bytes: Vec<u8>,
  body: BufferedBody,
  /// True when the read stopped at `MAX_HTTP_HEADER_BUFFER` /
  /// `MAX_HTTP_RESPONSE_BUFFER` rather than at the end of the response, so
  /// `bytes` holds only a prefix. Callers must fail the request instead of
  /// forwarding it: hyper derives a fresh Content-Length from whatever body it
  /// is handed, so a truncated response reaches the browser as a well-formed,
  /// self-consistent short one and silently corrupts the download.
  truncated: bool,
}

/// Progress of a chunked body walk.
enum ChunkedState {
  /// The terminating zero-length chunk was reached.
  Complete,
  /// Well-formed so far, but the terminating chunk has not arrived yet.
  Incomplete,
  /// The framing itself is broken, so no further byte of it can be trusted.
  Malformed,
}

/// Decode as much of a `Transfer-Encoding: chunked` body as `body` holds,
/// appending the payload to `out` and advancing `cursor` past every chunk
/// consumed in full. Carrying the cursor across reads keeps a body that arrives
/// in many pieces a single linear walk instead of one per read.
///
/// Any trailer section after the zero-length chunk is dropped; hyper re-derives
/// the framing of the response it sends.
fn decode_chunked(body: &[u8], cursor: &mut usize, out: &mut Vec<u8>) -> ChunkedState {
  loop {
    let rest = &body[*cursor..];
    let Some(line_end) = rest.windows(2).position(|w| w == b"\r\n") else {
      return ChunkedState::Incomplete;
    };
    let Ok(header) = std::str::from_utf8(&rest[..line_end]) else {
      return ChunkedState::Malformed;
    };
    // A chunk extension (`;name=value`) may follow the size and carries nothing
    // this proxy acts on.
    let size_text = header.split(';').next().unwrap_or("").trim();
    let Ok(size) = usize::from_str_radix(size_text, 16) else {
      return ChunkedState::Malformed;
    };
    // A chunk larger than the whole buffer cap can never be satisfied, and
    // rejecting it here keeps the offset arithmetic below overflow-free.
    if size > MAX_HTTP_RESPONSE_BUFFER {
      return ChunkedState::Malformed;
    }
    if size == 0 {
      return ChunkedState::Complete;
    }
    let data_start = line_end + 2;
    let data_end = data_start + size;
    let Some(trailing) = rest.get(data_end..) else {
      return ChunkedState::Incomplete;
    };
    if trailing.len() < 2 {
      return ChunkedState::Incomplete;
    }
    if !trailing.starts_with(b"\r\n") {
      return ChunkedState::Malformed;
    }
    out.extend_from_slice(&rest[data_start..data_end]);
    *cursor += data_end + 2;
  }
}

/// True when this raw header block declares `Transfer-Encoding: chunked`.
fn declares_chunked(header_block: &[u8]) -> bool {
  String::from_utf8_lossy(header_block).lines().any(|line| {
    let line = line.to_lowercase();
    line.starts_with("transfer-encoding:") && line.contains("chunked")
  })
}

/// Headers hyper re-derives for the `Full<Bytes>` body this proxy builds, plus
/// the hop-by-hop set. Forwarding the upstream's own framing would fight
/// hyper's and corrupt every response through these paths.
const NON_FORWARDED_RESPONSE_HEADERS: &[&str] = &[
  "content-length",
  "transfer-encoding",
  "connection",
  "keep-alive",
  "proxy-connection",
  "upgrade",
  "trailer",
  "te",
];

/// Copy an upstream's response headers onto a response assembled from raw
/// bytes. The SOCKS4 and Shadowsocks paths speak HTTP by hand, and without this
/// a redirect loses its `Location`, a sign-in loses its `Set-Cookie` and a
/// compressed body arrives with no `Content-Encoding` to undo it.
///
/// `header_block` is the raw header bytes including the status line; a trailing
/// blank line is tolerated. A line that does not parse is dropped rather than
/// failing the whole response, and `HeaderName`/`HeaderValue` do the rejecting,
/// so a hostile upstream cannot smuggle a header past this.
fn forward_upstream_headers(response: &mut Response<Full<Bytes>>, header_block: &[u8]) {
  let block = String::from_utf8_lossy(header_block);
  for line in block.split("\r\n").skip(1) {
    let Some((name, value)) = line.split_once(':') else {
      continue;
    };
    let name = name.trim();
    if NON_FORWARDED_RESPONSE_HEADERS
      .iter()
      .any(|skipped| name.eq_ignore_ascii_case(skipped))
    {
      continue;
    }
    let (Ok(name), Ok(value)) = (
      HeaderName::from_bytes(name.as_bytes()),
      HeaderValue::from_str(value.trim()),
    ) else {
      continue;
    };
    // `append`, not `insert`: every `Set-Cookie` has to survive.
    response.headers_mut().append(name, value);
  }
}

/// Read a full HTTP response from `stream` into a buffer: headers first
/// (capped at `MAX_HTTP_HEADER_BUFFER` — a peer streaming data that never
/// contains CRLFCRLF must not grow memory unboundedly), then the body per
/// Content-Length or until close, with the total capped at
/// `MAX_HTTP_RESPONSE_BUFFER`. Hitting either cap sets `truncated`.
async fn read_http_response_buffer<S: AsyncRead + Unpin>(stream: &mut S) -> BufferedHttpResponse {
  let mut response_buffer = Vec::with_capacity(8192);
  let mut temp_buf = [0u8; 4096];
  let mut content_length: Option<usize> = None;
  let mut is_chunked = false;
  let mut truncated = false;
  let mut body = BufferedBody::AsSent;

  // Read until we have complete headers
  loop {
    if response_buffer.len() > MAX_HTTP_HEADER_BUFFER {
      log::warn!(
        "HTTP response headers exceeded {} bytes without terminating; aborting read",
        MAX_HTTP_HEADER_BUFFER
      );
      truncated = true;
      break;
    }
    match stream.read(&mut temp_buf).await {
      Ok(0) => break, // Connection closed
      Ok(n) => {
        response_buffer.extend_from_slice(&temp_buf[..n]);
        // Check for end of headers (\r\n\r\n)
        if let Some(pos) = response_buffer.windows(4).position(|w| w == b"\r\n\r\n") {
          // Parse headers
          let headers_str = String::from_utf8_lossy(&response_buffer[..pos + 4]);
          for line in headers_str.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.starts_with("content-length:") {
              if let Some(len_str) = line.split(':').nth(1) {
                if let Ok(len) = len_str.trim().parse::<usize>() {
                  content_length = Some(len);
                }
              }
            } else if line_lower.starts_with("transfer-encoding:") && line_lower.contains("chunked")
            {
              is_chunked = true;
            }
          }
          // Read body if Content-Length is specified and we don't have it all
          if let Some(cl) = content_length {
            let body_start = pos + 4;
            let body_received = response_buffer.len() - body_start;
            if body_received < cl {
              // Read remaining body (but don't use read_exact as connection might close)
              let remaining = cl - body_received;
              let mut read_so_far = 0;
              while read_so_far < remaining {
                if response_buffer.len() >= MAX_HTTP_RESPONSE_BUFFER {
                  log::warn!(
                    "HTTP response body exceeded {} bytes; refusing to forward a truncated response",
                    MAX_HTTP_RESPONSE_BUFFER
                  );
                  truncated = true;
                  break;
                }
                match stream.read(&mut temp_buf).await {
                  Ok(0) => break, // Connection closed
                  Ok(m) => {
                    let to_read = (remaining - read_so_far).min(m);
                    response_buffer.extend_from_slice(&temp_buf[..to_read]);
                    read_so_far += to_read;
                    if to_read < m {
                      // More data than needed, might be next response - stop here
                      break;
                    }
                  }
                  Err(_) => break,
                }
              }
            }
          } else if is_chunked {
            // A chunked body has no Content-Length, so the framing itself says
            // where it ends. Walk it as the bytes arrive, and de-frame it here:
            // the chunk-size lines are not body bytes, and forwarding them left
            // the browser rendering the framing.
            let body_start = pos + 4;
            let mut cursor = 0;
            let mut decoded = Vec::new();
            let state = loop {
              match decode_chunked(&response_buffer[body_start..], &mut cursor, &mut decoded) {
                ChunkedState::Incomplete => {}
                terminal => break terminal,
              }
              if response_buffer.len() >= MAX_HTTP_RESPONSE_BUFFER {
                log::warn!(
                  "Chunked HTTP response exceeded {} bytes; refusing to forward a truncated response",
                  MAX_HTTP_RESPONSE_BUFFER
                );
                truncated = true;
                break ChunkedState::Incomplete;
              }
              match stream.read(&mut temp_buf).await {
                Ok(0) => break ChunkedState::Incomplete,
                Ok(n) => response_buffer.extend_from_slice(&temp_buf[..n]),
                Err(_) => break ChunkedState::Incomplete,
              }
            };
            body = match state {
              ChunkedState::Complete => BufferedBody::Dechunked(decoded),
              _ => BufferedBody::BrokenChunks,
            };
          } else {
            // No Content-Length and not chunked - read until connection closes
            // But limit to reasonable size to avoid memory issues
            loop {
              if response_buffer.len() >= MAX_HTTP_RESPONSE_BUFFER {
                log::warn!(
                  "HTTP response exceeded {} bytes; refusing to forward a truncated response",
                  MAX_HTTP_RESPONSE_BUFFER
                );
                truncated = true;
                break;
              }
              match stream.read(&mut temp_buf).await {
                Ok(0) => break, // Connection closed
                Ok(n) => {
                  response_buffer.extend_from_slice(&temp_buf[..n]);
                }
                Err(_) => break,
              }
            }
          }
          break;
        }
      }
      Err(e) => {
        log::error!("Error reading HTTP response: {}", e);
        break;
      }
    }
  }

  BufferedHttpResponse {
    bytes: response_buffer,
    body,
    truncated,
  }
}

async fn handle_http_via_socks4(
  req: Request<hyper::body::Incoming>,
  upstream_url: &str,
) -> Result<Response<Full<Bytes>>, Infallible> {
  // Extract domain for traffic tracking
  let domain = req
    .uri()
    .host()
    .map(|h| h.to_string())
    .unwrap_or_else(|| "unknown".to_string());

  // Parse upstream SOCKS4 proxy URL
  let upstream = match Url::parse(upstream_url) {
    Ok(url) => url,
    Err(e) => {
      log::error!("Failed to parse SOCKS4 proxy URL: {}", e);
      let mut response = Response::new(Full::new(Bytes::from("Invalid proxy URL")));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(response);
    }
  };

  let socks_host = upstream.host_str().unwrap_or("127.0.0.1");
  let socks_port = upstream.port().unwrap_or(1080);
  let socks_addr = format!("{}:{}", socks_host, socks_port);

  // Parse target from request URI
  let target_uri = req.uri();
  let target_host = target_uri.host().unwrap_or("localhost");
  let target_port = target_uri.port_u16().unwrap_or(80);

  // Connect to SOCKS4 proxy
  let mut socks_stream =
    match tokio::time::timeout(UPSTREAM_DIAL_TIMEOUT, TcpStream::connect(&socks_addr)).await {
      Ok(Ok(stream)) => stream,
      Ok(Err(e)) => {
        log::error!("Failed to connect to SOCKS4 proxy {}: {}", socks_addr, e);
        let mut response = Response::new(Full::new(Bytes::from(format!(
          "Failed to connect to SOCKS4 proxy: {}",
          e
        ))));
        *response.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(response);
      }
      Err(_) => {
        log::error!("Connect to SOCKS4 proxy {} timed out", socks_addr);
        let mut response =
          Response::new(Full::new(Bytes::from("Connect to SOCKS4 proxy timed out")));
        *response.status_mut() = StatusCode::GATEWAY_TIMEOUT;
        return Ok(response);
      }
    };

  // Build a SOCKS4a CONNECT request. We deliberately do NOT resolve the target
  // hostname locally: tokio::net::lookup_host would call the HOST resolver
  // (getaddrinfo), leaking the destination domain to the host's DNS server and
  // defeating the per-profile proxy. SOCKS4a has the PROXY resolve the name —
  // send the sentinel IP 0.0.0.x (x != 0), then the NULL-terminated userid, then
  // the NULL-terminated hostname. (Most SOCKS4 proxies support 4a; a legacy
  // SOCKS4-only proxy without remote DNS cannot be used leak-free for plaintext
  // HTTP — prefer SOCKS5 there.)
  let mut socks_request = vec![0x04, 0x01]; // SOCKS4, CONNECT
  socks_request.extend_from_slice(&target_port.to_be_bytes());
  socks_request.extend_from_slice(&[0, 0, 0, 1]); // 0.0.0.1 => SOCKS4a remote-DNS marker
  socks_request.push(0); // empty userid, NULL-terminated
  socks_request.extend_from_slice(target_host.as_bytes()); // hostname for the proxy to resolve
  socks_request.push(0); // NULL-terminated hostname

  // Send SOCKS4 CONNECT request
  if let Err(e) = socks_stream.write_all(&socks_request).await {
    log::error!("Failed to send SOCKS4 CONNECT request: {}", e);
    let mut response = Response::new(Full::new(Bytes::from(format!(
      "Failed to send SOCKS4 request: {}",
      e
    ))));
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    return Ok(response);
  }

  // Read SOCKS4 response
  let mut socks_response = [0u8; 8];
  match tokio::time::timeout(
    UPSTREAM_DIAL_TIMEOUT,
    socks_stream.read_exact(&mut socks_response),
  )
  .await
  {
    Ok(Ok(_)) => {}
    Ok(Err(e)) => {
      log::error!("Failed to read SOCKS4 response: {}", e);
      let mut response = Response::new(Full::new(Bytes::from(format!(
        "Failed to read SOCKS4 response: {}",
        e
      ))));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(response);
    }
    Err(_) => {
      log::error!("SOCKS4 handshake response timed out");
      let mut response = Response::new(Full::new(Bytes::from(
        "SOCKS4 handshake response timed out",
      )));
      *response.status_mut() = StatusCode::GATEWAY_TIMEOUT;
      return Ok(response);
    }
  }

  // Check SOCKS4 response (second byte should be 0x5A for success)
  if socks_response[1] != 0x5A {
    log::error!(
      "SOCKS4 connection failed, response code: {}",
      socks_response[1]
    );
    let mut response = Response::new(Full::new(Bytes::from("SOCKS4 connection failed")));
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    return Ok(response);
  }

  // Now send the HTTP request through the SOCKS4 connection
  // Build HTTP request line
  let method = req.method().as_str();
  let path = target_uri
    .path_and_query()
    .map(|pq| pq.as_str())
    .unwrap_or("/");
  let http_version = if req.version() == hyper::Version::HTTP_11 {
    "HTTP/1.1"
  } else {
    "HTTP/1.0"
  };

  let mut http_request = format!("{} {} {}\r\n", method, path, http_version);

  // Add Host header if not present
  let mut has_host = false;
  for (name, value) in req.headers().iter() {
    if name.as_str().eq_ignore_ascii_case("host") {
      has_host = true;
    }
    // Skip proxy-specific headers
    if name.as_str().eq_ignore_ascii_case("proxy-authorization")
      || name.as_str().eq_ignore_ascii_case("proxy-connection")
      || name.as_str().eq_ignore_ascii_case("proxy-authenticate")
    {
      continue;
    }
    // Skip Content-Length and Transfer-Encoding - we'll add our own Content-Length
    // based on the collected body size. Having both violates HTTP/1.1 (RFC 7230).
    if name.as_str().eq_ignore_ascii_case("content-length")
      || name.as_str().eq_ignore_ascii_case("transfer-encoding")
    {
      continue;
    }
    if let Ok(val) = value.to_str() {
      http_request.push_str(&format!("{}: {}\r\n", name.as_str(), val));
    }
  }

  if !has_host {
    http_request.push_str(&format!("Host: {}:{}\r\n", target_host, target_port));
  }

  // Get body
  let body_bytes = match req.collect().await {
    Ok(collected) => collected.to_bytes(),
    Err(_) => Bytes::new(),
  };

  // Add Content-Length if there's a body
  if !body_bytes.is_empty() {
    http_request.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
  }

  http_request.push_str("\r\n");

  // Send HTTP request
  if let Err(e) = socks_stream.write_all(http_request.as_bytes()).await {
    log::error!("Failed to send HTTP request through SOCKS4: {}", e);
    let mut response = Response::new(Full::new(Bytes::from(format!(
      "Failed to send HTTP request: {}",
      e
    ))));
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    return Ok(response);
  }

  // Send body if present
  if !body_bytes.is_empty() {
    if let Err(e) = socks_stream.write_all(&body_bytes).await {
      log::error!("Failed to send HTTP body through SOCKS4: {}", e);
      let mut response = Response::new(Full::new(Bytes::from(format!(
        "Failed to send HTTP body: {}",
        e
      ))));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(response);
    }
  }

  // Read HTTP response, bounded in both size and time so a stalled or
  // never-terminating upstream cannot pin this task (and its connection
  // permit) forever.
  let buffered = match tokio::time::timeout(
    PLAIN_HTTP_EXCHANGE_TIMEOUT,
    read_http_response_buffer(&mut socks_stream),
  )
  .await
  {
    Ok(buffer) => buffer,
    Err(_) => {
      log::error!("HTTP response via SOCKS4 timed out");
      let mut response = Response::new(Full::new(Bytes::from("Upstream response timed out")));
      *response.status_mut() = StatusCode::GATEWAY_TIMEOUT;
      return Ok(response);
    }
  };

  // A capped read holds only a prefix of the body. Forwarding it would hand the
  // browser a complete-looking short response, so fail the request instead.
  if buffered.truncated {
    log::error!(
      "HTTP response via SOCKS4 for {domain} exceeded the buffer cap; refusing to forward a truncated body"
    );
    let mut response = Response::new(Full::new(Bytes::from(
      "Upstream response too large to buffer",
    )));
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    return Ok(response);
  }
  let BufferedHttpResponse {
    bytes: response_buffer,
    body: buffered_body,
    ..
  } = buffered;

  // Parse HTTP response
  let response_str = String::from_utf8_lossy(&response_buffer);
  let mut lines = response_str.lines();
  let status_line = lines.next().unwrap_or("HTTP/1.1 500 Internal Server Error");
  let status_parts: Vec<&str> = status_line.split_whitespace().collect();
  let status_code = status_parts
    .get(1)
    .and_then(|s| s.parse::<u16>().ok())
    .unwrap_or(500);

  // Find header/body boundary
  let header_end = response_buffer
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .map(|p| p + 4)
    .unwrap_or(response_buffer.len());

  let body = match buffered_body {
    BufferedBody::AsSent => response_buffer[header_end..].to_vec(),
    BufferedBody::Dechunked(body) => body,
    BufferedBody::BrokenChunks => {
      log::error!("Chunked HTTP response via SOCKS4 for {domain} did not decode");
      let mut response = Response::new(Full::new(Bytes::from("Malformed upstream response")));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(response);
    }
  };

  // Record request in traffic tracker
  let response_size = body.len() as u64;
  if let Some(tracker) = get_traffic_tracker() {
    tracker.record_request(&domain, body_bytes.len() as u64, response_size);
  }

  let mut hyper_response = Response::new(Full::new(Bytes::from(body)));
  // A status line carrying something outside 100..=999 must not panic the
  // connection task.
  *hyper_response.status_mut() =
    StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_GATEWAY);
  forward_upstream_headers(&mut hyper_response, &response_buffer[..header_end]);

  Ok(hyper_response)
}

/// Handle plain HTTP requests through a Shadowsocks upstream.
/// reqwest doesn't support SS natively, so we connect through the SS tunnel
/// manually and forward the HTTP request/response.
async fn handle_http_via_shadowsocks(
  req: Request<hyper::body::Incoming>,
  upstream: &Url,
) -> Result<Response<Full<Bytes>>, Infallible> {
  let domain = req
    .uri()
    .host()
    .map(|h| h.to_string())
    .unwrap_or_else(|| "unknown".to_string());
  let port = req.uri().port_u16().unwrap_or(80);

  let ss_host = upstream.host_str().unwrap_or("127.0.0.1");
  let ss_port = upstream.port().unwrap_or(8388);
  let method_str = urlencoding::decode(upstream.username())
    .unwrap_or_default()
    .to_string();
  let password = urlencoding::decode(upstream.password().unwrap_or(""))
    .unwrap_or_default()
    .to_string();

  let cipher = match method_str.parse::<shadowsocks::crypto::CipherKind>() {
    Ok(c) => c,
    Err(_) => {
      let mut resp = Response::new(Full::new(Bytes::from(format!(
        "Bad SS cipher: {method_str}"
      ))));
      *resp.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(resp);
    }
  };

  let context = shadowsocks::context::Context::new_shared(shadowsocks::config::ServerType::Local);
  let svr_cfg = match shadowsocks::config::ServerConfig::new(
    shadowsocks::config::ServerAddr::from((ss_host.to_string(), ss_port)),
    &password,
    cipher,
  ) {
    Ok(c) => c,
    Err(e) => {
      let mut resp = Response::new(Full::new(Bytes::from(format!("SS config error: {e}"))));
      *resp.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(resp);
    }
  };

  let target_addr = shadowsocks::relay::Address::DomainNameAddress(domain.clone(), port);

  let mut stream = match shadowsocks::relay::tcprelay::proxy_stream::ProxyClientStream::connect(
    context,
    &svr_cfg,
    target_addr,
  )
  .await
  {
    Ok(s) => s,
    Err(e) => {
      let mut resp = Response::new(Full::new(Bytes::from(format!("SS connect: {e}"))));
      *resp.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(resp);
    }
  };

  // Build and send the HTTP request through the SS tunnel
  let path = req
    .uri()
    .path_and_query()
    .map(|pq| pq.as_str())
    .unwrap_or("/");
  let method = req.method().as_str();
  let mut raw_req = format!("{method} {path} HTTP/1.1\r\nHost: {domain}\r\nConnection: close\r\n");
  for (name, value) in req.headers() {
    if name != "host" && name != "connection" {
      raw_req.push_str(&format!("{}: {}\r\n", name, value.to_str().unwrap_or("")));
    }
  }
  raw_req.push_str("\r\n");

  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  if let Err(e) = stream.write_all(raw_req.as_bytes()).await {
    let mut resp = Response::new(Full::new(Bytes::from(format!("SS write: {e}"))));
    *resp.status_mut() = StatusCode::BAD_GATEWAY;
    return Ok(resp);
  }

  let mut response_buf = Vec::new();
  if let Err(e) = stream.read_to_end(&mut response_buf).await {
    log::warn!("SS read error (may be partial): {e}");
  }

  if let Some(tracker) = get_traffic_tracker() {
    tracker.record_request(&domain, raw_req.len() as u64, response_buf.len() as u64);
  }

  // Parse the raw HTTP response. The boundary is found in the raw bytes, not in
  // a lossy UTF-8 copy of them, so a body byte that is not valid UTF-8 cannot
  // shift the offset the body is sliced at.
  let header_end = response_buf
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .map(|p| p + 4)
    .unwrap_or(response_buf.len());
  let header_block = &response_buf[..header_end];
  let header_text = String::from_utf8_lossy(header_block);
  let status_line = header_text
    .lines()
    .next()
    .unwrap_or("HTTP/1.1 502 Bad Gateway");
  let status_code: u16 = status_line
    .split_whitespace()
    .nth(1)
    .and_then(|s| s.parse().ok())
    .unwrap_or(502);

  let raw_body = &response_buf[header_end..];
  let body = if declares_chunked(header_block) {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    match decode_chunked(raw_body, &mut cursor, &mut decoded) {
      ChunkedState::Complete => decoded,
      _ => {
        log::error!("Chunked HTTP response via Shadowsocks for {domain} did not decode");
        let mut resp = Response::new(Full::new(Bytes::from("Malformed upstream response")));
        *resp.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(resp);
      }
    }
  } else {
    raw_body.to_vec()
  };

  let mut hyper_response = Response::new(Full::new(Bytes::from(body)));
  *hyper_response.status_mut() =
    StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_GATEWAY);
  forward_upstream_headers(&mut hyper_response, header_block);

  Ok(hyper_response)
}

async fn handle_http(
  req: Request<hyper::body::Incoming>,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) -> Result<Response<Full<Bytes>>, Infallible> {
  // Extract domain for traffic tracking
  let domain = req
    .uri()
    .host()
    .map(|h| h.to_string())
    .unwrap_or_else(|| "unknown".to_string());

  // Block if domain is in the DNS blocklist (before any connection)
  if blocklist_matcher.is_blocked(&domain) {
    log::debug!("[blocklist] Blocked HTTP request to {}", domain);
    let mut response = Response::new(Full::new(Bytes::from("Blocked by DNS blocklist")));
    *response.status_mut() = StatusCode::FORBIDDEN;
    return Ok(response);
  }

  log::trace!(
    "Handling HTTP request: {} {} (host: {:?})",
    req.method(),
    req.uri(),
    req.uri().host()
  );

  let should_bypass = bypass_matcher.should_bypass(&domain);

  // Handle proxy types that reqwest doesn't support natively
  if !should_bypass {
    if let Some(ref upstream) = upstream_url {
      if upstream != "DIRECT" {
        if let Ok(url) = Url::parse(upstream) {
          match url.scheme() {
            "socks4" => {
              return handle_http_via_socks4(req, upstream).await;
            }
            "ss" | "shadowsocks" => {
              return handle_http_via_shadowsocks(req, &url).await;
            }
            _ => {}
          }
        }
      }
    }
  }

  // Use reqwest for HTTP/HTTPS/SOCKS5 proxies
  let client = if should_bypass {
    direct_http_client()
  } else if let Some(ref upstream) = upstream_url {
    if upstream == "DIRECT" {
      direct_http_client()
    } else {
      match proxied_http_client(upstream) {
        Ok(c) => c,
        Err(e) => {
          log::error!("Failed to create proxy client: {}", e);
          let mut response = Response::new(Full::new(Bytes::from(format!(
            "Proxy configuration error: {}",
            e
          ))));
          *response.status_mut() = StatusCode::BAD_GATEWAY;
          return Ok(response);
        }
      }
    }
  } else {
    direct_http_client()
  };

  // Convert hyper request to reqwest request
  let uri = req.uri().to_string();
  let method = req.method().clone();
  let headers = req.headers().clone();

  let mut request_builder = match method.as_str() {
    "GET" => client.get(&uri),
    "POST" => client.post(&uri),
    "PUT" => client.put(&uri),
    "DELETE" => client.delete(&uri),
    "PATCH" => client.patch(&uri),
    "HEAD" => client.head(&uri),
    _ => {
      let mut response = Response::new(Full::new(Bytes::from("Unsupported method")));
      *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
      return Ok(response);
    }
  };

  // Copy headers, but skip proxy-specific headers that shouldn't be forwarded
  for (name, value) in headers.iter() {
    // Skip proxy-specific headers - these are for the local proxy, not the upstream
    if name.as_str().eq_ignore_ascii_case("proxy-authorization")
      || name.as_str().eq_ignore_ascii_case("proxy-connection")
      || name.as_str().eq_ignore_ascii_case("proxy-authenticate")
    {
      continue;
    }
    if let Ok(val) = value.to_str() {
      request_builder = request_builder.header(name.as_str(), val);
    }
  }

  // Get body
  let body_bytes = match req.collect().await {
    Ok(collected) => collected.to_bytes(),
    Err(_) => Bytes::new(),
  };

  if !body_bytes.is_empty() {
    request_builder = request_builder.body(body_bytes.to_vec());
  }

  // Execute request
  match request_builder.send().await {
    Ok(response) => {
      let status = response.status();
      let headers = response.headers().clone();
      // Never swallow a body error into an empty body: the status and headers
      // are already captured, so an empty `Full` would be forwarded as a
      // well-formed short 200 that the browser cannot distinguish from a real
      // one (hyper drops the mismatched Content-Length and writes 0).
      let body = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
          log::warn!("Failed to read response body from {domain}: {e}");
          let mut error_response =
            Response::new(Full::new(Bytes::from(format!("Response body failed: {e}"))));
          *error_response.status_mut() = StatusCode::BAD_GATEWAY;
          return Ok(error_response);
        }
      };

      // Record request in traffic tracker
      let response_size = body.len() as u64;
      if let Some(tracker) = get_traffic_tracker() {
        tracker.record_request(&domain, body_bytes.len() as u64, response_size);
      }

      let mut hyper_response = Response::new(Full::new(body));
      *hyper_response.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap();

      // Copy response headers
      for (name, value) in headers.iter() {
        if let Ok(val) = value.to_str() {
          hyper_response
            .headers_mut()
            .insert(name, val.parse().unwrap());
        }
      }

      Ok(hyper_response)
    }
    Err(e) => {
      log::error!("Request failed: {}", e);
      let mut response = Response::new(Full::new(Bytes::from(format!("Request failed: {}", e))));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      Ok(response)
    }
  }
}

/// Shared reqwest client for direct (no-upstream / bypass) plain-HTTP
/// forwarding. reqwest clients hold a connection pool, TLS config and
/// resolver state — building one per request would redo full TCP+TLS setup
/// every time and never reuse upstream connections.
fn direct_http_client() -> reqwest::Client {
  static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
  CLIENT
    .get_or_init(|| {
      reqwest::Client::builder()
        .connect_timeout(UPSTREAM_DIAL_TIMEOUT)
        .read_timeout(PLAIN_HTTP_EXCHANGE_TIMEOUT)
        .build()
        .unwrap_or_default()
    })
    .clone()
}

/// Shared per-upstream reqwest clients. A worker serves exactly one upstream,
/// so this normally holds a single entry.
fn proxied_http_client(upstream_url: &str) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
  static CLIENTS: OnceLock<Mutex<HashMap<String, reqwest::Client>>> = OnceLock::new();
  let map = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
  let mut guard = map.lock().unwrap();
  if let Some(client) = guard.get(upstream_url) {
    return Ok(client.clone());
  }
  let client = build_reqwest_client_with_proxy(upstream_url)?;
  guard.insert(upstream_url.to_string(), client.clone());
  Ok(client)
}

fn build_reqwest_client_with_proxy(
  upstream_url: &str,
) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
  use reqwest::Proxy;

  let client_builder = reqwest::Client::builder()
    .connect_timeout(UPSTREAM_DIAL_TIMEOUT)
    .read_timeout(PLAIN_HTTP_EXCHANGE_TIMEOUT);

  // Parse the upstream URL
  let url = Url::parse(upstream_url)?;
  let scheme = url.scheme();

  let proxy = match scheme {
    "http" | "https" => {
      // Both are a plaintext hop to the proxy. `https` is only a provider
      // label here; the tunnel path treats it identically to `http`.
      Proxy::http(upstream_url)?
    }
    "httpstls" => {
      // TLS to the proxy. reqwest spells that `https://`, which is what
      // `reqwest_upstream_url` produces; the scheme rewrite is the whole
      // difference, the endpoint and credentials are unchanged.
      Proxy::http(crate::proxy_storage::reqwest_upstream_url(upstream_url))?
    }
    "socks5" => {
      // Force REMOTE (proxy-side) DNS for plaintext HTTP over a SOCKS5
      // upstream. reqwest maps the bare `socks5` scheme to DnsResolve::Local,
      // which resolves the destination hostname on the HOST (getaddrinfo) BEFORE
      // connecting — leaking the destination domain to the host's DNS resolver
      // and defeating the per-profile proxy. The `socks5h` scheme maps to
      // DnsResolve::Proxy, so the proxy resolves the hostname and nothing leaks.
      // (The CONNECT/HTTPS path already does remote DNS via connect_via_socks's
      // AddrKind::Domain.)
      let remote_dns_url = match upstream_url.strip_prefix("socks5://") {
        Some(rest) => format!("socks5h://{rest}"),
        None => upstream_url.to_string(),
      };
      Proxy::all(remote_dns_url)?
    }
    "socks4" => {
      // SOCKS4 is handled manually in handle_http_via_socks4
      // This should not be reached, but return error as fallback
      return Err("SOCKS4 should be handled manually".into());
    }
    _ => {
      return Err(format!("Unsupported proxy scheme: {}", scheme).into());
    }
  };

  Ok(client_builder.proxy(proxy).build()?)
}

/// Handle a single proxy connection (used by both the proxy worker and in-process proxy checks).
pub async fn handle_proxy_connection(
  mut stream: tokio::net::TcpStream,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) {
  let _ = stream.set_nodelay(true);

  if stream.readable().await.is_err() {
    return;
  }

  // Classify the connection by its request line. One read is not enough: TCP
  // may deliver fewer than the 7 bytes needed to recognise "CONNECT", and a
  // misclassified CONNECT goes to hyper, which refuses it with 501 rather than
  // tunneling it. Accumulate until the verb is decidable.
  let mut peek_buffer = [0u8; 16];
  let mut peeked = 0usize;
  const CONNECT_VERB_LEN: usize = 7;
  loop {
    match stream.read(&mut peek_buffer[peeked..]).await {
      Ok(0) => break,
      Ok(m) => {
        peeked += m;
        if peeked >= CONNECT_VERB_LEN {
          break;
        }
      }
      Err(_) => return,
    }
  }

  match peeked {
    0 => {}
    n => {
      let request_start_upper =
        String::from_utf8_lossy(&peek_buffer[..n.min(CONNECT_VERB_LEN)]).to_uppercase();
      let is_connect = request_start_upper.starts_with("CONNECT");

      if is_connect {
        let mut full_request = Vec::with_capacity(4096);
        full_request.extend_from_slice(&peek_buffer[..n]);

        let mut remaining = [0u8; 4096];
        let mut total_read = n;
        let max_reads = 100;
        let mut reads = 0;

        loop {
          if reads >= max_reads {
            break;
          }
          match stream.read(&mut remaining).await {
            Ok(0) => {
              if full_request.ends_with(b"\r\n\r\n")
                || full_request.ends_with(b"\n\n")
                || total_read > 0
              {
                break;
              }
              return;
            }
            Ok(m) => {
              reads += 1;
              total_read += m;
              full_request.extend_from_slice(&remaining[..m]);
              if full_request.ends_with(b"\r\n\r\n") || full_request.ends_with(b"\n\n") {
                break;
              }
            }
            Err(_) => {
              if total_read > 0 {
                break;
              }
              return;
            }
          }
        }

        if let Err(e) = handle_connect_from_buffer(
          stream,
          full_request,
          upstream_url,
          bypass_matcher,
          blocklist_matcher,
        )
        .await
        {
          let msg = e.to_string();
          if let Some(suppressed) = log_throttle(&msg) {
            if suppressed > 0 {
              log::warn!(
                "CONNECT tunnel ended with error: {msg} ({suppressed} more suppressed in last 30s)"
              );
            } else {
              log::warn!("CONNECT tunnel ended with error: {msg}");
            }
          }
        }
        return;
      }

      // Non-CONNECT: prepend consumed bytes and pass to hyper
      let prepended_bytes = peek_buffer[..n].to_vec();
      let prepended_reader = PrependReader {
        prepended: prepended_bytes,
        prepended_pos: 0,
        inner: stream,
      };
      let io = TokioIo::new(prepended_reader);
      let service = service_fn(move |req| {
        handle_request(
          req,
          upstream_url.clone(),
          bypass_matcher.clone(),
          blocklist_matcher.clone(),
        )
      });

      let _ = http1::Builder::new().serve_connection(io, service).await;
    }
  }
}

/// Render an upstream proxy URL for logging with any embedded credentials
/// stripped. `config.upstream_url` carries `scheme://user:pass@host:port`, and
/// diagnostic logs and command responses must never expose the userinfo.
pub fn redacted_upstream(upstream: &str) -> String {
  if upstream.is_empty() {
    return "none".to_string();
  }
  match Url::parse(upstream) {
    Ok(u) => match (u.host_str(), u.port()) {
      (Some(host), Some(port)) => format!("{}://{host}:{port}", u.scheme()),
      (Some(host), None) => format!("{}://{host}", u.scheme()),
      _ => "<redacted>".to_string(),
    },
    Err(_) => "<redacted>".to_string(),
  }
}

pub async fn run_proxy_server(config: ProxyConfig) -> Result<(), Box<dyn std::error::Error>> {
  log::info!(
    "Proxy worker starting, looking for config id: {}",
    config.id
  );

  // Load the config from disk to get the latest state
  let config = match crate::proxy_storage::get_proxy_config(&config.id) {
    Some(c) => c,
    None => {
      log::error!("Config not found for id: {}", config.id);
      return Err("Config not found".into());
    }
  };

  log::info!(
    "Found config: id={}, port={:?}, upstream={}, profile_id={:?}",
    config.id,
    config.local_port,
    redacted_upstream(&config.upstream_url),
    config.profile_id
  );

  // Initialize traffic tracker with profile ID if available.
  // This can be called multiple times to update the tracker.
  init_traffic_tracker(config.id.clone(), config.profile_id.clone());

  // Determine the bind address
  let bind_addr = SocketAddr::from(([127, 0, 0, 1], config.local_port.unwrap_or(0)));

  log::info!("Attempting to bind proxy server to {}", bind_addr);

  // Bind to the port. Use SO_REUSEADDR so that a freshly-restarted worker
  // can bind a port that the previous worker left in TIME_WAIT, and retry
  // briefly to absorb transient races with the OS releasing the socket.
  let listener = {
    let mut attempts: u32 = 0;
    loop {
      let socket = tokio::net::TcpSocket::new_v4()?;
      let _ = socket.set_reuseaddr(true);
      match socket.bind(bind_addr) {
        Ok(()) => match socket.listen(1024) {
          Ok(l) => break l,
          Err(e) if attempts < 5 => {
            attempts += 1;
            let delay = std::time::Duration::from_millis(200 * u64::from(attempts));
            log::warn!(
              "listen() on {} failed (attempt {}/5): {}, retrying in {}ms",
              bind_addr,
              attempts,
              e,
              delay.as_millis()
            );
            tokio::time::sleep(delay).await;
          }
          Err(e) => {
            return Err(format!("Failed to listen on {bind_addr} after 5 attempts: {e}").into())
          }
        },
        Err(e) if attempts < 5 => {
          attempts += 1;
          let delay = std::time::Duration::from_millis(200 * u64::from(attempts));
          log::warn!(
            "bind() on {} failed (attempt {}/5): {}, retrying in {}ms",
            bind_addr,
            attempts,
            e,
            delay.as_millis()
          );
          tokio::time::sleep(delay).await;
        }
        Err(e) => return Err(format!("Failed to bind {bind_addr} after 5 attempts: {e}").into()),
      }
    }
  };
  let actual_port = listener.local_addr()?.port();

  log::info!("Successfully bound to port {}", actual_port);

  // Protocol served to the browser: "socks5" (Wayfern) or "http" (default).
  let local_protocol = config.local_protocol_or_default();
  let serve_socks5 = local_protocol == "socks5";

  // Update config with actual port and local_url (scheme matches the protocol
  // we serve, so the parent's readiness check and any consumer see the truth)
  let mut updated_config = config.clone();
  updated_config.local_port = Some(actual_port);
  updated_config.local_url = Some(format!(
    "{}://127.0.0.1:{}",
    if serve_socks5 { "socks5" } else { "http" },
    actual_port
  ));

  if !crate::proxy_storage::update_proxy_config(&updated_config) {
    log::error!("Failed to update proxy config");
    return Err("Failed to update proxy config".into());
  }

  let upstream_url = if updated_config.upstream_url == "DIRECT" {
    None
  } else {
    Some(updated_config.upstream_url.clone())
  };

  log::info!(
    "Proxy server listening on 127.0.0.1:{} (ready to accept connections)",
    actual_port
  );
  log::info!("Proxy server entering accept loop - process should stay alive");

  // Start a background task to write lightweight session snapshots for real-time updates
  // These are much smaller than full stats and can be written frequently (~100 bytes every 2 seconds)
  if let Some(tracker) = get_traffic_tracker() {
    let tracker_clone = tracker.clone();
    tokio::spawn(async move {
      let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
      interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
      // The snapshot content is derived entirely from these counters, so an
      // unchanged tuple means the on-disk session file is already current —
      // skip the write instead of rewriting identical bytes every 2s.
      let mut last_written: Option<(u64, u64, u64)> = None;

      loop {
        interval.tick().await;
        let snapshot = tracker_clone.get_snapshot();
        if last_written == Some(snapshot) {
          continue;
        }
        // Write lightweight session snapshot (only current counters, ~100 bytes)
        match tracker_clone.write_session_snapshot() {
          Ok(()) => last_written = Some(snapshot),
          Err(e) => log::debug!("Failed to write session snapshot: {}", e),
        }
      }
    });
  }

  // Start a background task to periodically flush traffic stats to disk
  // Use adaptive flush frequency: every 5 seconds when active, every 30 seconds when idle
  tokio::spawn(async move {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_activity_time = std::time::Instant::now();
    let mut last_flush_time = std::time::Instant::now();
    let mut current_interval_secs = 5u64;

    loop {
      interval.tick().await;
      // Catch panics so a poisoned lock or unexpected error inside
      // flush_to_disk doesn't abort the flush task and leave stats
      // unwritten for the lifetime of the worker. The captured state
      // is all Copy or atomic-assignment, so AssertUnwindSafe is sound.
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(tracker) = get_traffic_tracker() {
          let (sent, recv, requests) = tracker.get_snapshot();
          let current_bytes = sent + recv;
          let time_since_activity = last_activity_time.elapsed();
          let time_since_flush = last_flush_time.elapsed();
          let has_traffic = current_bytes > 0 || requests > 0;

          let desired_interval_secs =
            if has_traffic || time_since_activity < std::time::Duration::from_secs(30) {
              5u64
            } else {
              30u64
            };

          if desired_interval_secs != current_interval_secs {
            current_interval_secs = desired_interval_secs;
            interval =
              tokio::time::interval(tokio::time::Duration::from_secs(desired_interval_secs));
          }

          let flush_interval = std::time::Duration::from_secs(desired_interval_secs);
          let should_flush = time_since_flush >= flush_interval;

          if should_flush {
            match tracker.flush_to_disk() {
              Ok(Some((sent, recv))) => {
                last_flush_time = std::time::Instant::now();
                if sent > 0 || recv > 0 {
                  last_activity_time = std::time::Instant::now();
                }
              }
              Ok(None) => {
                last_flush_time = std::time::Instant::now();
              }
              Err(e) => {
                log::error!("Failed to flush traffic stats: {}", e);
              }
            }
          }
        }
      }));
      if let Err(panic) = result {
        log::error!("Panic caught in proxy traffic flush task; continuing: {panic:?}");
      }
    }
  });

  // Self-reaping supervisor. The worker is a detached process that outlives the
  // GUI, so it cannot rely on the GUI's in-memory death-monitor (which is lost
  // when the GUI restarts). Once the GUI records the browser PID and start time
  // this worker serves, poll that exact process identity and exit when it is
  // gone — never while it is alive. A 2-miss debounce avoids exiting on a
  // transient sysinfo false-negative under load / sleep-wake. The decision table
  // itself lives in `proxy_storage::supervisor_verdict` so every branch is unit
  // tested without spawning browsers.
  //
  // This runs on a DEDICATED OS THREAD, not a tokio task. If the worker's
  // accept/dial path ever busy-loops (e.g. a client retry-storm against a
  // failing upstream), it saturates the async runtime, and a tokio-based
  // supervisor would never be scheduled — leaving the worker spinning forever
  // even after its browser exits or its config is deleted (observed in the
  // field as pegged-CPU orphans that survive config deletion). A real thread
  // with a blocking sleep cannot be starved that way, so the worker always
  // reaps itself. Every call here is synchronous and safe off the runtime.
  {
    let watch_id = config.id.clone();
    let poll_interval = watchdog_poll_interval();
    std::thread::spawn(move || {
      use crate::proxy_storage::SupervisorVerdict;

      let mut consecutive_misses: u32 = 0;
      loop {
        std::thread::sleep(poll_interval);
        let cfg = crate::proxy_storage::get_proxy_config(&watch_id);
        let verdict = crate::proxy_storage::supervisor_verdict(
          cfg.as_ref(),
          crate::proxy_storage::proxy_config_age_secs(&watch_id),
          crate::proxy_storage::browser_owner_is_alive,
        );

        match verdict {
          SupervisorVerdict::Keep => consecutive_misses = 0,
          SupervisorVerdict::ExitOwnerGone => {
            consecutive_misses += 1;
            if consecutive_misses >= 2 {
              let owner = cfg.as_ref().and_then(|c| c.browser_pid).unwrap_or(0);
              log::info!("Browser PID {owner} for config {watch_id} is gone; worker exiting");
              crate::proxy_storage::delete_proxy_config(&watch_id);
              std::process::exit(0);
            }
          }
          SupervisorVerdict::ExitNeverClaimed => {
            log::info!(
              "Config {watch_id} was never claimed by a browser within the launch window; worker exiting"
            );
            crate::proxy_storage::delete_proxy_config(&watch_id);
            std::process::exit(0);
          }
          SupervisorVerdict::ExitConfigRemoved => {
            log::info!("Proxy config {watch_id} was removed; worker exiting");
            std::process::exit(0);
          }
        }
      }
    });
  }

  let bypass_matcher = BypassMatcher::new(&config.bypass_rules);
  let blocklist_matcher = if let Some(ref path) = config.blocklist_file {
    match BlocklistMatcher::from_file_with_mode(path, config.dns_allowlist_mode) {
      Ok(m) => m,
      Err(e) => {
        log::error!("[blocklist] Failed to load from {}: {}", path, e);
        BlocklistMatcher::new()
      }
    }
  } else {
    BlocklistMatcher::new()
  };

  // Bound concurrent connection handlers. A client retry-storm (e.g. a browser
  // hammering CONNECT requests while DNS is failing) must not spawn unbounded
  // tasks,
  // each of which parks a Tokio blocking thread inside getaddrinfo — that is
  // what exhausted the resolver pool and pegged the CPU on long-lived workers.
  // A real browser never approaches this ceiling; waiting for a permit
  // backpressures a storm instead of amplifying it.
  let conn_semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

  // Keep the runtime alive with an infinite loop
  // This ensures the process doesn't exit even if there are no active connections
  loop {
    match listener.accept().await {
      Ok((stream, _peer_addr)) => {
        // The semaphore is never closed, so acquire cannot fail.
        let permit = conn_semaphore
          .clone()
          .acquire_owned()
          .await
          .expect("connection semaphore is never closed");
        let upstream = upstream_url.clone();
        let matcher = bypass_matcher.clone();
        let blocker = blocklist_matcher.clone();
        if serve_socks5 {
          tokio::task::spawn(async move {
            let _permit = permit;
            crate::socks5_local::handle_socks5_connection(stream, upstream, matcher, blocker).await;
          });
        } else {
          tokio::task::spawn(async move {
            let _permit = permit;
            handle_proxy_connection(stream, upstream, matcher, blocker).await;
          });
        }
      }
      Err(e) => {
        log::error!("Error accepting connection: {:?}", e);
        // Continue accepting connections even if one fails
        // Add a small delay to avoid busy-waiting on errors
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
      }
    }
  }
}

async fn handle_connect_from_buffer(
  mut client_stream: TcpStream,
  request_buffer: Vec<u8>,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) -> Result<(), Box<dyn std::error::Error>> {
  // Parse the CONNECT request from the buffer
  let request_str = String::from_utf8_lossy(&request_buffer);
  let lines: Vec<&str> = request_str.lines().collect();

  if lines.is_empty() {
    let _ = client_stream
      .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
      .await;
    return Err("Empty CONNECT request".into());
  }

  // Parse CONNECT request: "CONNECT host:port HTTP/1.1"
  let parts: Vec<&str> = lines[0].split_whitespace().collect();
  if parts.len() < 2 || parts[0] != "CONNECT" {
    let _ = client_stream
      .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
      .await;
    return Err("Invalid CONNECT request".into());
  }

  let target = parts[1];
  let (target_host, target_port) = if let Some(colon_pos) = target.find(':') {
    let host = &target[..colon_pos];
    let port: u16 = target[colon_pos + 1..].parse().unwrap_or(443);
    (host, port)
  } else {
    (target, 443)
  };

  // Block if domain is in the DNS blocklist (before any connection)
  if blocklist_matcher.is_blocked(target_host) {
    log::debug!("[blocklist] Blocked CONNECT tunnel to {}", target_host);
    let _ = client_stream
      .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 24\r\n\r\nBlocked by DNS blocklist")
      .await;
    return Ok(());
  }

  // Record domain access in traffic tracker
  let domain = target_host.to_string();
  if let Some(tracker) = get_traffic_tracker() {
    tracker.record_request(&domain, 0, 0);
  }

  log::debug!(
    "CONNECT {}:{} (upstream={})",
    target_host,
    target_port,
    upstream_url
      .as_deref()
      .map(redacted_upstream)
      .unwrap_or_else(|| "DIRECT".to_string())
  );

  // Connect to target (directly or via upstream proxy).
  let target_stream = connect_to_target_via_upstream(
    target_host,
    target_port,
    upstream_url.as_deref(),
    &bypass_matcher,
  )
  .await?;

  // Send 200 Connection Established response to client
  // CRITICAL: Must flush after writing to ensure response is sent before tunneling
  client_stream
    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
    .await?;
  client_stream.flush().await?;

  log::trace!("Sent 200 Connection Established response, starting tunnel");

  tunnel_streams(client_stream, target_stream, domain).await;

  Ok(())
}

/// How often the self-reaping supervisor re-checks its owner. Overridable via
/// `DONUT_PROXY_WATCHDOG_INTERVAL_MS` so lifecycle tests can observe a real
/// worker reaping itself in seconds instead of a minute; the floor keeps a
/// mistyped value from turning the supervisor into a spin loop.
const WATCHDOG_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
const WATCHDOG_POLL_INTERVAL_FLOOR: std::time::Duration = std::time::Duration::from_millis(100);

fn watchdog_poll_interval() -> std::time::Duration {
  std::env::var("DONUT_PROXY_WATCHDOG_INTERVAL_MS")
    .ok()
    .and_then(|raw| raw.trim().parse::<u64>().ok())
    .map(std::time::Duration::from_millis)
    .map(|interval| interval.max(WATCHDOG_POLL_INTERVAL_FLOOR))
    .unwrap_or(WATCHDOG_POLL_INTERVAL)
}

/// Upper bound on concurrent connection handlers per worker. A real browser
/// never holds anywhere near this many simultaneous tunnels; the cap stops a
/// client retry-storm from spawning unbounded tasks (each of which parks a
/// Tokio blocking thread inside getaddrinfo).
const MAX_CONCURRENT_CONNECTIONS: usize = 512;

/// Connect timeout for the direct (no-upstream) dial path. Bounds a wedged
/// `getaddrinfo` so a broken resolver can't park a blocking thread for the
/// full OS timeout.
const DIRECT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Overall timeout for dialing an UPSTREAM proxy (TCP connect + CONNECT/SOCKS/SS
/// handshake). Without it, an upstream that accepts TCP but stalls before
/// replying hangs the worker task forever and holds a connection slot; under
/// load (e.g. two profiles sharing one proxy) the slots exhaust and the browser
/// sees `ERR_PROXY_CONNECTION_FAILED` until the profile is restarted. A
/// bounded dial fails fast and releases the slot.
pub(crate) const UPSTREAM_DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Cap on bytes buffered while waiting for the end of the HTTP response
/// headers on the manual plain-HTTP forwarding path.
const MAX_HTTP_HEADER_BUFFER: usize = 64 * 1024;

/// Cap on the total buffered HTTP response on the manual plain-HTTP
/// forwarding path.
const MAX_HTTP_RESPONSE_BUFFER: usize = 10 * 1024 * 1024;

/// Budget for a proxied plain-HTTP exchange on the manual (SOCKS4/Shadowsocks)
/// forwarding paths, which buffer the whole response themselves.
///
/// On the reqwest paths this is applied as a *read* timeout, not a total one:
/// it bounds the gap between successive reads, so a stalled upstream still
/// fails fast and releases its connection-semaphore permit, while a legitimately
/// slow transfer — a large download, an SSE stream, a long-poll — is not killed
/// mid-flight. `ClientBuilder::timeout` would cap the whole exchange including
/// the body and break all three.
const PLAIN_HTTP_EXCHANGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Per-host failure state (last failure instant, consecutive failure count) for
/// the direct dial path. Process-global — each worker is its own process.
fn direct_dial_failures() -> &'static Mutex<HashMap<String, (std::time::Instant, u32)>> {
  static M: OnceLock<Mutex<HashMap<String, (std::time::Instant, u32)>>> = OnceLock::new();
  M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// If `host` is inside its failure backoff window, return the remaining time so
/// the caller can short-circuit without a fresh getaddrinfo/connect. Never
/// mutates state, so the window always expires and the path self-heals once
/// DNS recovers.
fn direct_backoff_remaining(host: &str) -> Option<std::time::Duration> {
  let map = direct_dial_failures();
  let guard = map.lock().unwrap();
  let (last, fails) = guard.get(host).copied()?;
  // Exponential window capped at 30s: 2, 4, 8, 16, 30, 30, ...
  let window = std::time::Duration::from_secs((1u64 << fails.min(5)).min(30));
  let elapsed = last.elapsed();
  if elapsed < window {
    Some(window - elapsed)
  } else {
    None
  }
}

/// Record a direct-dial failure for `host`, growing its backoff window.
fn direct_backoff_record(host: &str) {
  let map = direct_dial_failures();
  let mut guard = map.lock().unwrap();
  // Bound memory against a page that emits many distinct failing hosts.
  if guard.len() > 2048 {
    guard.retain(|_, (last, _)| last.elapsed() < std::time::Duration::from_secs(60));
  }
  let entry = guard
    .entry(host.to_string())
    .or_insert_with(|| (std::time::Instant::now(), 0));
  entry.0 = std::time::Instant::now();
  entry.1 = entry.1.saturating_add(1);
}

/// Clear `host`'s failure state after a successful dial.
fn direct_backoff_clear(host: &str) {
  direct_dial_failures().lock().unwrap().remove(host);
}

/// Dial a target directly (no upstream) with a connect timeout and per-host
/// failure backoff. This is the server-side counterpart to the browser's
/// instant client-side retry: when a host's DNS/connect is failing (e.g. the
/// macOS resolver wedges after sleep/wake), repeated CONNECT requests
/// short-circuit
/// here instead of each spawning a fresh blocking getaddrinfo — which is what
/// let a retry-storm exhaust the blocking thread pool and peg the CPU.
async fn dial_direct(host: &str, port: u16) -> Result<TcpStream, Box<dyn std::error::Error>> {
  if let Some(remaining) = direct_backoff_remaining(host) {
    return Err(
      format!(
        "skipping direct dial to {host}: backing off ~{}s after repeated connect failures",
        remaining.as_secs().max(1)
      )
      .into(),
    );
  }
  match tokio::time::timeout(DIRECT_CONNECT_TIMEOUT, TcpStream::connect((host, port))).await {
    Ok(Ok(stream)) => {
      let _ = stream.set_nodelay(true);
      direct_backoff_clear(host);
      Ok(stream)
    }
    Ok(Err(e)) => {
      direct_backoff_record(host);
      Err(e.into())
    }
    Err(_) => {
      direct_backoff_record(host);
      Err(
        format!(
          "direct connect to {host}:{port} timed out after {}s",
          DIRECT_CONNECT_TIMEOUT.as_secs()
        )
        .into(),
      )
    }
  }
}

/// Rate-limit a repetitive log line keyed by `key`: returns `Some(suppressed)`
/// when the caller should emit (first time or after a 30s window, with the
/// count dropped since the last emit), or `None` to skip. Stops a connect/DNS
/// storm from writing the same WARN millions of times (the line that grew
/// worker logs to 100MB).
pub(crate) fn log_throttle(key: &str) -> Option<u64> {
  fn throttle_map() -> &'static Mutex<HashMap<String, (std::time::Instant, u64)>> {
    static M: OnceLock<Mutex<HashMap<String, (std::time::Instant, u64)>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
  }
  let map = throttle_map();
  let mut guard = map.lock().unwrap();
  if guard.len() > 2048 {
    guard.retain(|_, (last, _)| last.elapsed() < std::time::Duration::from_secs(60));
  }
  let now = std::time::Instant::now();
  match guard.get_mut(key) {
    Some((last, suppressed)) => {
      if now.duration_since(*last) >= std::time::Duration::from_secs(30) {
        let dropped = *suppressed;
        *last = now;
        *suppressed = 0;
        Some(dropped)
      } else {
        *suppressed += 1;
        None
      }
    }
    None => {
      guard.insert(key.to_string(), (now, 0));
      Some(0)
    }
  }
}

/// Read an upstream proxy's response to our CONNECT request.
///
/// TCP is a stream, not a sequence of messages, so a single `read` is wrong in
/// both directions: the status line can arrive split from the rest of the
/// headers (a lone `read` would reject a tunnel the upstream actually granted),
/// and the terminating CRLFCRLF can arrive with destination payload appended
/// (those bytes belong to the tunnel). Reads until the header terminator and
/// returns `(headers, bytes_after_headers)`.
async fn read_upstream_connect_response<S: AsyncRead + Unpin>(
  stream: &mut S,
) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
  let mut buffer = Vec::with_capacity(1024);
  let mut chunk = [0u8; 4096];
  // Only the terminator needs finding, so rescanning can resume from just
  // before the previous tail rather than restarting at 0 each read.
  let mut scanned = 0usize;

  loop {
    if buffer.len() > MAX_HTTP_HEADER_BUFFER {
      return Err("upstream proxy CONNECT response headers too large".into());
    }
    let n = tokio::time::timeout(UPSTREAM_DIAL_TIMEOUT, stream.read(&mut chunk))
      .await
      .map_err(|_| "upstream proxy CONNECT response timed out")??;
    if n == 0 {
      return Err("upstream proxy closed the connection during CONNECT".into());
    }
    buffer.extend_from_slice(&chunk[..n]);

    if let Some(pos) = buffer[scanned..]
      .windows(4)
      .position(|w| w == b"\r\n\r\n")
      .map(|p| p + scanned)
    {
      let header_end = pos + 4;
      let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
      return Ok((headers, buffer[header_end..].to_vec()));
    }
    scanned = buffer.len().saturating_sub(3);
  }
}

/// Perform the HTTP CONNECT handshake over an already-established hop to the
/// proxy and return the tunnelled stream.
///
/// Generic over the hop so the identical handshake runs on a bare `TcpStream`
/// (`http`/`https`) and on a `TlsStream<TcpStream>` (`httpstls`). This is the
/// only place `Proxy-Authorization` is written, so whether those credentials
/// cross the network in the clear is decided entirely by which stream the
/// caller hands in, nothing here can weaken it.
async fn connect_via_http_proxy<S: AsyncStream + 'static>(
  mut proxy_stream: S,
  proxy_host: &str,
  proxy_port: u16,
  target_host: &str,
  target_port: u16,
  upstream: &Url,
) -> Result<BoxedAsyncStream, Box<dyn std::error::Error>> {
  let mut connect_req = format!(
    "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n",
    target_host, target_port, target_host, target_port
  );

  let (username, password) = upstream_userpass(upstream);
  if !username.is_empty() {
    use base64::{engine::general_purpose, Engine as _};
    let auth = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth));
  }

  connect_req.push_str("\r\n");

  proxy_stream.write_all(connect_req.as_bytes()).await?;

  let (response_headers, coalesced) = read_upstream_connect_response(&mut proxy_stream).await?;
  let status_line = response_headers.lines().next().unwrap_or("").to_string();

  if !response_headers.starts_with("HTTP/1.1 200") && !response_headers.starts_with("HTTP/1.0 200")
  {
    log::warn!(
      "Upstream CONNECT to {}:{} via {}:{} rejected: {}",
      target_host,
      target_port,
      proxy_host,
      proxy_port,
      status_line
    );
    return Err(format!("Upstream proxy CONNECT failed: {status_line}").into());
  }

  log::info!(
    "Upstream CONNECT to {}:{} via {}:{} accepted ({})",
    target_host,
    target_port,
    proxy_host,
    proxy_port,
    status_line
  );

  if coalesced.is_empty() {
    Ok(Box::new(proxy_stream))
  } else {
    // The upstream packed the destination's first bytes into the same
    // segment as its 200. They are tunnel payload, not proxy protocol:
    // replay them ahead of the socket so the client sees an unbroken
    // stream. Server-speaks-first protocols (SMTP/IMAP/SSH banners)
    // reach this reliably.
    log::debug!(
      "Upstream CONNECT response coalesced {} byte(s) of payload; forwarding",
      coalesced.len()
    );
    Ok(Box::new(PrependReader {
      prepended: coalesced,
      prepended_pos: 0,
      inner: proxy_stream,
    }))
  }
}

/// Wrap an established TCP hop to the proxy in TLS, verifying the proxy's
/// certificate against `proxy_host`.
///
/// There is deliberately no opportunistic downgrade and no
/// `danger_accept_invalid_certs` escape hatch: a failed handshake is a failed
/// connection. Certificate verification is what makes this hop resistant to an
/// active man-in-the-middle and not merely to a passive sniffer, and a bypass
/// switch would be clicked the first time a provider hands out a bare IP.
async fn tls_wrap_upstream_hop(
  tcp: TcpStream,
  proxy_host: &str,
) -> Result<tokio_native_tls::TlsStream<TcpStream>, Box<dyn std::error::Error>> {
  let connector = tokio_native_tls::TlsConnector::from(native_tls::TlsConnector::new()?);
  match tokio::time::timeout(UPSTREAM_DIAL_TIMEOUT, connector.connect(proxy_host, tcp)).await {
    Ok(result) => Ok(result?),
    Err(_) => Err(format!("TLS handshake with upstream proxy {proxy_host} timed out").into()),
  }
}

/// Establish a stream to `target_host:target_port`, either directly or through
/// the configured upstream proxy. Shared by the HTTP CONNECT path and the
/// local SOCKS5 server so every upstream type (direct, HTTP/HTTPS CONNECT,
/// TLS-wrapped CONNECT, SOCKS4/5, Shadowsocks) is dialed in exactly one place.
/// Returns a `BoxedAsyncStream` so the caller can tunnel over any upstream
/// uniformly.
pub(crate) async fn connect_to_target_via_upstream(
  target_host: &str,
  target_port: u16,
  upstream_url: Option<&str>,
  bypass_matcher: &BypassMatcher,
) -> Result<BoxedAsyncStream, Box<dyn std::error::Error>> {
  let should_bypass = bypass_matcher.should_bypass(target_host);
  // Helper: configure outbound TCP to match browser TCP fingerprint
  let configure_tcp = |stream: &TcpStream| {
    let _ = stream.set_nodelay(true);
  };
  let target_stream: BoxedAsyncStream = match upstream_url {
    None | Some("DIRECT") => Box::new(dial_direct(target_host, target_port).await?),
    _ if should_bypass => Box::new(dial_direct(target_host, target_port).await?),
    Some(upstream_url_str) => {
      let upstream = Url::parse(upstream_url_str)?;
      let scheme = upstream.scheme();

      match scheme {
        // `https` here is NOT TLS to the proxy: it is a label many providers
        // put on a plaintext CONNECT endpoint, and Donut has always treated it
        // byte-for-byte like `http`. Changing that would silently break every
        // stored `https` proxy, so the encrypted hop is the separate
        // `httpstls` scheme below.
        "http" | "https" => {
          let proxy_host = upstream.host_str().unwrap_or("127.0.0.1");
          let proxy_port = upstream.port().unwrap_or(8080);
          let proxy_stream = tokio::time::timeout(
            UPSTREAM_DIAL_TIMEOUT,
            TcpStream::connect((proxy_host, proxy_port)),
          )
          .await
          .map_err(|_| {
            format!("upstream proxy connect to {proxy_host}:{proxy_port} timed out")
          })??;
          configure_tcp(&proxy_stream);

          connect_via_http_proxy(
            proxy_stream,
            proxy_host,
            proxy_port,
            target_host,
            target_port,
            &upstream,
          )
          .await?
        }
        // TLS to the proxy first, CONNECT second. The target hostname and the
        // `Proxy-Authorization` credentials are written only after the
        // handshake, so neither reaches the wire in the clear.
        "httpstls" => {
          let proxy_host = upstream.host_str().unwrap_or("127.0.0.1");
          let proxy_port = upstream.port().unwrap_or(443);
          let tcp = tokio::time::timeout(
            UPSTREAM_DIAL_TIMEOUT,
            TcpStream::connect((proxy_host, proxy_port)),
          )
          .await
          .map_err(|_| {
            format!("upstream proxy connect to {proxy_host}:{proxy_port} timed out")
          })??;
          configure_tcp(&tcp);

          let tls = tls_wrap_upstream_hop(tcp, proxy_host).await?;

          connect_via_http_proxy(
            tls,
            proxy_host,
            proxy_port,
            target_host,
            target_port,
            &upstream,
          )
          .await?
        }
        "socks4" | "socks5" => {
          let socks_host = upstream.host_str().unwrap_or("127.0.0.1");
          let socks_port = upstream.port().unwrap_or(1080);
          let socks_addr = format!("{}:{}", socks_host, socks_port);

          let (username, password) = upstream_userpass(&upstream);
          let auth = (!username.is_empty()).then_some((username.as_str(), password.as_str()));

          let stream = connect_via_socks(
            &socks_addr,
            target_host,
            target_port,
            scheme == "socks5",
            auth,
          )
          .await?;
          Box::new(stream)
        }
        "ss" | "shadowsocks" => {
          // Shadowsocks: URL format is ss://method:password@host:port
          // where "method" is the cipher (e.g. aes-256-gcm, chacha20-ietf-poly1305)
          // and "password" is the SS server password.
          let ss_host = upstream.host_str().unwrap_or("127.0.0.1");
          let ss_port = upstream.port().unwrap_or(8388);

          // The "username" field carries the cipher method
          let method_str = urlencoding::decode(upstream.username())
            .unwrap_or_default()
            .to_string();
          let password = urlencoding::decode(upstream.password().unwrap_or(""))
            .unwrap_or_default()
            .to_string();

          if method_str.is_empty() || password.is_empty() {
            return Err(
              "Shadowsocks requires method and password (URL: ss://method:password@host:port)"
                .into(),
            );
          }

          let cipher = method_str.parse::<shadowsocks::crypto::CipherKind>().map_err(|_| {
            format!("Unsupported Shadowsocks cipher: {method_str}. Use e.g. aes-256-gcm, chacha20-ietf-poly1305, aes-128-gcm")
          })?;

          let context =
            shadowsocks::context::Context::new_shared(shadowsocks::config::ServerType::Local);
          let svr_cfg = shadowsocks::config::ServerConfig::new(
            shadowsocks::config::ServerAddr::from((ss_host.to_string(), ss_port)),
            &password,
            cipher,
          )
          .map_err(|e| format!("Invalid Shadowsocks config: {e}"))?;

          let target_addr =
            shadowsocks::relay::Address::DomainNameAddress(target_host.to_string(), target_port);

          let stream = tokio::time::timeout(
            UPSTREAM_DIAL_TIMEOUT,
            shadowsocks::relay::tcprelay::proxy_stream::ProxyClientStream::connect(
              context,
              &svr_cfg,
              target_addr,
            ),
          )
          .await
          .map_err(|_| "Shadowsocks connection timed out".to_string())?
          .map_err(|e| format!("Shadowsocks connection failed: {e}"))?;

          Box::new(stream)
        }
        _ => {
          return Err(format!("Unsupported upstream proxy scheme: {}", scheme).into());
        }
      }
    }
  };

  Ok(target_stream)
}

/// Bidirectionally relay `client_stream` <-> `target_stream` until either side
/// closes, counting bytes for traffic stats and attributing them to `domain`.
/// The caller is responsible for having already sent any protocol-specific
/// success reply (HTTP `200` or SOCKS5 reply) before calling this.
pub(crate) async fn tunnel_streams(
  client_stream: TcpStream,
  target_stream: BoxedAsyncStream,
  domain: String,
) {
  // Count each payload byte once, when it is successfully written to its
  // destination. Writes to the target are uploads; writes to the client are
  // downloads.
  let mut counting_client = CountingStream::new(client_stream, TrafficDirection::Received);
  let mut counting_target = CountingStream::new(target_stream, TrafficDirection::Sent);

  log::trace!("Starting bidirectional tunnel");

  // Relay both directions in this single task. Spawning one task per
  // direction and returning when the first finishes would detach the
  // surviving copy, leaving it (and both underlying sockets) alive
  // indefinitely when a peer dies without FIN.
  match tokio::io::copy_bidirectional(&mut counting_client, &mut counting_target).await {
    Ok((to_target, to_client)) => {
      log::trace!("Tunneled {to_target} bytes client->target, {to_client} bytes target->client");
    }
    Err(e) => {
      log::debug!("Tunnel ended with error: {e:?}");
    }
  }

  // Log final byte counts and update domain stats
  let final_sent = counting_target.bytes_written.load(Ordering::Relaxed);
  let final_recv = counting_client.bytes_written.load(Ordering::Relaxed);
  log::trace!("Tunnel closed - sent: {final_sent} bytes, received: {final_recv} bytes");

  // Update domain-specific byte counts now that tunnel is complete
  if let Some(tracker) = get_traffic_tracker() {
    tracker.update_domain_bytes(&domain, final_sent, final_recv);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Write;

  /// Build an upstream URL with `urlencoding::encode`-d user/pass,
  /// mirroring what `proxy_manager::build_proxy_url` actually emits
  fn parse_encoded_upstream(scheme: &str, user: &str, pass: &str) -> Url {
    let s = format!(
      "{}://{}:{}@127.0.0.1:1080",
      scheme,
      urlencoding::encode(user),
      urlencoding::encode(pass),
    );
    Url::parse(&s).unwrap()
  }

  #[test]
  fn upstream_userpass_handles_plain_ascii() {
    let u = parse_encoded_upstream("socks5", "alice", "secret123");
    assert_eq!(upstream_userpass(&u), ("alice".into(), "secret123".into()));
  }

  #[test]
  fn upstream_userpass_decodes_special_chars() {
    // These characters all get percent-encoded by build_proxy_url before
    // landing in the URL, and must be decoded back to the original literal
    // before being handed off to the upstream
    let cases = [
      ("alice", "p@ssw0rd"),
      ("alice", "p:assw0rd"),
      ("alice", "p ass word"),
      ("alice", "abc/d+e=f"),
      ("alice", "100%off!"),
      ("alice", "测试密码"),
      ("u@name", "v@lue"),
    ];
    for (user, pass) in cases {
      let u = parse_encoded_upstream("socks5", user, pass);
      assert_eq!(
        upstream_userpass(&u),
        (user.to_string(), pass.to_string()),
        "decode failed: user={user:?} pass={pass:?}"
      );
    }
  }

  #[test]
  fn upstream_userpass_empty_when_no_credentials() {
    let u = Url::parse("socks5://127.0.0.1:1080").unwrap();
    assert_eq!(upstream_userpass(&u), (String::new(), String::new()));
  }

  #[test]
  fn upstream_userpass_handles_username_only() {
    let s = format!("socks5://{}@127.0.0.1:1080", urlencoding::encode("u@name"));
    let u = Url::parse(&s).unwrap();
    assert_eq!(upstream_userpass(&u), ("u@name".into(), String::new()));
  }

  #[test]
  fn upstream_log_value_never_contains_credentials() {
    assert_eq!(
      redacted_upstream("http://user:p%40ss@example.com:8080"),
      "http://example.com:8080"
    );
    assert_eq!(redacted_upstream("not a URL"), "<redacted>");
    assert_eq!(redacted_upstream(""), "none");
  }

  #[test]
  fn test_blocklist_exact_match() {
    let mut matcher = BlocklistMatcher::new();
    let mut domains = HashSet::new();
    domains.insert("example.com".to_string());
    domains.insert("tracker.net".to_string());
    matcher.domains = Arc::new(domains);

    assert!(matcher.is_blocked("example.com"));
    assert!(matcher.is_blocked("tracker.net"));
    assert!(!matcher.is_blocked("safe.com"));
  }

  #[test]
  fn test_blocklist_subdomain_match() {
    let mut matcher = BlocklistMatcher::new();
    let mut domains = HashSet::new();
    domains.insert("example.com".to_string());
    matcher.domains = Arc::new(domains);

    assert!(matcher.is_blocked("foo.example.com"));
    assert!(matcher.is_blocked("bar.baz.example.com"));
    assert!(matcher.is_blocked("a.b.c.example.com"));
  }

  #[test]
  fn test_blocklist_no_false_positives() {
    let mut matcher = BlocklistMatcher::new();
    let mut domains = HashSet::new();
    domains.insert("example.com".to_string());
    matcher.domains = Arc::new(domains);

    // "notexample.com" should NOT match "example.com"
    assert!(!matcher.is_blocked("notexample.com"));
    assert!(!matcher.is_blocked("myexample.com"));
    // But subdomain should
    assert!(matcher.is_blocked("sub.example.com"));
  }

  #[test]
  fn test_blocklist_empty_blocks_nothing() {
    let matcher = BlocklistMatcher::new();
    assert!(!matcher.is_blocked("anything.com"));
    assert!(!matcher.is_blocked("example.com"));
  }

  #[test]
  fn test_allowlist_mode_blocks_everything_not_listed() {
    let mut matcher = BlocklistMatcher::new();
    let mut domains = HashSet::new();
    domains.insert("example.com".to_string());
    domains.insert("api.trusted.io".to_string());
    matcher.domains = Arc::new(domains);
    matcher.allowlist_mode = true;

    // Listed domains (and their subdomains) are allowed.
    assert!(!matcher.is_blocked("example.com"));
    assert!(!matcher.is_blocked("cdn.example.com"));
    assert!(!matcher.is_blocked("api.trusted.io"));
    // Everything else is blocked.
    assert!(matcher.is_blocked("evil.com"));
    assert!(matcher.is_blocked("trusted.io")); // parent of api.trusted.io is NOT allowed
    assert!(matcher.is_blocked("google.com"));
  }

  #[test]
  fn test_allowlist_mode_empty_fails_open() {
    let mut matcher = BlocklistMatcher::new();
    matcher.allowlist_mode = true;
    // Empty allowlist would block everything and brick the browser — fail open.
    assert!(!matcher.is_blocked("anything.com"));
  }

  #[test]
  fn test_blocklist_case_insensitive() {
    let mut matcher = BlocklistMatcher::new();
    let mut domains = HashSet::new();
    domains.insert("example.com".to_string());
    matcher.domains = Arc::new(domains);

    assert!(matcher.is_blocked("EXAMPLE.COM"));
    assert!(matcher.is_blocked("Example.Com"));
    assert!(matcher.is_blocked("FOO.EXAMPLE.COM"));
  }

  #[test]
  fn test_blocklist_from_file() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    writeln!(tmpfile, "# This is a comment").unwrap();
    writeln!(tmpfile).unwrap();
    writeln!(tmpfile, "tracker.example.com").unwrap();
    writeln!(tmpfile, "ads.network.com").unwrap();
    writeln!(tmpfile, "# Another comment").unwrap();
    writeln!(tmpfile, "malware.site").unwrap();
    tmpfile.flush().unwrap();

    let matcher = BlocklistMatcher::from_file(tmpfile.path().to_str().unwrap()).unwrap();

    assert!(matcher.is_blocked("tracker.example.com"));
    assert!(matcher.is_blocked("ads.network.com"));
    assert!(matcher.is_blocked("malware.site"));
    assert!(matcher.is_blocked("sub.malware.site"));
    assert!(!matcher.is_blocked("safe.com"));
    // Comments and empty lines should be skipped: 3 domains loaded
    assert_eq!(matcher.domains.len(), 3);
  }

  /// Serve one canned upstream CONNECT reply, written as the given segments so
  /// the reader is forced to cope with real TCP framing.
  async fn serve_connect_reply(
    segments: Vec<&'static [u8]>,
  ) -> (TcpStream, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
      let (mut s, _) = listener.accept().await.unwrap();
      let _ = s.set_nodelay(true);
      for seg in segments {
        if s.write_all(seg).await.is_err() {
          return;
        }
        let _ = s.flush().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
      }
      // Hold the connection open so the reader never sees a premature EOF.
      tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });
    let client = TcpStream::connect(addr).await.unwrap();
    (client, server)
  }

  #[tokio::test]
  async fn read_upstream_connect_response_forwards_coalesced_payload() {
    // The upstream packs the destination's first bytes into the same segment as
    // its 200. Dropping them corrupts the tunnel for any server-speaks-first
    // protocol, so they must come back as leftover for the caller to replay.
    let (mut client, server) = serve_connect_reply(vec![
      b"HTTP/1.1 200 Connection Established\r\n\r\nSSH-2.0-OpenSSH_9.6",
    ])
    .await;

    let (headers, leftover) = read_upstream_connect_response(&mut client).await.unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert_eq!(leftover, b"SSH-2.0-OpenSSH_9.6");
    server.abort();
  }

  #[tokio::test]
  async fn read_upstream_connect_response_accepts_split_status_line() {
    // A single read would see only "HTTP/1.1 " here and reject a tunnel the
    // upstream actually granted.
    let (mut client, server) = serve_connect_reply(vec![
      b"HTTP/1.1 ",
      b"200 Connection Established\r\n",
      b"Proxy-Agent: squid\r\n\r\n",
    ])
    .await;

    let (headers, leftover) = read_upstream_connect_response(&mut client).await.unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(headers.contains("Proxy-Agent: squid"));
    assert!(
      leftover.is_empty(),
      "no payload followed the headers, so nothing should be replayed"
    );
    server.abort();
  }

  #[tokio::test]
  async fn read_upstream_connect_response_waits_for_terminator_across_segments() {
    // The terminating CRLFCRLF straddles two segments. Without a scan that
    // spans the boundary the reader would miss it and relay header bytes into
    // the tunnel as if they were payload.
    let (mut client, server) = serve_connect_reply(vec![
      b"HTTP/1.1 200 OK\r\nProxy-Agent: x\r",
      b"\n\r\nPAYLOAD",
    ])
    .await;

    let (headers, leftover) = read_upstream_connect_response(&mut client).await.unwrap();
    assert!(headers.ends_with("\r\n\r\n"));
    assert_eq!(leftover, b"PAYLOAD");
    server.abort();
  }

  #[tokio::test]
  async fn read_upstream_connect_response_errors_on_early_close() {
    let (mut client, server) = serve_connect_reply(vec![]).await;
    // serve_connect_reply holds the socket open with no data; a closed upstream
    // is simulated by dropping the server task and shutting the peer down.
    server.abort();
    let _ = client.shutdown().await;
    let result = read_upstream_connect_response(&mut client).await;
    assert!(result.is_err(), "a CONNECT with no reply must not succeed");
  }

  #[tokio::test]
  async fn read_http_response_buffer_caps_endless_header_stream() {
    let (mut writer, mut reader) = tokio::io::duplex(16 * 1024);
    let feeder = tokio::spawn(async move {
      // Stream bytes that never contain CRLFCRLF.
      let chunk = [b'a'; 4096];
      loop {
        if writer.write_all(&chunk).await.is_err() {
          break;
        }
      }
    });

    let buf = read_http_response_buffer(&mut reader).await;
    assert!(
      buf.bytes.len() <= MAX_HTTP_HEADER_BUFFER + 4096,
      "pre-header buffering must stop at the cap, got {} bytes",
      buf.bytes.len()
    );
    assert!(
      buf.truncated,
      "a header stream that never terminates must be reported as truncated"
    );
    feeder.abort();
  }

  #[tokio::test]
  async fn read_http_response_buffer_reads_content_length_body() {
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    let resp: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
    writer.write_all(resp).await.unwrap();
    drop(writer);

    let buf = read_http_response_buffer(&mut reader).await;
    assert_eq!(buf.bytes, resp);
    assert!(!buf.truncated);
  }

  /// Frame `pieces` as a chunked body, terminator included.
  fn chunked_wire(pieces: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for piece in pieces {
      out.extend_from_slice(format!("{:x}\r\n", piece.len()).as_bytes());
      out.extend_from_slice(piece.as_bytes());
      out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
  }

  #[tokio::test]
  async fn read_http_response_buffer_dechunks_a_chunked_body() {
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    let mut resp = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    resp.extend_from_slice(&chunked_wire(&["hello ", "world"]));
    writer.write_all(&resp).await.unwrap();
    drop(writer);

    let buf = read_http_response_buffer(&mut reader).await;
    assert!(!buf.truncated);
    match buf.body {
      BufferedBody::Dechunked(body) => assert_eq!(body, b"hello world".to_vec()),
      _ => panic!("a chunked body must be de-framed before it reaches the browser"),
    }
  }

  #[test]
  fn chunked_body_decodes_to_the_payload_alone() {
    let wire = chunked_wire(&["hello ", "world"]);
    let mut cursor = 0;
    let mut out = Vec::new();
    assert!(matches!(
      decode_chunked(&wire, &mut cursor, &mut out),
      ChunkedState::Complete
    ));
    assert_eq!(out, b"hello world".to_vec());
  }

  #[test]
  fn a_chunked_body_split_across_reads_is_walked_once() {
    let wire = chunked_wire(&["one", "two"]);
    let mut cursor = 0;
    let mut out = Vec::new();
    // Half the buffer holds the first chunk and part of the second header.
    assert!(matches!(
      decode_chunked(&wire[..wire.len() / 2], &mut cursor, &mut out),
      ChunkedState::Incomplete
    ));
    assert!(matches!(
      decode_chunked(&wire, &mut cursor, &mut out),
      ChunkedState::Complete
    ));
    assert_eq!(out, b"onetwo".to_vec());
  }

  #[test]
  fn chunk_extensions_are_ignored() {
    let mut cursor = 0;
    let mut out = Vec::new();
    assert!(matches!(
      decode_chunked(b"5;name=value\r\nhello\r\n0\r\n\r\n", &mut cursor, &mut out),
      ChunkedState::Complete
    ));
    assert_eq!(out, b"hello".to_vec());
  }

  #[test]
  fn a_malformed_chunk_stream_is_rejected_not_half_decoded() {
    for wire in [
      // A size that is not hexadecimal.
      b"zz\r\nnope\r\n0\r\n\r\n".to_vec(),
      // Chunk data not followed by its CRLF.
      b"5\r\nhelloXX\r\n0\r\n\r\n".to_vec(),
    ] {
      let mut cursor = 0;
      let mut out = Vec::new();
      assert!(
        matches!(
          decode_chunked(&wire, &mut cursor, &mut out),
          ChunkedState::Malformed
        ),
        "{}",
        String::from_utf8_lossy(&wire)
      );
    }
  }

  #[test]
  fn upstream_response_headers_reach_the_browser() {
    let block = b"HTTP/1.1 302 Found\r\n\
Location: https://example.com/next\r\n\
Set-Cookie: a=1; Path=/\r\n\
Set-Cookie: b=2; Path=/\r\n\
Content-Type: text/html; charset=utf-8\r\n\
Content-Length: 17\r\n\
Transfer-Encoding: chunked\r\n\
Connection: keep-alive\r\n\
this line has no colon\r\n\
\r\n";
    let mut response = Response::new(Full::new(Bytes::new()));
    forward_upstream_headers(&mut response, block);
    let headers = response.headers();

    assert_eq!(headers.get("location").unwrap(), "https://example.com/next");
    assert_eq!(
      headers.get("content-type").unwrap(),
      "text/html; charset=utf-8"
    );
    // Every Set-Cookie survives, so a sign-in actually establishes a session.
    let cookies: Vec<&str> = headers
      .get_all("set-cookie")
      .iter()
      .map(|value| value.to_str().unwrap())
      .collect();
    assert_eq!(cookies, ["a=1; Path=/", "b=2; Path=/"]);
    // hyper re-derives the framing for the body it is handed; the upstream's
    // own framing headers would contradict it.
    for framing in ["content-length", "transfer-encoding", "connection"] {
      assert!(
        headers.get(framing).is_none(),
        "{framing} must not be forwarded"
      );
    }
  }

  #[tokio::test]
  async fn read_http_response_buffer_caps_oversized_content_length_body() {
    let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);
    let feeder = tokio::spawn(async move {
      let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        MAX_HTTP_RESPONSE_BUFFER * 2
      );
      if writer.write_all(header.as_bytes()).await.is_err() {
        return;
      }
      let chunk = [b'b'; 8192];
      loop {
        if writer.write_all(&chunk).await.is_err() {
          break;
        }
      }
    });

    let buf = read_http_response_buffer(&mut reader).await;
    assert!(
      buf.bytes.len() <= MAX_HTTP_RESPONSE_BUFFER + 8192,
      "body buffering must stop at the cap, got {} bytes",
      buf.bytes.len()
    );
    assert!(
      buf.truncated,
      "a body cut short by the cap must be reported as truncated so the caller \
       fails the request instead of forwarding a short response"
    );
    feeder.abort();
  }

  #[tokio::test]
  #[serial_test::serial]
  async fn tunnel_traffic_counts_chunked_duplex_bytes_once_per_direction() {
    let temp_dir = tempfile::tempdir().unwrap();
    let _cache_guard = crate::app_dirs::set_test_cache_dir(temp_dir.path().to_path_buf());
    let profile_id = "traffic-counting-profile";
    let domain = "counting.example";
    init_traffic_tracker("traffic-counting-proxy".into(), Some(profile_id.into()));
    let tracker = get_traffic_tracker().unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (browser_result, accepted_result) =
      tokio::join!(TcpStream::connect(addr), listener.accept());
    let browser_stream = browser_result.unwrap();
    let (proxy_client_stream, _) = accepted_result.unwrap();

    // Keep the duplex buffer deliberately small so client-to-target writes
    // must make partial progress while both directions remain active.
    let (proxy_target_stream, target_stream) = tokio::io::duplex(11);
    let tunnel = tokio::spawn(tunnel_streams(
      proxy_client_stream,
      Box::new(proxy_target_stream),
      domain.into(),
    ));

    let upload_chunks = vec![vec![0x11; 3], vec![0x22; 31], vec![0x33; 8_193]];
    let download_chunks = vec![vec![0x44; 5], vec![0x55; 47], vec![0x66; 5_003]];
    let expected_upload = upload_chunks.concat();
    let expected_download = download_chunks.concat();
    let upload_len = expected_upload.len();
    let download_len = expected_download.len();

    let (browser_reader, mut browser_writer) = browser_stream.into_split();
    let (target_reader, mut target_writer) = tokio::io::split(target_stream);
    let transfer = async move {
      let send_upload = async move {
        for chunk in upload_chunks {
          browser_writer.write_all(&chunk).await.unwrap();
          tokio::task::yield_now().await;
        }
        browser_writer.flush().await.unwrap();
        browser_writer
      };
      let send_download = async move {
        for chunk in download_chunks {
          target_writer.write_all(&chunk).await.unwrap();
          tokio::task::yield_now().await;
        }
        target_writer.flush().await.unwrap();
        target_writer
      };
      let receive_upload = async move {
        let mut target_reader = target_reader;
        let mut bytes = vec![0; upload_len];
        target_reader.read_exact(&mut bytes).await.unwrap();
        (target_reader, bytes)
      };
      let receive_download = async move {
        let mut browser_reader = browser_reader;
        let mut bytes = vec![0; download_len];
        browser_reader.read_exact(&mut bytes).await.unwrap();
        (browser_reader, bytes)
      };

      tokio::join!(send_upload, send_download, receive_upload, receive_download)
    };

    let (
      mut browser_writer,
      mut target_writer,
      (target_reader, actual_upload),
      (browser_reader, actual_download),
    ) = tokio::time::timeout(std::time::Duration::from_secs(5), transfer)
      .await
      .expect("duplex transfer timed out");

    assert_eq!(actual_upload, expected_upload);
    assert_eq!(actual_download, expected_download);
    assert!(
      !tunnel.is_finished(),
      "the tunnel should remain live until its peers close"
    );
    assert_eq!(
      tracker.get_snapshot(),
      (upload_len as u64, download_len as u64, 0),
      "global counters must update in real time without double-counting"
    );

    let (browser_shutdown, target_shutdown) =
      tokio::join!(browser_writer.shutdown(), target_writer.shutdown());
    browser_shutdown.unwrap();
    target_shutdown.unwrap();
    drop((browser_writer, target_writer, browser_reader, target_reader));
    tokio::time::timeout(std::time::Duration::from_secs(5), tunnel)
      .await
      .expect("tunnel did not close")
      .expect("tunnel task panicked");

    assert_eq!(
      tracker.get_snapshot(),
      (upload_len as u64, download_len as u64, 0),
      "closing the tunnel must not add another copy of its traffic"
    );
    assert_eq!(
      tracker.flush_to_disk().unwrap(),
      Some((upload_len as u64, download_len as u64))
    );

    let stats = crate::traffic_stats::load_traffic_stats(profile_id).unwrap();
    assert_eq!(stats.total_bytes_sent, upload_len as u64);
    assert_eq!(stats.total_bytes_received, download_len as u64);
    let domain_stats = stats.domains.get(domain).unwrap();
    assert_eq!(domain_stats.bytes_sent, upload_len as u64);
    assert_eq!(domain_stats.bytes_received, download_len as u64);
  }

  /// Dial `connect_to_target_via_upstream` at a listener that never answers and
  /// return the first bytes it puts on the wire.
  ///
  /// The dial cannot complete (nothing on the far end speaks proxy or TLS), and
  /// that is the point: what matters is what leaves this machine BEFORE the
  /// other side has proved anything.
  async fn first_bytes_sent_to_upstream(scheme: &str) -> Vec<u8> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = format!("{scheme}://donutuser:hunter2secret@127.0.0.1:{port}");

    let dial = tokio::spawn(async move {
      let matcher = BypassMatcher::new(&[]);
      let _ = connect_to_target_via_upstream(
        "private-target.example.com",
        443,
        Some(&upstream),
        &matcher,
      )
      .await;
    });

    let (mut server, _) = listener.accept().await.unwrap();
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(10), server.read(&mut buf))
      .await
      .expect("upstream saw no bytes at all before the timeout")
      .expect("reading from the mock upstream failed");
    buf.truncate(n);

    dial.abort();
    buf
  }

  #[tokio::test]
  async fn httpstls_upstream_negotiates_tls_before_writing_anything_readable() {
    // This is the whole point of the type. Nothing readable may reach the wire
    // ahead of the TLS handshake: not the CONNECT verb, not the target host,
    // and above all not the Proxy-Authorization credentials. The handshake here
    // never completes, which proves the credentials never left the machine.
    let first = first_bytes_sent_to_upstream("httpstls").await;

    assert_eq!(
      first.first().copied(),
      Some(0x16),
      "the first byte must be a TLS handshake record (0x16), got {:02x?}",
      &first[..first.len().min(16)]
    );

    let as_text = String::from_utf8_lossy(&first);
    for secret in [
      "CONNECT ",
      "Proxy-Authorization",
      "private-target.example.com",
      "donutuser",
      "hunter2secret",
    ] {
      assert!(
        !as_text.contains(secret),
        "{secret:?} reached the wire in the clear on an httpstls upstream"
      );
    }
  }

  #[tokio::test]
  async fn http_upstream_still_writes_a_plaintext_connect() {
    // The counterpart, pinning today's behaviour rather than wishing it away.
    // If this ever stops holding, the `http` path changed and every stored
    // plaintext proxy changed with it.
    let first = first_bytes_sent_to_upstream("http").await;
    let as_text = String::from_utf8_lossy(&first);

    assert!(
      as_text.starts_with("CONNECT private-target.example.com:443 "),
      "expected a plaintext CONNECT, got {as_text:?}"
    );
    assert!(
      as_text.contains("Proxy-Authorization: Basic "),
      "expected plaintext proxy credentials, got {as_text:?}"
    );
  }

  #[tokio::test]
  async fn https_upstream_is_a_plaintext_hop_despite_the_name() {
    // `https` is a provider label, not TLS to the proxy. The UI now says so;
    // this is the assertion that keeps the code and the copy agreeing.
    let first = first_bytes_sent_to_upstream("https").await;
    assert!(
      String::from_utf8_lossy(&first).starts_with("CONNECT "),
      "the `https` type must keep behaving exactly like `http`"
    );
    assert_ne!(
      first.first().copied(),
      Some(0x16),
      "`https` must not have silently become a TLS hop"
    );
  }

  #[tokio::test]
  async fn httpstls_refuses_a_hop_that_answers_in_plaintext() {
    // The downgrade case: a proxy (or something sitting in front of it)
    // answering the way a plaintext CONNECT endpoint would. There is no
    // opportunistic fallback, a failed handshake is a failed connection, or
    // the whole type is worth nothing against an active attacker.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
      let (mut socket, _) = listener.accept().await.unwrap();
      let mut scratch = [0u8; 1024];
      let _ = socket.read(&mut scratch).await;
      let _ = socket
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await;
      tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });

    let upstream = format!("httpstls://user:pass@127.0.0.1:{port}");
    let matcher = BypassMatcher::new(&[]);
    let result = tokio::time::timeout(
      std::time::Duration::from_secs(20),
      connect_to_target_via_upstream("example.com", 443, Some(&upstream), &matcher),
    )
    .await
    .expect("the dial must not hang");

    assert!(
      result.is_err(),
      "a plaintext answer must not yield a usable tunnel on an httpstls upstream"
    );
    server.abort();
  }

  #[tokio::test]
  async fn build_reqwest_client_with_proxy_accepts_the_tls_scheme() {
    // reqwest rejects the `httpstls` scheme outright, so without the rewrite
    // every plain-HTTP request through such a proxy fails to build a client.
    build_reqwest_client_with_proxy("httpstls://user:pass@proxy.example.com:443")
      .expect("httpstls must build a reqwest client");
    build_reqwest_client_with_proxy("http://proxy.example.com:8080")
      .expect("http must keep building a reqwest client");
  }

  #[tokio::test]
  async fn prepend_reader_replays_payload_over_a_non_tcp_stream() {
    // The coalesced-payload replay has to survive the TLS stream type, not just
    // TcpStream. `duplex` stands in for any non-TCP AsyncRead+AsyncWrite.
    let (mut peer, inner) = tokio::io::duplex(1024);
    peer.write_all(b"-rest-of-stream").await.unwrap();

    let mut reader = PrependReader {
      prepended: b"replayed-".to_vec(),
      prepended_pos: 0,
      inner,
    };

    let mut got = [0u8; 24];
    let mut filled = 0;
    while filled < got.len() {
      let n = reader.read(&mut got[filled..]).await.unwrap();
      assert_ne!(n, 0, "stream ended before the expected bytes arrived");
      filled += n;
    }
    assert_eq!(&got[..filled], b"replayed--rest-of-stream");
  }

  #[tokio::test]
  async fn read_upstream_connect_response_works_off_a_non_tcp_stream() {
    // Guards the generic bound: a `&mut TcpStream` signature would not compile
    // against the TLS stream the httpstls path hands it.
    let (mut peer, mut inner) = tokio::io::duplex(1024);
    peer
      .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nBANNER")
      .await
      .unwrap();

    let (headers, leftover) = read_upstream_connect_response(&mut inner).await.unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert_eq!(leftover, b"BANNER");
  }

  #[test]
  fn test_blocklist_comments_skipped() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    writeln!(tmpfile, "# Title: HaGeZi's Light DNS Blocklist").unwrap();
    writeln!(tmpfile, "# Description: test").unwrap();
    writeln!(tmpfile, "# Version: 2026.0330.0928.01").unwrap();
    writeln!(tmpfile).unwrap();
    writeln!(tmpfile, "domain1.com").unwrap();
    writeln!(tmpfile, "domain2.com").unwrap();
    tmpfile.flush().unwrap();

    let matcher = BlocklistMatcher::from_file(tmpfile.path().to_str().unwrap()).unwrap();
    assert_eq!(matcher.domains.len(), 2);
    assert!(matcher.is_blocked("domain1.com"));
    assert!(matcher.is_blocked("domain2.com"));
  }
}
