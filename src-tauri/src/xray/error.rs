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
