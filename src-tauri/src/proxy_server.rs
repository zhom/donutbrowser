use crate::proxy_storage::ProxyConfig;
use crate::traffic_stats::{get_traffic_tracker, init_traffic_tracker};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use url::Url;

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
) -> Result<Response<Full<Bytes>>, Infallible> {
  // Handle CONNECT method for HTTPS tunneling
  if req.method() == Method::CONNECT {
    return handle_connect(req, upstream_url).await;
  }

  // Handle regular HTTP requests
  handle_http(req, upstream_url).await
}

async fn handle_connect(
  req: Request<hyper::body::Incoming>,
  upstream_url: Option<String>,
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

    // If no upstream proxy, connect directly
    if upstream_url.is_none()
      || upstream_url
        .as_ref()
        .map(|s| s == "DIRECT")
        .unwrap_or(false)
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

async fn handle_http(
  req: Request<hyper::body::Incoming>,
  upstream_url: Option<String>,
) -> Result<Response<Full<Bytes>>, Infallible> {
  // Extract domain for traffic tracking
  let domain = req
    .uri()
    .host()
    .map(|h| h.to_string())
    .unwrap_or_else(|| "unknown".to_string());

  log::error!(
    "DEBUG: Handling HTTP request: {} {} (host: {:?})",
    req.method(),
    req.uri(),
    req.uri().host()
  );

  // Check if we need to handle SOCKS4 manually (reqwest doesn't support it)
  if let Some(ref upstream) = upstream_url {
    if upstream != "DIRECT" {
      if let Ok(url) = Url::parse(upstream) {
        if url.scheme() == "socks4" {
          // Handle SOCKS4 manually for HTTP requests
          return handle_http_via_socks4(req, upstream).await;
        }
      }
    }
  }

  // Use reqwest for HTTP/HTTPS/SOCKS5 proxies
  use reqwest::Client;

  let client_builder = Client::builder();
  let client = if let Some(ref upstream) = upstream_url {
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

  // Bind to the port
  let listener = TcpListener::bind(bind_addr).await?;
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
      if let Some(tracker) = get_traffic_tracker() {
        let (sent, recv, requests) = tracker.get_snapshot();
        let current_bytes = sent + recv;
        let time_since_activity = last_activity_time.elapsed();
        let time_since_flush = last_flush_time.elapsed();
        let has_traffic = current_bytes > 0 || requests > 0;

        // Determine flush frequency based on activity
        // When active: flush every 5 seconds
        // When idle: flush every 30 seconds
        let desired_interval_secs =
          if has_traffic || time_since_activity < std::time::Duration::from_secs(30) {
            5u64
          } else {
            30u64
          };

        // Update interval if needed
        if desired_interval_secs != current_interval_secs {
          current_interval_secs = desired_interval_secs;
          interval = tokio::time::interval(tokio::time::Duration::from_secs(desired_interval_secs));
        }

        // Only flush if enough time has passed since last flush
        let flush_interval = std::time::Duration::from_secs(desired_interval_secs);
        let should_flush = time_since_flush >= flush_interval;

        if should_flush {
          match tracker.flush_to_disk() {
            Ok(Some((sent, recv))) => {
              // Successful flush with data
              last_flush_time = std::time::Instant::now();
              if sent > 0 || recv > 0 {
                last_activity_time = std::time::Instant::now();
              }
            }
            Ok(None) => {
              // No data to flush - this is normal
              last_flush_time = std::time::Instant::now();
            }
            Err(e) => {
              log::error!("Failed to flush traffic stats: {}", e);
              // Don't update flush time on error - retry sooner
            }
          }
        }
      }
    }
  });

  // Keep the runtime alive with an infinite loop
  // This ensures the process doesn't exit even if there are no active connections
  loop {
    match listener.accept().await {
      Ok((mut stream, peer_addr)) => {
        // Enable TCP_NODELAY to ensure small packets are sent immediately
        // This is critical for CONNECT responses to be sent before tunneling begins
        let _ = stream.set_nodelay(true);
        log::error!("DEBUG: Accepted connection from {:?}", peer_addr);

        let upstream = upstream_url.clone();

        tokio::task::spawn(async move {
          // Read first bytes to detect CONNECT requests
          // CONNECT requests need special handling for tunneling
          // Use a larger buffer to ensure we can detect CONNECT even with partial reads
          let mut peek_buffer = [0u8; 16];
          match stream.read(&mut peek_buffer).await {
            Ok(0) => {
              log::error!("DEBUG: Connection closed immediately (0 bytes read)");
            }
            Ok(n) => {
              // Check if this looks like a CONNECT request
              // Be more lenient - check if the first bytes match "CONNECT" (case-insensitive)
              let request_start_upper =
                String::from_utf8_lossy(&peek_buffer[..n.min(7)]).to_uppercase();
              let is_connect = request_start_upper.starts_with("CONNECT");

              log::error!(
                "DEBUG: Read {} bytes, starts with: {:?}, is_connect: {}",
                n,
                String::from_utf8_lossy(&peek_buffer[..n.min(20)]),
                is_connect
              );

              if is_connect {
                // Handle CONNECT request manually for tunneling
                let mut full_request = Vec::with_capacity(4096);
                full_request.extend_from_slice(&peek_buffer[..n]);

                // Read the rest of the CONNECT request until we have the full headers
                // CONNECT requests end with \r\n\r\n (or \n\n)
                let mut remaining = [0u8; 4096];
                let mut total_read = n;
                let max_reads = 100; // Prevent infinite loop
                let mut reads = 0;

                loop {
                  if reads >= max_reads {
                    log::error!("DEBUG: Max reads reached, breaking");
                    break;
                  }

                  match stream.read(&mut remaining).await {
                    Ok(0) => {
                      // Connection closed, but we might have a complete request
                      if full_request.ends_with(b"\r\n\r\n") || full_request.ends_with(b"\n\n") {
                        break;
                      }
                      // If we have some data, try to process it anyway
                      if total_read > 0 {
                        break;
                      }
                      return; // No data at all
                    }
                    Ok(m) => {
                      reads += 1;
                      total_read += m;
                      full_request.extend_from_slice(&remaining[..m]);

                      // Check if we have complete headers
                      if full_request.ends_with(b"\r\n\r\n") || full_request.ends_with(b"\n\n") {
                        break;
                      }

                      // Also check if we have enough to parse (at least "CONNECT host:port HTTP/1.x")
                      if total_read >= 20 {
                        // Check if we have a newline that might indicate end of request line
                        if let Some(pos) = full_request.iter().position(|&b| b == b'\n') {
                          if pos < full_request.len() - 1 {
                            // We have at least the request line, check if we have headers
                            let request_str = String::from_utf8_lossy(&full_request);
                            if request_str.contains("\r\n\r\n") || request_str.contains("\n\n") {
                              break;
                            }
                          }
                        }
                      }
                    }
                    Err(e) => {
                      log::error!("DEBUG: Error reading CONNECT request: {:?}", e);
                      // If we have some data, try to process it
                      if total_read > 0 {
                        break;
                      }
                      return;
                    }
                  }
                }

                // Handle CONNECT manually
                log::error!(
                  "DEBUG: Handling CONNECT manually for: {}",
                  String::from_utf8_lossy(&full_request[..full_request.len().min(200)])
                );
                if let Err(e) = handle_connect_from_buffer(stream, full_request, upstream).await {
                  log::error!("Error handling CONNECT request: {:?}", e);
                } else {
                  log::error!("DEBUG: CONNECT handled successfully");
                }
                return;
              }

              // Not CONNECT (or partial read) - reconstruct stream with consumed bytes prepended
              // This is critical: we MUST prepend any bytes we consumed, even if < 7 bytes
              log::error!(
                "DEBUG: Non-CONNECT request, first {} bytes: {:?}",
                n,
                String::from_utf8_lossy(&peek_buffer[..n.min(50)])
              );
              let prepended_bytes = peek_buffer[..n].to_vec();
              let prepended_reader = PrependReader {
                prepended: prepended_bytes,
                prepended_pos: 0,
                inner: stream,
              };
              let io = TokioIo::new(prepended_reader);
              let service = service_fn(move |req| handle_request(req, upstream.clone()));

              if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                log::error!("Error serving connection: {:?}", err);
              }
            }
            Err(e) => {
              log::error!("Error reading from connection: {:?}", e);
            }
          }
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

  // Record domain access in traffic tracker
  let domain = target_host.to_string();
  if let Some(tracker) = get_traffic_tracker() {
    tracker.record_request(&domain, 0, 0);
  }

  // Connect to target (directly or via upstream proxy)
  let target_stream = match upstream_url.as_ref() {
    None => {
      // Direct connection
      TcpStream::connect((target_host, target_port)).await?
    }
    Some(url) if url == "DIRECT" => {
      // Direct connection
      TcpStream::connect((target_host, target_port)).await?
    }
    Some(upstream_url_str) => {
      // Connect via upstream proxy
      let upstream = Url::parse(upstream_url_str)?;
      let scheme = upstream.scheme();

      match scheme {
        "http" | "https" => {
          // Connect via HTTP/HTTPS proxy CONNECT
          // Note: HTTPS proxy URLs still use HTTP CONNECT method (CONNECT is always HTTP-based)
          // For HTTPS proxies, reqwest handles TLS automatically in handle_http
          // For manual CONNECT here, we use plain TCP - HTTPS proxy CONNECT typically works over plain TCP
          let proxy_host = upstream.host_str().unwrap_or("127.0.0.1");
          let proxy_port = upstream.port().unwrap_or(8080);
          let mut proxy_stream = TcpStream::connect((proxy_host, proxy_port)).await?;

          // Add authentication if provided
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

          // Send CONNECT request to upstream proxy
          proxy_stream.write_all(connect_req.as_bytes()).await?;

          // Read response
          let mut buffer = [0u8; 4096];
          let n = proxy_stream.read(&mut buffer).await?;
          let response = String::from_utf8_lossy(&buffer[..n]);

          if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
            return Err(format!("Upstream proxy CONNECT failed: {}", response).into());
          }

          proxy_stream
        }
        "socks4" | "socks5" => {
          // Connect via SOCKS proxy
          let socks_host = upstream.host_str().unwrap_or("127.0.0.1");
          let socks_port = upstream.port().unwrap_or(1080);
          let socks_addr = format!("{}:{}", socks_host, socks_port);

          let username = upstream.username();
          let password = upstream.password().unwrap_or("");

          connect_via_socks(
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
          .await?
        }
        _ => {
          return Err(format!("Unsupported upstream proxy scheme: {}", scheme).into());
        }
      }
    }
  };

  // Enable TCP_NODELAY on target stream for immediate data transfer
  let _ = target_stream.set_nodelay(true);

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
