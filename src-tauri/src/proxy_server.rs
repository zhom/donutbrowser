use crate::proxy_storage::ProxyConfig;
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
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use url::Url;

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

async fn handle_http(
  req: Request<hyper::body::Incoming>,
  upstream_url: Option<String>,
) -> Result<Response<Full<Bytes>>, Infallible> {
  // Use reqwest for all HTTP requests as it handles proxies better
  // This is faster and more reliable than trying to use hyper-proxy with version conflicts
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
      Proxy::http(upstream_url)?
    }
    "socks5" => {
      // For SOCKS5, reqwest supports it directly
      Proxy::all(upstream_url)?
    }
    "socks4" => {
      // SOCKS4 is not directly supported by reqwest, would need custom handling
      return Err("SOCKS4 not supported for HTTP requests via reqwest".into());
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
    "Found config: id={}, port={:?}, upstream={}",
    config.id,
    config.local_port,
    config.upstream_url
  );

  log::error!("Starting proxy server for config id: {}", config.id);

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

  // Keep the runtime alive with an infinite loop
  // This ensures the process doesn't exit even if there are no active connections
  loop {
    match listener.accept().await {
      Ok((mut stream, _)) => {
        let upstream = upstream_url.clone();

        tokio::task::spawn(async move {
          // Read first bytes to detect CONNECT requests
          // CONNECT requests need special handling for tunneling
          let mut peek_buffer = [0u8; 8];
          match stream.read(&mut peek_buffer).await {
            Ok(n) if n >= 7 => {
              let request_start = String::from_utf8_lossy(&peek_buffer[..n.min(7)]);
              if request_start.starts_with("CONNECT") {
                // Handle CONNECT request manually for tunneling
                let mut full_request = Vec::with_capacity(4096);
                full_request.extend_from_slice(&peek_buffer[..n]);

                // Read the rest of the CONNECT request
                let mut remaining = [0u8; 4096];
                loop {
                  match stream.read(&mut remaining).await {
                    Ok(0) => break,
                    Ok(m) => {
                      full_request.extend_from_slice(&remaining[..m]);
                      if full_request.ends_with(b"\r\n\r\n") || full_request.ends_with(b"\n\n") {
                        break;
                      }
                    }
                    Err(_) => break,
                  }
                }

                // Handle CONNECT manually
                log::error!(
                  "DEBUG: Handling CONNECT manually for: {}",
                  String::from_utf8_lossy(&full_request[..full_request.len().min(100)])
                );
                if let Err(e) = handle_connect_from_buffer(stream, full_request, upstream).await {
                  log::error!("Error handling CONNECT request: {:?}", e);
                } else {
                  log::error!("DEBUG: CONNECT handled successfully");
                }
                return;
              }
              // Not CONNECT - reconstruct stream with consumed bytes prepended
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
              return;
            }
            _ => {}
          }

          // For non-CONNECT requests, use hyper's HTTP handling
          let io = TokioIo::new(stream);
          let service = service_fn(move |req| handle_request(req, upstream.clone()));

          if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
            log::error!("Error serving connection: {:?}", err);
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

  // Connect to target (directly or via upstream proxy)
  let target_stream = if upstream_url.is_none()
    || upstream_url
      .as_ref()
      .map(|s| s == "DIRECT")
      .unwrap_or(false)
  {
    // Direct connection
    TcpStream::connect((target_host, target_port)).await?
  } else {
    // Connect via upstream proxy
    let upstream = Url::parse(upstream_url.as_ref().unwrap())?;
    let scheme = upstream.scheme();

    match scheme {
      "http" | "https" => {
        // Connect via HTTP proxy CONNECT
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
  };

  // Send 200 Connection Established response to client
  // CRITICAL: Must flush after writing to ensure response is sent before tunneling
  client_stream
    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
    .await?;
  client_stream.flush().await?;

  log::error!("DEBUG: Sent 200 Connection Established response, starting tunnel");

  // Now tunnel data bidirectionally
  // Split streams for bidirectional copying
  let (mut client_read, mut client_write) = tokio::io::split(client_stream);
  let (mut target_read, mut target_write) = tokio::io::split(target_stream);

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

  Ok(())
}
