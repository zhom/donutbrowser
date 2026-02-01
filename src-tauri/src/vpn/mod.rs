//! VPN support module for WireGuard and OpenVPN configurations.
//!
//! This module provides:
//! - VPN config parsing (WireGuard .conf and OpenVPN .ovpn files)
//! - Encrypted storage for VPN configurations
//! - Tunnel management with userspace WireGuard (boringtun) and OpenVPN process management

mod config;
mod openvpn;
mod storage;
mod tunnel;
mod wireguard;

pub use config::{
  detect_vpn_type, parse_openvpn_config, parse_wireguard_config, OpenVpnConfig, VpnConfig,
  VpnError, VpnImportResult, VpnStatus, VpnType, WireGuardConfig,
};
pub use openvpn::OpenVpnTunnel;
pub use storage::VpnStorage;
pub use tunnel::{TunnelManager, VpnTunnel};
pub use wireguard::WireGuardTunnel;

use once_cell::sync::Lazy;
use std::sync::Mutex;

/// Global VPN storage instance
pub static VPN_STORAGE: Lazy<Mutex<VpnStorage>> = Lazy::new(|| Mutex::new(VpnStorage::new()));

/// Global tunnel manager instance
pub static TUNNEL_MANAGER: Lazy<tokio::sync::Mutex<TunnelManager>> =
  Lazy::new(|| tokio::sync::Mutex::new(TunnelManager::new()));
