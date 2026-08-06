use thiserror::Error;

pub type XrayResult<T> = Result<T, XrayError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XrayError {
  #[error("invalid VLESS URI")]
  InvalidUri,
  #[error("URI scheme must be vless")]
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
  /// A stable, translatable identifier for *why* a URI was rejected.
  ///
  /// Donut supports one VLESS shape — REALITY + XTLS Vision over TCP — so most
  /// rejections are "your setup is a kind we do not support", not "you made a
  /// typo". The frontend turns these into a sentence naming the unsupported
  /// part; without them every rejection reads as a malformed URI and a user
  /// with a working WebSocket or plain-TLS server has no idea why it failed.
  pub fn reason_code(&self) -> &'static str {
    match self {
      Self::UnsupportedScheme => "scheme",
      Self::UnsupportedValue { field, .. } | Self::InvalidField { field, .. } => match *field {
        "security" => "security",
        "flow" => "flow",
        "type" => "transport",
        "encryption" => "encryption",
        "headerType" => "headerType",
        "fp" => "fingerprint",
        // A malformed sni/public key is the same user-facing problem as a
        // missing one, so it earns the same specific help rather than the
        // generic "invalid URI".
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
