use regex_lite::Regex;
use std::sync::LazyLock;

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]{1,20}://[^\s<>"']+"#).expect("valid URL regex")
});
static PRIVATE_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?is)-----BEGIN [^-\r\n]*PRIVATE KEY-----.*?-----END [^-\r\n]*PRIVATE KEY-----")
    .expect("valid private-key regex")
});
/// Every HTTP auth scheme that carries its credential as a single token after
/// the scheme name, not just `Bearer`. SECRET_RE cannot reach these: its value
/// class stops at the space between the scheme and the credential, so a
/// `Basic`/`NTLM` blob used to survive into an exported log verbatim. Digest's
/// quoted-parameter form (`response="..."`) is out of scope.
static AUTH_SCHEME_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?i)\b(Bearer|Basic|Token|Digest|Negotiate|NTLM)\s+[A-Za-z0-9._~+/=-]+")
    .expect("valid auth-scheme regex")
});
static SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(
    r"(?i)\b(api[_-]?key|authorization|password|passwd|private[_-]?key|proxy[_-]?(password|username)|refresh[_-]?token|secret|token|username)\b\s*[:=]\s*[^\s,;]+",
  )
  .expect("valid secret regex")
});
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").expect("valid email regex")
});
static UNIX_HOME_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"/(Users|home)/[^/\s]+").expect("valid Unix home regex"));
static WINDOWS_HOME_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?i)\b[A-Z]:\\Users\\[^\\\s]+").expect("valid Windows home regex"));
static IPV4_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"\b([0-9]{1,3}\.){3}[0-9]{1,3}\b").expect("valid IPv4 regex"));
static DOMAIN_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?i)\b([a-z0-9-]+\.)+[a-z]{2,}\b").expect("valid domain regex"));
static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b")
    .expect("valid UUID regex")
});

/// A caller-supplied string as it may appear in a log line: control
/// characters, a newline above all, are shown escaped, so no request can
/// forge a second log entry or hide the end of the real one.
pub struct Plain<'a>(pub &'a str);

impl std::fmt::Display for Plain<'_> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use std::fmt::Write;
    for c in self.0.chars() {
      if c.is_control() {
        for escaped in c.escape_default() {
          f.write_char(escaped)?;
        }
      } else {
        f.write_char(c)?;
      }
    }
    Ok(())
  }
}

/// The first characters of an identifier: enough to match log lines up by
/// eye, and not the whole value, which for a session is a bearer of sorts.
pub struct ShortId<'a>(pub &'a str);

impl std::fmt::Display for ShortId<'_> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    const SHOWN: usize = 8;
    let shown: String = Plain(self.0).to_string().chars().take(SHOWN).collect();
    f.write_str(&shown)?;
    if self.0.chars().count() > SHOWN {
      f.write_str("\u{2026}")?;
    }
    Ok(())
  }
}

pub fn url_label(value: &str) -> String {
  url::Url::parse(value)
    .map(|parsed| format!("{}://<redacted>", parsed.scheme()))
    .unwrap_or_else(|_| "<redacted-url>".to_string())
}

pub fn text(value: &str) -> String {
  let redacted = PRIVATE_KEY_RE.replace_all(value, "<redacted-private-key>");
  let redacted = URL_RE.replace_all(&redacted, "<redacted-url>");
  // Must stay ahead of SECRET_RE, which would otherwise consume
  // `Authorization: Basic` and leave the credential with no scheme to match.
  let redacted = AUTH_SCHEME_RE.replace_all(&redacted, "${1} <redacted-secret>");
  let redacted = SECRET_RE.replace_all(&redacted, "<redacted-secret>");
  let redacted = EMAIL_RE.replace_all(&redacted, "<redacted-email>");
  let redacted = UNIX_HOME_RE.replace_all(&redacted, "/<redacted-home>");
  let redacted = WINDOWS_HOME_RE.replace_all(&redacted, "<redacted-home>");
  let redacted = IPV4_RE.replace_all(&redacted, "<redacted-ip>");
  let redacted = DOMAIN_RE.replace_all(&redacted, "<redacted-domain>");
  UUID_RE
    .replace_all(&redacted, "<redacted-identifier>")
    .into_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn plain_escapes_every_control_character() {
    assert_eq!(Plain("wayfern").to_string(), "wayfern");
    assert_eq!(
      Plain("1.0\nINFO forged line\r\t").to_string(),
      "1.0\\nINFO forged line\\r\\t"
    );
  }

  #[test]
  fn short_id_keeps_a_prefix_and_marks_the_cut() {
    assert_eq!(ShortId("abcdef").to_string(), "abcdef");
    assert_eq!(ShortId("0123456789abcdef").to_string(), "01234567\u{2026}");
    assert_eq!(ShortId("ab\ncd").to_string(), "ab\\ncd");
  }

  #[test]
  fn redacts_sensitive_log_content() {
    let input = format!(
      concat!(
        "URL https://user:pass@example.com/callback?code=private\n",
        "Authorization: Bearer secret-token\n",
        "password=hunter2\n",
        "user@example.com /Users/alice/Library C:\\Users\\alice\\AppData\n",
        "exit 203.0.113.42\n",
        "-----BEGIN {0} KEY-----\nprivate-material\n-----END {0} KEY-----\n",
      ),
      "PRIVATE"
    );
    let output = text(&input);
    for sensitive in [
      "user:pass",
      "example.com",
      "private-material",
      "secret-token",
      "hunter2",
      "user@example.com",
      "alice",
      "203.0.113.42",
    ] {
      assert!(!output.contains(sensitive), "log output leaked {sensitive}");
    }
  }

  #[test]
  fn redacts_non_bearer_authorization_credentials() {
    let headers = [
      ("Authorization: Basic ", "dXNlcjpwYXNzd29yZA=="),
      ("Proxy-Authorization: Basic ", "cHJveHk6c2VjcmV0"),
      ("Authorization: Token ", "gh_example_credential"),
      ("authorization: bearer ", "lower-case-credential"),
      ("WWW-Authenticate: NTLM ", "TlRMTVNTUAAB"),
    ];
    for (header, credential) in headers {
      let output = text(&format!("{header}{credential}"));
      assert!(
        !output.contains(credential),
        "log output leaked {credential}"
      );
    }

    // The scheme survives wherever the header name is not itself redacted, so a
    // log still says which kind of authentication was in play.
    assert!(text("WWW-Authenticate: NTLM TlRMTVNTUAAB").contains("NTLM"));
  }

  #[test]
  fn url_labels_retain_only_the_scheme() {
    assert_eq!(
      url_label("https://user:pass@example.com/path?token=value"),
      "https://<redacted>"
    );
    assert_eq!(url_label("not a URL"), "<redacted-url>");
  }
}
