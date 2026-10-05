mod client;
mod error;
mod model;
mod uri;

pub use client::{build_client_config, build_client_config_json, XrayClientRuntime};
pub use error::{XrayError, XrayResult};
pub use model::{
  is_xray_proxy_type, Fingerprint, GrpcMode, HttpHeader, ParsedShareLink, RealitySettings,
  Security, StreamSettings, TlsSettings, Transport, VlessFlow, VmessSecurity, XhttpMode,
  XrayConfig, XrayOutbound, XrayProtocol,
};
pub use uri::{export_share_link, parse_share_link, share_link_host};
