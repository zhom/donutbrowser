use crate::proxy_storage::ProxyConfig;
use crate::traffic_stats::{get_traffic_tracker, init_traffic_tracker, LiveTrafficTracker};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
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

/// Wrapper stream that counts bytes read and written
struct CountingStream<S> {
  inner: S,
  bytes_read: Arc<AtomicU64>,
  bytes_written: Arc<AtomicU64>,
  // Resolved once per stream: the global tracker is fixed after init, so the
  // hot poll paths avoid taking the global RwLock on every packet
  tracker: Option<Arc<LiveTrafficTracker>>,
}

impl<S> CountingStream<S> {
  fn new(inner: S) -> Self {
    Self {
      inner,
      bytes_read: Arc::new(AtomicU64::new(0)),
      bytes_written: Arc::new(AtomicU64::new(0)),
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
    let filled_before = buf.filled().len();
    let result = Pin::new(&mut self.inner).poll_read(cx, buf);
    if let Poll::Ready(Ok(())) = &result {
      let bytes_read = buf.filled().len() - filled_before;
      if bytes_read > 0 {
        self
          .bytes_read
          .fetch_add(bytes_read as u64, Ordering::Relaxed);
        // Update global tracker - count as received (data coming into proxy)
        if let Some(tracker) = &self.tracker {
          tracker.add_bytes_received(bytes_read as u64);
        }
      }
    }
    result
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
      // Update global tracker - count as sent (data going out of proxy)
      if let Some(tracker) = &self.tracker {
        tracker.add_bytes_sent(*n as u64);
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

// Wrapper to prepend consumed bytes to a stream
struct PrependReader {
  prepended: Vec<u8>,
  prepended_pos: usize,
  inner: TcpStream,
}

impl AsyncRead for PrependReader {
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

impl AsyncWrite for PrependReader {
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

/// A buffered HTTP response read off a raw upstream stream.
struct BufferedHttpResponse {
  bytes: Vec<u8>,
  /// True when the read stopped at `MAX_HTTP_HEADER_BUFFER` /
  /// `MAX_HTTP_RESPONSE_BUFFER` rather than at the end of the response, so
  /// `bytes` holds only a prefix. Callers must fail the request instead of
  /// forwarding it: hyper derives a fresh Content-Length from whatever body it
  /// is handed, so a truncated response reaches the browser as a well-formed,
  /// self-consistent short one and silently corrupts the download.
  truncated: bool,
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
          } else if !is_chunked {
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
          // Note: Chunked encoding is complex to parse manually, so we'll read what we can
          // For full chunked support, we'd need a proper HTTP parser
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
  let response_buffer = buffered.bytes;

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

  let body = response_buffer[header_end..].to_vec();

  // Record request in traffic tracker
  let response_size = body.len() as u64;
  if let Some(tracker) = get_traffic_tracker() {
    tracker.record_request(&domain, body_bytes.len() as u64, response_size);
  }

  let mut hyper_response = Response::new(Full::new(Bytes::from(body)));
  *hyper_response.status_mut() = StatusCode::from_u16(status_code).unwrap();

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

  // Parse the raw HTTP response
  let response_str = String::from_utf8_lossy(&response_buf);
  let header_end = response_str.find("\r\n\r\n").unwrap_or(response_str.len());
  let status_line = response_str
    .lines()
    .next()
    .unwrap_or("HTTP/1.1 502 Bad Gateway");
  let status_code: u16 = status_line
    .split_whitespace()
    .nth(1)
    .and_then(|s| s.parse().ok())
    .unwrap_or(502);
  let body = if header_end + 4 < response_buf.len() {
    &response_buf[header_end + 4..]
  } else {
    b""
  };

  let mut hyper_response = Response::new(Full::new(Bytes::from(body.to_vec())));
  *hyper_response.status_mut() =
    StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_GATEWAY);

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
      // For HTTP/HTTPS proxies, reqwest handles them directly
      // Note: HTTPS proxy URLs still use HTTP CONNECT method, reqwest handles TLS automatically
      Proxy::http(upstream_url)?
    }
    "socks5" => {
      // Donut: force REMOTE (proxy-side) DNS for plaintext HTTP over a SOCKS5
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
  // when the GUI restarts). Once the GUI records the browser PID this worker
  // serves, poll it and exit when that browser is gone — never while it is
  // alive, and never before a PID is recorded (covers the launch window and
  // pre-upgrade configs lacking the field). A 2-miss debounce avoids exiting on
  // a transient sysinfo false-negative under load / sleep-wake.
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
    std::thread::spawn(move || {
      let mut consecutive_misses: u32 = 0;
      loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        match crate::proxy_storage::get_proxy_config(&watch_id) {
          Some(cfg) => match cfg.browser_pid {
            Some(bpid) if bpid != 0 => {
              if crate::proxy_storage::is_process_running(bpid) {
                consecutive_misses = 0;
              } else {
                consecutive_misses += 1;
                if consecutive_misses >= 2 {
                  log::info!("Browser PID {bpid} for config {watch_id} is gone; worker exiting");
                  crate::proxy_storage::delete_proxy_config(&watch_id);
                  std::process::exit(0);
                }
              }
            }
            // No browser PID recorded yet (launch window / old config): keep running.
            _ => consecutive_misses = 0,
          },
          // Our own config was removed (e.g. GUI stopped us): nothing to serve.
          None => {
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
async fn read_upstream_connect_response(
  stream: &mut TcpStream,
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

/// Establish a stream to `target_host:target_port`, either directly or through
/// the configured upstream proxy. Shared by the HTTP CONNECT path and the
/// local SOCKS5 server so every upstream type (direct, HTTP/HTTPS CONNECT,
/// SOCKS4/5, Shadowsocks) is dialed in exactly one place. Returns a
/// `BoxedAsyncStream` so the caller can tunnel over any upstream uniformly.
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
        "http" | "https" => {
          let proxy_host = upstream.host_str().unwrap_or("127.0.0.1");
          let proxy_port = upstream.port().unwrap_or(8080);
          let mut proxy_stream = tokio::time::timeout(
            UPSTREAM_DIAL_TIMEOUT,
            TcpStream::connect((proxy_host, proxy_port)),
          )
          .await
          .map_err(|_| {
            format!("upstream proxy connect to {proxy_host}:{proxy_port} timed out")
          })??;
          configure_tcp(&proxy_stream);

          let mut connect_req = format!(
            "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n",
            target_host, target_port, target_host, target_port
          );

          let (username, password) = upstream_userpass(&upstream);
          if !username.is_empty() {
            use base64::{engine::general_purpose, Engine as _};
            let auth = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
            connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth));
          }

          connect_req.push_str("\r\n");

          proxy_stream.write_all(connect_req.as_bytes()).await?;

          let (response_headers, coalesced) =
            read_upstream_connect_response(&mut proxy_stream).await?;
          let status_line = response_headers.lines().next().unwrap_or("").to_string();

          if !response_headers.starts_with("HTTP/1.1 200")
            && !response_headers.starts_with("HTTP/1.0 200")
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
            Box::new(proxy_stream)
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
            Box::new(PrependReader {
              prepended: coalesced,
              prepended_pos: 0,
              inner: proxy_stream,
            })
          }
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
  // Wrap streams to count bytes transferred
  let mut counting_client = CountingStream::new(client_stream);
  let mut counting_target = CountingStream::new(target_stream);

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
  let final_sent = counting_client.bytes_read.load(Ordering::Relaxed)
    + counting_target.bytes_written.load(Ordering::Relaxed);
  let final_recv = counting_target.bytes_read.load(Ordering::Relaxed)
    + counting_client.bytes_written.load(Ordering::Relaxed);
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
