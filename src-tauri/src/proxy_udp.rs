//! Whether a proxy can carry UDP, asked the way the browser would ask.
//!
//! This decides more than it looks like it does: WebRTC is UDP, so a profile
//! on a proxy without `UDP ASSOCIATE` either leaks WebRTC around the proxy or
//! loses it entirely. The answer therefore has to be a fact, not a guess —
//! hence three verdicts, with "unknown" reserved for everything the probe
//! could not establish.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::browser::ProxySettings;

/// How long the whole handshake gets. A proxy that cannot answer a three-byte
/// greeting and one request inside this is not going to carry a media stream.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

const SOCKS5: u8 = 0x05;
const AUTH_NONE: u8 = 0x00;
const AUTH_USERPASS: u8 = 0x02;
const AUTH_UNACCEPTABLE: u8 = 0xFF;
const CMD_UDP_ASSOCIATE: u8 = 0x03;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const REP_SUCCEEDED: u8 = 0x00;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UdpSupport {
  /// The proxy accepted `UDP ASSOCIATE`.
  Yes,
  /// The proxy cannot carry UDP: it refused the command, or its protocol has
  /// no way to carry a datagram at all.
  No,
  /// Not established. Never reported as `Yes`, and never as `No` either: an
  /// unreachable proxy has not proved anything.
  #[default]
  Unknown,
}

/// The verdict that follows from the protocol alone, before anything is
/// dialled. `None` means the protocol can carry UDP in principle and the
/// proxy itself has to be asked.
///
/// HTTP proxies answer `No` here rather than being probed: CONNECT builds a
/// TCP tunnel and the protocol has no datagram command to send.
pub fn udp_verdict_for_type(proxy_type: &str) -> Option<UdpSupport> {
  match proxy_type.trim().to_ascii_lowercase().as_str() {
    "socks5" | "socks5h" => None,
    // SOCKS4 and SOCKS4a define CONNECT and BIND only.
    "http" | "https" | "httpstls" | "socks4" | "socks4a" => Some(UdpSupport::No),
    // Shadowsocks and VLESS can carry UDP, but whether this endpoint does is
    // not something a SOCKS handshake can answer.
    _ => Some(UdpSupport::Unknown),
  }
}

/// Map a SOCKS5 reply code onto a verdict.
///
/// Only an accepted association counts as `Yes`. Every other reply is a
/// refusal the proxy stated in answer to the exact request, which is a real
/// `No`. That deliberately includes codes RFC 1928 never defined: a residential
/// gateway tested here answers `UDP ASSOCIATE` with `0xFF`, and calling that
/// "unknown" would leave the column blank for the proxies it matters most for.
///
/// `Unknown` belongs to the cases where the proxy was never actually asked —
/// unreachable, timed out, refused the authentication, or answered something
/// that is not a SOCKS5 reply — and those are decided by the caller before a
/// reply code ever gets here.
pub fn udp_verdict_from_socks_reply(reply: u8) -> UdpSupport {
  if reply == REP_SUCCEEDED {
    UdpSupport::Yes
  } else {
    UdpSupport::No
  }
}

/// Ask a proxy whether it carries UDP.
///
/// The probe dials the proxy exactly as a launch would — the same host, the
/// same port, the same credentials — and asks for an association it never
/// uses. No datagram is sent and no third-party host is named, so nothing
/// about this check reaches anywhere the browser would not already go.
pub async fn probe_udp_support(settings: &ProxySettings) -> UdpSupport {
  if let Some(verdict) = udp_verdict_for_type(&settings.proxy_type) {
    return verdict;
  }

  match tokio::time::timeout(PROBE_TIMEOUT, socks5_udp_associate(settings)).await {
    Ok(Ok(verdict)) => verdict,
    Ok(Err(e)) => {
      log::debug!(
        "UDP probe of {}:{} could not complete: {e}",
        settings.host,
        settings.port
      );
      UdpSupport::Unknown
    }
    Err(_) => {
      log::debug!("UDP probe of {}:{} timed out", settings.host, settings.port);
      UdpSupport::Unknown
    }
  }
}

async fn socks5_udp_associate(settings: &ProxySettings) -> std::io::Result<UdpSupport> {
  let mut stream = tokio::net::TcpStream::connect((settings.host.as_str(), settings.port)).await?;

  let credentials = settings
    .username
    .as_deref()
    .filter(|user| !user.is_empty())
    .map(|user| (user, settings.password.as_deref().unwrap_or_default()));

  let greeting: Vec<u8> = match credentials {
    Some(_) => vec![SOCKS5, 2, AUTH_NONE, AUTH_USERPASS],
    None => vec![SOCKS5, 1, AUTH_NONE],
  };
  stream.write_all(&greeting).await?;

  let mut selection = [0u8; 2];
  stream.read_exact(&mut selection).await?;
  if selection[0] != SOCKS5 {
    return Ok(UdpSupport::Unknown);
  }
  match selection[1] {
    AUTH_NONE => {}
    AUTH_USERPASS => {
      let Some((user, password)) = credentials else {
        return Ok(UdpSupport::Unknown);
      };
      if !authenticate(&mut stream, user, password).await? {
        return Ok(UdpSupport::Unknown);
      }
    }
    AUTH_UNACCEPTABLE => return Ok(UdpSupport::Unknown),
    _ => return Ok(UdpSupport::Unknown),
  }

  // An all-zero address is what a client sends when it does not yet know the
  // address it will send datagrams from, which is exactly this case: the
  // association is requested and then dropped.
  stream
    .write_all(&[SOCKS5, CMD_UDP_ASSOCIATE, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
    .await?;

  // Only the version and the reply code are read up front. RFC 1928 says a
  // reply carries a bound address as well, but a refusing server does not
  // always send one: the residential gateway this was tested against answers
  // `05 FF` and closes. Demanding the full four-byte header there turns a
  // stated refusal into a read error, and the verdict into "unknown".
  let mut head = [0u8; 2];
  stream.read_exact(&mut head).await?;
  if head[0] != SOCKS5 {
    return Ok(UdpSupport::Unknown);
  }
  let verdict = udp_verdict_from_socks_reply(head[1]);

  if verdict == UdpSupport::Yes {
    // An accepted association does carry the address to send datagrams to.
    // Nothing here uses it, but reading it leaves the socket drained rather
    // than closing under a server that is still writing.
    let mut tail = [0u8; 2];
    if stream.read_exact(&mut tail).await.is_ok() {
      let _ = drain_bound_address(&mut stream, tail[1]).await;
    }
  }
  Ok(verdict)
}

async fn authenticate(
  stream: &mut tokio::net::TcpStream,
  user: &str,
  password: &str,
) -> std::io::Result<bool> {
  if user.len() > 255 || password.len() > 255 {
    return Ok(false);
  }
  let mut request = Vec::with_capacity(3 + user.len() + password.len());
  request.push(0x01);
  request.push(user.len() as u8);
  request.extend_from_slice(user.as_bytes());
  request.push(password.len() as u8);
  request.extend_from_slice(password.as_bytes());
  stream.write_all(&request).await?;

  let mut reply = [0u8; 2];
  stream.read_exact(&mut reply).await?;
  Ok(reply[1] == 0x00)
}

async fn drain_bound_address(
  stream: &mut tokio::net::TcpStream,
  address_type: u8,
) -> std::io::Result<()> {
  let length = match address_type {
    ATYP_IPV4 => 4,
    ATYP_IPV6 => 16,
    ATYP_DOMAIN => {
      let mut len = [0u8; 1];
      stream.read_exact(&mut len).await?;
      len[0] as usize
    }
    _ => return Ok(()),
  };
  let mut scratch = vec![0u8; length + 2];
  stream.read_exact(&mut scratch).await?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  /// RFC 1928's "command not supported". Production no longer needs to name
  /// it — every non-zero reply is a refusal — but a test server has to send
  /// something a real proxy would send.
  const REP_CMD_NOT_SUPPORTED: u8 = 0x07;

  #[test]
  fn an_http_proxy_is_answered_without_ever_being_dialled() {
    for proxy_type in ["http", "HTTP", "https", "httpstls", "socks4", "socks4a"] {
      assert_eq!(
        udp_verdict_for_type(proxy_type),
        Some(UdpSupport::No),
        "{proxy_type}"
      );
    }
  }

  #[test]
  fn socks5_is_the_only_type_that_gets_probed() {
    assert_eq!(udp_verdict_for_type("socks5"), None);
    assert_eq!(udp_verdict_for_type("SOCKS5"), None);
    assert_eq!(udp_verdict_for_type("socks5h"), None);
  }

  #[test]
  fn a_protocol_no_socks_handshake_can_answer_stays_unknown() {
    for proxy_type in ["ss", "vless", "", "something-new"] {
      assert_eq!(
        udp_verdict_for_type(proxy_type),
        Some(UdpSupport::Unknown),
        "{proxy_type}"
      );
    }
  }

  #[test]
  fn only_an_accepted_association_counts_as_yes() {
    assert_eq!(udp_verdict_from_socks_reply(0x00), UdpSupport::Yes);
    // Every stated refusal is a refusal, including 0xFF, which is not in
    // RFC 1928 but is what a real residential gateway answers.
    for refused in [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xFF] {
      assert_eq!(
        udp_verdict_from_socks_reply(refused),
        UdpSupport::No,
        "reply {refused:#04x}"
      );
    }
  }

  #[test]
  fn the_default_verdict_is_unknown_so_an_old_receipt_never_claims_support() {
    assert_eq!(UdpSupport::default(), UdpSupport::Unknown);
    assert_eq!(serde_json::to_string(&UdpSupport::Yes).unwrap(), "\"yes\"");
    assert_eq!(
      serde_json::from_str::<UdpSupport>("\"unknown\"").unwrap(),
      UdpSupport::Unknown
    );
  }

  #[tokio::test]
  async fn a_proxy_that_cannot_be_reached_reports_unknown_not_no() {
    let settings = ProxySettings {
      proxy_type: "socks5".to_string(),
      // Discard port on loopback: nothing is listening, so the dial fails.
      host: "127.0.0.1".to_string(),
      port: 9,
      username: None,
      password: None,
      vless_uri: None,
    };
    assert_eq!(probe_udp_support(&settings).await, UdpSupport::Unknown);
  }

  #[tokio::test]
  async fn a_socks5_server_that_accepts_the_association_reports_yes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut greeting = [0u8; 3];
      stream.read_exact(&mut greeting).await.unwrap();
      stream.write_all(&[SOCKS5, AUTH_NONE]).await.unwrap();
      let mut request = [0u8; 10];
      stream.read_exact(&mut request).await.unwrap();
      assert_eq!(request[1], CMD_UDP_ASSOCIATE);
      stream
        .write_all(&[
          SOCKS5,
          REP_SUCCEEDED,
          0x00,
          ATYP_IPV4,
          127,
          0,
          0,
          1,
          0x11,
          0x11,
        ])
        .await
        .unwrap();
    });

    let settings = ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "127.0.0.1".to_string(),
      port,
      username: None,
      password: None,
      vless_uri: None,
    };
    assert_eq!(probe_udp_support(&settings).await, UdpSupport::Yes);
  }

  /// A refusal that arrives as two bytes and a closed socket, which is what a
  /// real residential gateway sends. Reading a full reply header here would
  /// hit end-of-file and report "unknown" for a proxy that plainly said no.
  #[tokio::test]
  async fn a_truncated_refusal_is_still_a_refusal() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut greeting = [0u8; 3];
      stream.read_exact(&mut greeting).await.unwrap();
      stream.write_all(&[SOCKS5, AUTH_NONE]).await.unwrap();
      let mut request = [0u8; 10];
      stream.read_exact(&mut request).await.unwrap();
      stream.write_all(&[SOCKS5, 0xFF]).await.unwrap();
    });

    let settings = ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "127.0.0.1".to_string(),
      port,
      username: None,
      password: None,
      vless_uri: None,
    };
    assert_eq!(probe_udp_support(&settings).await, UdpSupport::No);
  }

  /// A server that will not accept the offered authentication never got asked
  /// about UDP, so the answer is "unknown", not "no".
  #[tokio::test]
  async fn a_refused_handshake_reports_unknown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut greeting = [0u8; 3];
      stream.read_exact(&mut greeting).await.unwrap();
      stream
        .write_all(&[SOCKS5, AUTH_UNACCEPTABLE])
        .await
        .unwrap();
    });

    let settings = ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "127.0.0.1".to_string(),
      port,
      username: None,
      password: None,
      vless_uri: None,
    };
    assert_eq!(probe_udp_support(&settings).await, UdpSupport::Unknown);
  }

  #[tokio::test]
  async fn a_socks5_server_that_refuses_the_command_reports_no() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut greeting = [0u8; 4];
      stream.read_exact(&mut greeting).await.unwrap();
      stream.write_all(&[SOCKS5, AUTH_USERPASS]).await.unwrap();
      let mut header = [0u8; 2];
      stream.read_exact(&mut header).await.unwrap();
      let mut user = vec![0u8; header[1] as usize];
      stream.read_exact(&mut user).await.unwrap();
      let mut password_len = [0u8; 1];
      stream.read_exact(&mut password_len).await.unwrap();
      let mut password = vec![0u8; password_len[0] as usize];
      stream.read_exact(&mut password).await.unwrap();
      assert_eq!(user, b"probe-user");
      stream.write_all(&[0x01, 0x00]).await.unwrap();
      let mut request = [0u8; 10];
      stream.read_exact(&mut request).await.unwrap();
      stream
        .write_all(&[
          SOCKS5,
          REP_CMD_NOT_SUPPORTED,
          0x00,
          ATYP_IPV4,
          0,
          0,
          0,
          0,
          0,
          0,
        ])
        .await
        .unwrap();
    });

    let settings = ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "127.0.0.1".to_string(),
      port,
      username: Some("probe-user".to_string()),
      password: Some("probe-pass".to_string()),
      vless_uri: None,
    };
    assert_eq!(probe_udp_support(&settings).await, UdpSupport::No);
  }
}
