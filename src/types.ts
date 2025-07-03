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
}

export interface StoredProxy {
  id: string;
  name: string;
  proxy_settings: ProxySettings;
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
