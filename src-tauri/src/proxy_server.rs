use crate::proxy_storage::ProxyConfig;
use crate::traffic_stats::{get_traffic_tracker, init_traffic_tracker};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use regex_lite::Regex;
use std::collections::HashSet;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

/// Combined read+write trait for tunnel target streams, allowing
/// `handle_connect_from_buffer` to handle plain TCP, SOCKS, and
/// Shadowsocks through the same bidirectional-copy path.
trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}
type BoxedAsyncStream = Box<dyn AsyncStream>;
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
    }
  }

  pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let domains: HashSet<String> = content
      .lines()
      .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
      .map(|line| line.trim().to_lowercase())
      .collect();
    log::info!("[blocklist] Loaded {} domains from {}", domains.len(), path);
    Ok(Self {
      domains: Arc::new(domains),
    })
  }

  pub fn is_blocked(&self, host: &str) -> bool {
    if self.domains.is_empty() {
      return false;
    }
    let host_lower = host.to_lowercase();
    // Exact match
    if self.domains.contains(host_lower.as_str()) {
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
}

/// Wrapper stream that counts bytes read and written
struct CountingStream<S> {
  inner: S,
  bytes_read: Arc<AtomicU64>,
  bytes_written: Arc<AtomicU64>,
}

impl<S> CountingStream<S> {
  fn new(inner: S) -> Self {
    Self {
      inner,
      bytes_read: Arc::new(AtomicU64::new(0)),
      bytes_written: Arc::new(AtomicU64::new(0)),
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
        if let Some(tracker) = get_traffic_tracker() {
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
      if let Some(tracker) = get_traffic_tracker() {
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
  // Handle CONNECT method for HTTPS tunneling
  if req.method() == Method::CONNECT {
    return handle_connect(req, upstream_url, bypass_matcher, blocklist_matcher).await;
  }

  // Handle regular HTTP requests
  handle_http(req, upstream_url, bypass_matcher, blocklist_matcher).await
}

async fn handle_connect(
  req: Request<hyper::body::Incoming>,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) -> Result<Response<Full<Bytes>>, Infallible> {
  let authority = req.uri().authority().cloned();

  if let Some(authority) = authority {
    let target_addr = format!("{}", authority);

    // Parse target host and port
    let (target_host, target_port) = if let Some(colon_pos) = target_addr.find(':') {
      let host = &target_addr[..colon_pos];
      let port: u16 = target_addr[colon_pos + 1..].parse().unwrap_or(443);
      (host, port)
    } else {
      (&target_addr[..], 443)
    };

    // Block if domain is in the DNS blocklist (before any connection)
    if blocklist_matcher.is_blocked(target_host) {
      log::debug!("[blocklist] Blocked CONNECT to {}", target_host);
      let mut response = Response::new(Full::new(Bytes::from("Blocked by DNS blocklist")));
      *response.status_mut() = StatusCode::FORBIDDEN;
      return Ok(response);
    }

    // If no upstream proxy, or bypass rule matches, connect directly
    if upstream_url.is_none()
      || upstream_url
        .as_ref()
        .map(|s| s == "DIRECT")
        .unwrap_or(false)
      || bypass_matcher.should_bypass(target_host)
    {
      match TcpStream::connect(&target_addr).await {
        Ok(_stream) => {
          let mut response = Response::new(Full::new(Bytes::from("")));
          *response.status_mut() = StatusCode::from_u16(200).unwrap();
          return Ok(response);
        }
        Err(e) => {
          log::error!("Failed to connect to {}: {}", target_addr, e);
          let mut response =
            Response::new(Full::new(Bytes::from(format!("Connection failed: {}", e))));
          *response.status_mut() = StatusCode::BAD_GATEWAY;
          return Ok(response);
        }
      }
    }

    // Connect through upstream proxy
    let upstream = match upstream_url.as_ref().and_then(|u| Url::parse(u).ok()) {
      Some(url) => url,
      None => {
        let mut response = Response::new(Full::new(Bytes::from("Invalid upstream URL")));
        *response.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(response);
      }
    };

    let scheme = upstream.scheme();
    match scheme {
      "http" | "https" => {
        // Use manual CONNECT for HTTP/HTTPS proxies
        match connect_via_http_proxy(&upstream, target_host, target_port).await {
          Ok(_) => {
            let mut response = Response::new(Full::new(Bytes::from("")));
            *response.status_mut() = StatusCode::from_u16(200).unwrap();
            Ok(response)
          }
          Err(e) => {
            log::error!("HTTP proxy CONNECT failed: {}", e);
            let mut response = Response::new(Full::new(Bytes::from(format!(
              "Proxy connection failed: {}",
              e
            ))));
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            Ok(response)
          }
        }
      }
      "socks4" | "socks5" => {
        // Use async-socks5 for SOCKS proxies
        let host = upstream.host_str().unwrap_or("127.0.0.1");
        let port = upstream.port().unwrap_or(1080);
        let socks_addr = format!("{}:{}", host, port);

        let username = upstream.username();
        let password = upstream.password().unwrap_or("");

        match connect_via_socks(
          &socks_addr,
          target_host,
          target_port,
          scheme == "socks5",
          if !username.is_empty() {
            Some((username, password))
          } else {
            None
          },
        )
        .await
        {
          Ok(_stream) => {
            let mut response = Response::new(Full::new(Bytes::from("")));
            *response.status_mut() = StatusCode::from_u16(200).unwrap();
            Ok(response)
          }
          Err(e) => {
            log::error!("SOCKS connection failed: {}", e);
            let mut response = Response::new(Full::new(Bytes::from(format!(
              "SOCKS connection failed: {}",
              e
            ))));
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            Ok(response)
          }
        }
      }
      _ => {
        let mut response = Response::new(Full::new(Bytes::from("Unsupported upstream scheme")));
        *response.status_mut() = StatusCode::BAD_GATEWAY;
        Ok(response)
      }
    }
  } else {
    let mut response = Response::new(Full::new(Bytes::from("Bad Request")));
    *response.status_mut() = StatusCode::BAD_REQUEST;
    Ok(response)
  }
}

async fn connect_via_http_proxy(
  upstream: &Url,
  target_host: &str,
  target_port: u16,
) -> Result<TcpStream, Box<dyn std::error::Error>> {
  let proxy_host = upstream.host_str().unwrap_or("127.0.0.1");
  let proxy_port = upstream.port().unwrap_or(8080);
  let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;

  // Add proxy authentication if provided
  let mut connect_req = format!(
    "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n",
    target_host, target_port, target_host, target_port
  );

  if !upstream.username().is_empty() {
    use base64::{engine::general_purpose, Engine as _};
    let username = upstream.username();
    let password = upstream.password().unwrap_or("");
    let auth = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth));
  }

  connect_req.push_str("\r\n");

  stream.write_all(connect_req.as_bytes()).await?;

  let mut buffer = [0u8; 4096];
  let n = stream.read(&mut buffer).await?;
  let response = String::from_utf8_lossy(&buffer[..n]);

  if response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200") {
    Ok(stream)
  } else {
    Err(format!("Upstream proxy CONNECT failed: {}", response).into())
  }
}

async fn connect_via_socks(
  socks_addr: &str,
  target_host: &str,
  target_port: u16,
  is_socks5: bool,
  auth: Option<(&str, &str)>,
) -> Result<TcpStream, Box<dyn std::error::Error>> {
  let mut stream = TcpStream::connect(socks_addr).await?;

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

    connect(&mut stream, target, auth_info).await?;
    Ok(stream)
  } else {
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
  let mut socks_stream = match TcpStream::connect(&socks_addr).await {
    Ok(stream) => stream,
    Err(e) => {
      log::error!("Failed to connect to SOCKS4 proxy {}: {}", socks_addr, e);
      let mut response = Response::new(Full::new(Bytes::from(format!(
        "Failed to connect to SOCKS4 proxy: {}",
        e
      ))));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(response);
    }
  };

  // Resolve target host to IP (SOCKS4 requires IP addresses)
  let target_ip = match tokio::net::lookup_host((target_host, target_port)).await {
    Ok(mut addrs) => {
      if let Some(addr) = addrs.next() {
        match addr.ip() {
          std::net::IpAddr::V4(ipv4) => ipv4.octets(),
          std::net::IpAddr::V6(_) => {
            log::error!("SOCKS4 does not support IPv6");
            let mut response = Response::new(Full::new(Bytes::from(
              "SOCKS4 does not support IPv6 addresses",
            )));
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(response);
          }
        }
      } else {
        log::error!("Failed to resolve target host: {}", target_host);
        let mut response = Response::new(Full::new(Bytes::from(format!(
          "Failed to resolve target host: {}",
          target_host
        ))));
        *response.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(response);
      }
    }
    Err(e) => {
      log::error!("Failed to resolve target host {}: {}", target_host, e);
      let mut response = Response::new(Full::new(Bytes::from(format!(
        "Failed to resolve target host: {}",
        e
      ))));
      *response.status_mut() = StatusCode::BAD_GATEWAY;
      return Ok(response);
    }
  };

  // Build SOCKS4 CONNECT request
  let mut socks_request = vec![0x04, 0x01]; // SOCKS4, CONNECT
  socks_request.extend_from_slice(&target_port.to_be_bytes());
  socks_request.extend_from_slice(&target_ip);
  socks_request.push(0); // NULL terminator for userid

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
  if let Err(e) = socks_stream.read_exact(&mut socks_response).await {
    log::error!("Failed to read SOCKS4 response: {}", e);
    let mut response = Response::new(Full::new(Bytes::from(format!(
      "Failed to read SOCKS4 response: {}",
      e
    ))));
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    return Ok(response);
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

  // Read HTTP response
  let mut response_buffer = Vec::with_capacity(8192);
  let mut temp_buf = [0u8; 4096];
  let mut content_length: Option<usize> = None;
  let mut is_chunked = false;

  // Read until we have complete headers
  loop {
    match socks_stream.read(&mut temp_buf).await {
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
                match socks_stream.read(&mut temp_buf).await {
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
            let max_body_size = 10 * 1024 * 1024; // 10MB max
            while response_buffer.len() < max_body_size {
              match socks_stream.read(&mut temp_buf).await {
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
        log::error!("Error reading HTTP response from SOCKS4: {}", e);
        break;
      }
    }
  }

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

  log::error!(
    "DEBUG: Handling HTTP request: {} {} (host: {:?})",
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
  use reqwest::Client;

  let client_builder = Client::builder();
  let client = if should_bypass {
    client_builder.build().unwrap_or_default()
  } else if let Some(ref upstream) = upstream_url {
    if upstream == "DIRECT" {
      client_builder.build().unwrap_or_default()
    } else {
      // Build reqwest client with proxy
      match build_reqwest_client_with_proxy(upstream) {
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
    client_builder.build().unwrap_or_default()
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
      let body = response.bytes().await.unwrap_or_default();

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

fn build_reqwest_client_with_proxy(
  upstream_url: &str,
) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
  use reqwest::Proxy;

  let client_builder = reqwest::Client::builder();

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
      // For SOCKS5, reqwest supports it directly
      Proxy::all(upstream_url)?
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

  let mut peek_buffer = [0u8; 16];
  match stream.read(&mut peek_buffer).await {
    Ok(0) => {}
    Ok(n) => {
      let request_start_upper = String::from_utf8_lossy(&peek_buffer[..n.min(7)]).to_uppercase();
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

        let _ = handle_connect_from_buffer(
          stream,
          full_request,
          upstream_url,
          bypass_matcher,
          blocklist_matcher,
        )
        .await;
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
    Err(_) => {}
  }
}

pub async fn run_proxy_server(config: ProxyConfig) -> Result<(), Box<dyn std::error::Error>> {
  log::error!(
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

  log::error!(
    "Found config: id={}, port={:?}, upstream={}, profile_id={:?}",
    config.id,
    config.local_port,
    config.upstream_url,
    config.profile_id
  );

  log::error!("Starting proxy server for config id: {}", config.id);

  // Initialize traffic tracker with profile ID if available
  // This can now be called multiple times to update the tracker
  init_traffic_tracker(config.id.clone(), config.profile_id.clone());
  log::error!(
    "Traffic tracker initialized for proxy: {} (profile_id: {:?})",
    config.id,
    config.profile_id
  );

  // Verify tracker was initialized correctly
  if let Some(tracker) = crate::traffic_stats::get_traffic_tracker() {
    log::error!(
      "Tracker verified: proxy_id={}, profile_id={:?}",
      tracker.proxy_id,
      tracker.profile_id
    );
  } else {
    log::error!("WARNING: Tracker was not initialized!");
  }

  // Determine the bind address
  let bind_addr = SocketAddr::from(([127, 0, 0, 1], config.local_port.unwrap_or(0)));

  log::error!("Attempting to bind proxy server to {}", bind_addr);

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

  log::error!("Successfully bound to port {}", actual_port);

  // Update config with actual port and local_url
  let mut updated_config = config.clone();
  updated_config.local_port = Some(actual_port);
  updated_config.local_url = Some(format!("http://127.0.0.1:{}", actual_port));

  // Save the updated config
  log::error!(
    "Saving updated config with local_url={:?}",
    updated_config.local_url
  );
  if !crate::proxy_storage::update_proxy_config(&updated_config) {
    log::error!("Failed to update proxy config");
    return Err("Failed to update proxy config".into());
  }

  let upstream_url = if updated_config.upstream_url == "DIRECT" {
    None
  } else {
    Some(updated_config.upstream_url.clone())
  };

  log::error!("Proxy server bound to 127.0.0.1:{}", actual_port);
  log::error!(
    "Proxy server listening on 127.0.0.1:{} (ready to accept connections)",
    actual_port
  );
  log::error!("Proxy server entering accept loop - process should stay alive");

  // Start a background task to write lightweight session snapshots for real-time updates
  // These are much smaller than full stats and can be written frequently (~100 bytes every 2 seconds)
  if let Some(tracker) = get_traffic_tracker() {
    let tracker_clone = tracker.clone();
    tokio::spawn(async move {
      let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
      interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

      loop {
        interval.tick().await;
        // Write lightweight session snapshot (only current counters, ~100 bytes)
        if let Err(e) = tracker_clone.write_session_snapshot() {
          log::debug!("Failed to write session snapshot: {}", e);
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

  let bypass_matcher = BypassMatcher::new(&config.bypass_rules);
  let blocklist_matcher = if let Some(ref path) = config.blocklist_file {
    match BlocklistMatcher::from_file(path) {
      Ok(m) => m,
      Err(e) => {
        log::error!("[blocklist] Failed to load from {}: {}", path, e);
        BlocklistMatcher::new()
      }
    }
  } else {
    BlocklistMatcher::new()
  };

  // Keep the runtime alive with an infinite loop
  // This ensures the process doesn't exit even if there are no active connections
  loop {
    match listener.accept().await {
      Ok((stream, _peer_addr)) => {
        let upstream = upstream_url.clone();
        let matcher = bypass_matcher.clone();
        let blocker = blocklist_matcher.clone();
        tokio::task::spawn(async move {
          handle_proxy_connection(stream, upstream, matcher, blocker).await;
        });
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

  // Connect to target (directly or via upstream proxy).
  // Returns a BoxedAsyncStream so all upstream types (plain TCP, SOCKS,
  // Shadowsocks) share the same bidirectional-copy tunnel code below.
  let should_bypass = bypass_matcher.should_bypass(target_host);
  // Helper: configure outbound TCP to match browser TCP fingerprint
  let configure_tcp = |stream: &TcpStream| {
    let _ = stream.set_nodelay(true);
  };
  let target_stream: BoxedAsyncStream = match upstream_url.as_ref() {
    None => {
      let s = TcpStream::connect((target_host, target_port)).await?;
      configure_tcp(&s);
      Box::new(s)
    }
    Some(url) if url == "DIRECT" => {
      let s = TcpStream::connect((target_host, target_port)).await?;
      configure_tcp(&s);
      Box::new(s)
    }
    _ if should_bypass => {
      let s = TcpStream::connect((target_host, target_port)).await?;
      configure_tcp(&s);
      Box::new(s)
    }
    Some(upstream_url_str) => {
      let upstream = Url::parse(upstream_url_str)?;
      let scheme = upstream.scheme();

      match scheme {
        "http" | "https" => {
          let proxy_host = upstream.host_str().unwrap_or("127.0.0.1");
          let proxy_port = upstream.port().unwrap_or(8080);
          let mut proxy_stream = TcpStream::connect((proxy_host, proxy_port)).await?;
          configure_tcp(&proxy_stream);

          let mut connect_req = format!(
            "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n",
            target_host, target_port, target_host, target_port
          );

          if !upstream.username().is_empty() {
            use base64::{engine::general_purpose, Engine as _};
            let username = upstream.username();
            let password = upstream.password().unwrap_or("");
            let auth = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
            connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", auth));
          }

          connect_req.push_str("\r\n");

          proxy_stream.write_all(connect_req.as_bytes()).await?;

          let mut buffer = [0u8; 4096];
          let n = proxy_stream.read(&mut buffer).await?;
          let response = String::from_utf8_lossy(&buffer[..n]);

          if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
            return Err(format!("Upstream proxy CONNECT failed: {}", response).into());
          }

          Box::new(proxy_stream)
        }
        "socks4" | "socks5" => {
          let socks_host = upstream.host_str().unwrap_or("127.0.0.1");
          let socks_port = upstream.port().unwrap_or(1080);
          let socks_addr = format!("{}:{}", socks_host, socks_port);

          let username = upstream.username();
          let password = upstream.password().unwrap_or("");

          let stream = connect_via_socks(
            &socks_addr,
            target_host,
            target_port,
            scheme == "socks5",
            if !username.is_empty() {
              Some((username, password))
            } else {
              None
            },
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

          let stream = shadowsocks::relay::tcprelay::proxy_stream::ProxyClientStream::connect(
            context,
            &svr_cfg,
            target_addr,
          )
          .await
          .map_err(|e| format!("Shadowsocks connection failed: {e}"))?;

          Box::new(stream)
        }
        _ => {
          return Err(format!("Unsupported upstream proxy scheme: {}", scheme).into());
        }
      }
    }
  };

  // TCP_NODELAY is set per-stream where applicable (TcpStream paths).
  // For encrypted streams (Shadowsocks), the underlying TCP connection
  // is managed by the library and nodelay is handled internally.

  // Send 200 Connection Established response to client
  // CRITICAL: Must flush after writing to ensure response is sent before tunneling
  client_stream
    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
    .await?;
  client_stream.flush().await?;

  log::error!("DEBUG: Sent 200 Connection Established response, starting tunnel");

  // Now tunnel data bidirectionally with counting
  // Wrap streams to count bytes transferred
  let counting_client = CountingStream::new(client_stream);
  let counting_target = CountingStream::new(target_stream);

  // Get references for final stats
  let client_read_counter = counting_client.bytes_read.clone();
  let client_write_counter = counting_client.bytes_written.clone();
  let target_read_counter = counting_target.bytes_read.clone();
  let target_write_counter = counting_target.bytes_written.clone();

  // Split streams for bidirectional copying
  let (mut client_read, mut client_write) = tokio::io::split(counting_client);
  let (mut target_read, mut target_write) = tokio::io::split(counting_target);

  log::error!("DEBUG: Starting bidirectional tunnel");

  // Spawn two tasks to forward data in both directions
  let client_to_target = tokio::spawn(async move {
    let result = tokio::io::copy(&mut client_read, &mut target_write).await;
    match result {
      Ok(bytes) => {
        log::error!("DEBUG: Tunneled {} bytes from client->target", bytes);
      }
      Err(e) => {
        log::error!("Error forwarding client->target: {:?}", e);
      }
    }
  });

  let target_to_client = tokio::spawn(async move {
    let result = tokio::io::copy(&mut target_read, &mut client_write).await;
    match result {
      Ok(bytes) => {
        log::error!("DEBUG: Tunneled {} bytes from target->client", bytes);
      }
      Err(e) => {
        log::error!("Error forwarding target->client: {:?}", e);
      }
    }
  });

  // Wait for either direction to finish (connection closed)
  tokio::select! {
    _ = client_to_target => {
      log::error!("DEBUG: Client->target tunnel closed");
    }
    _ = target_to_client => {
      log::error!("DEBUG: Target->client tunnel closed");
    }
  }

  // Log final byte counts and update domain stats
  let final_sent =
    client_read_counter.load(Ordering::Relaxed) + target_write_counter.load(Ordering::Relaxed);
  let final_recv =
    target_read_counter.load(Ordering::Relaxed) + client_write_counter.load(Ordering::Relaxed);
  log::error!(
    "DEBUG: Tunnel closed - sent: {} bytes, received: {} bytes",
    final_sent,
    final_recv
  );

  // Update domain-specific byte counts now that tunnel is complete
  if let Some(tracker) = get_traffic_tracker() {
    tracker.update_domain_bytes(&domain, final_sent, final_recv);
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Write;

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
