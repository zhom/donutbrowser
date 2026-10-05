use thiserror::Error;

pub type XrayResult<T> = Result<T, XrayError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XrayError {
  #[error("invalid share link")]
  InvalidUri,
  #[error("share link scheme must be vless, vmess, trojan, hysteria2 or hy2")]
  UnsupportedScheme,
  #[error("missing required field: {0}")]
  MissingField(&'static str),
  #[error("invalid field: {field} ({reason})")]
  InvalidField {
    field: &'static str,
    reason: &'static str,
  },
  #[error("unsupported query parameter: {0}")]
  UnsupportedParameter(String),
  #[error("duplicate query parameter: {0}")]
  DuplicateParameter(String),
  #[error("unsupported value for {field}; expected {expected}")]
  UnsupportedValue {
    field: &'static str,
    expected: &'static str,
  },
  #[error("failed to serialize Xray client configuration")]
  Serialization,
}

impl XrayError {
  /// A stable, translatable identifier for *why* a share link was rejected.
  ///
  /// Most rejections are "your setup is a kind we do not support", not "you
  /// made a typo". The frontend turns these into a sentence naming the
  /// unsupported part; without them every rejection reads as a malformed link
  /// and a user with a working mKCP or port-hopping server has no idea why it
  /// failed.
  pub fn reason_code(&self) -> &'static str {
    match self {
      Self::UnsupportedScheme => "scheme",
      Self::UnsupportedValue { field, .. } | Self::InvalidField { field, .. } => match *field {
        "security" => "security",
        "flow" => "flow",
        "type" | "mode" => "transport",
        "encryption" => "encryption",
        "headerType" => "headerType",
        "fp" => "fingerprint",
        "obfs" => "obfs",
        // A malformed sni/public key is the same user-facing problem as a
        // missing one, so it earns the same specific help rather than the
        // generic "invalid link".
        "sni" | "server_name" => "sni",
        "pbk" | "public_key" => "publicKey",
        _ => "malformed",
      },
      Self::MissingField(field) => match *field {
        "sni" => "sni",
        "pbk" => "publicKey",
        "security" => "security",
        "flow" => "flow",
        _ => "malformed",
      },
      Self::UnsupportedParameter(_) => "parameter",
      Self::DuplicateParameter(_) => "malformed",
      Self::InvalidUri | Self::Serialization => "malformed",
    }
  }
}
