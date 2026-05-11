pub const FINGERPRINT_NETWORK_ZIP: &[u8] = include_bytes!("fingerprint-network-definition.zip");
pub const INPUT_NETWORK_ZIP: &[u8] = include_bytes!("input-network-definition.zip");
pub const HEADER_NETWORK_ZIP: &[u8] = include_bytes!("header-network-definition.zip");
pub const BROWSER_HELPER_JSON: &str = include_str!("browser-helper-file.json");
pub const HEADERS_ORDER_JSON: &str = include_str!("headers-order.json");
pub const FONTS_JSON: &str = include_str!("fonts.json");
pub const BROWSERFORGE_YML: &str = include_str!("browserforge.yml");
pub const WEBGL_DATA_DB: &[u8] = include_bytes!("webgl_data.db");
pub const TERRITORY_INFO_XML: &str = include_str!("territoryInfo.xml");

/// Real fingerprint presets bundled with the original Camoufox v135 line
/// (Firefox <= 148). Frozen upstream — kept around so users who haven't
/// upgraded their Camoufox binary keep getting matched fingerprints.
/// Mirrors `pythonlib/camoufox/fingerprint-presets.json` upstream.
pub const FINGERPRINT_PRESETS_V135_JSON: &str = include_str!("fingerprint-presets-v135.json");

/// Real fingerprint presets for every Camoufox release after the v135 line
/// (currently Firefox 149+ via the v150 build). This file is expected to
/// be refreshed regularly as upstream Camoufox tracks newer Firefox
/// releases — we keep the upstream filename here so each refresh is a
/// straight `cp` from `pythonlib/camoufox/fingerprint-presets-v150.json`.
pub const FINGERPRINT_PRESETS_NEWER_JSON: &str = include_str!("fingerprint-presets-v150.json");

/// Firefox major version at which the newer preset bundle takes over from
/// the frozen v135 bundle. Matches `PRESETS_V150_MIN_FF` in
/// `pythonlib/camoufox/fingerprints.py`.
pub const PRESETS_NEWER_MIN_FF: u32 = 149;
