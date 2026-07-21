use regex_lite::Regex;
use std::sync::LazyLock;

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]{1,20}://[^\s<>"']+"#).expect("valid URL regex")
});
static PRIVATE_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?is)-----BEGIN [^-\r\n]*PRIVATE KEY-----.*?-----END [^-\r\n]*PRIVATE KEY-----")
    .expect("valid private-key regex")
});
static BEARER_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+").expect("valid bearer regex"));
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

pub fn url_label(value: &str) -> String {
  url::Url::parse(value)
    .map(|parsed| format!("{}://<redacted>", parsed.scheme()))
    .unwrap_or_else(|_| "<redacted-url>".to_string())
}

pub fn text(value: &str) -> String {
  let redacted = PRIVATE_KEY_RE.replace_all(value, "<redacted-private-key>");
  let redacted = URL_RE.replace_all(&redacted, "<redacted-url>");
  let redacted = BEARER_RE.replace_all(&redacted, "Bearer <redacted-secret>");
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
  fn url_labels_retain_only_the_scheme() {
    assert_eq!(
      url_label("https://user:pass@example.com/path?token=value"),
      "https://<redacted>"
    );
    assert_eq!(url_label("not a URL"), "<redacted-url>");
  }
}
