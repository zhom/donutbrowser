export interface ProxySettings {
  proxy_type: string; // "http", "https", "socks4", or "socks5"
  host: string;
  port: number;
  username?: string;
  password?: string;
}

export interface TableSortingSettings {
  column: string; // "name", "browser", "status"
  direction: string; // "asc" or "desc"
}

export interface BrowserProfile {
  id: string; // UUID of the profile
  name: string;
  browser: string;
  version: string;
  proxy_id?: string; // Reference to stored proxy
  process_id?: number;
  last_launch?: number;
  release_type: string; // "stable" or "nightly"
  camoufox_config?: CamoufoxConfig; // Camoufox configuration
  group_id?: string; // Reference to profile group
}

export interface StoredProxy {
  id: string;
  name: string;
  proxy_settings: ProxySettings;
}

export interface ProfileGroup {
  id: string;
  name: string;
}

export interface GroupWithCount {
  id: string;
  name: string;
  count: number;
}

export interface DetectedProfile {
  browser: string;
  name: string;
  path: string;
  description: string;
}

export interface BrowserReleaseTypes {
  stable?: string;
  nightly?: string;
}

export interface AppUpdateInfo {
  current_version: string;
  new_version: string;
  release_notes: string;
  download_url: string;
  is_nightly: boolean;
  published_at: string;
}

export interface AppUpdateProgress {
  stage: string; // "downloading", "extracting", "installing", "completed"
  percentage?: number;
  speed?: string; // MB/s
  eta?: string; // estimated time remaining
  message: string;
}

export interface CamoufoxConfig {
  os?: string[];
  block_images?: boolean;
  block_webrtc?: boolean;
  block_webgl?: boolean;
  disable_coop?: boolean;
  geoip?: string | boolean;
  country?: string;
  timezone?: string;
  latitude?: number;
  longitude?: number;
  humanize?: boolean;
  humanize_duration?: number;
  headless?: boolean;
  locale?: string[];
  addons?: string[];
  fonts?: string[];
  custom_fonts_only?: boolean;
  exclude_addons?: string[];
  screen_min_width?: number;
  screen_max_width?: number;
  screen_min_height?: number;
  screen_max_height?: number;
  window_width?: number;
  window_height?: number;
  ff_version?: number;
  main_world_eval?: boolean;
  webgl_vendor?: string;
  webgl_renderer?: string;
  proxy?: string;
  enable_cache?: boolean;
  virtual_display?: string;
  debug?: boolean;
  additional_args?: string[];
  env_vars?: Record<string, string>;
  firefox_prefs?: Record<string, unknown>;
}

export interface CamoufoxLaunchResult {
  id: string;
  port?: number;
  wsEndpoint?: string;
  profilePath?: string;
  url?: string;
}
