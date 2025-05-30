export interface ProxySettings {
  enabled: boolean;
  proxy_type: string; // "http", "https", "socks4", or "socks5"
  host: string;
  port: number;
}

export interface TableSortingSettings {
  column: string; // "name", "browser", "status"
  direction: string; // "asc" or "desc"
}

export interface BrowserProfile {
  name: string;
  browser: string;
  version: string;
  profile_path: string;
  proxy?: ProxySettings;
  process_id?: number;
  last_launch?: number;
}

export interface AppUpdateInfo {
  current_version: string;
  new_version: string;
  release_notes: string;
  download_url: string;
  is_nightly: boolean;
  published_at: string;
}

export interface AppVersionInfo {
  version: string;
  is_nightly: boolean;
}
