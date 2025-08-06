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
  proxy?: string;
  screen_max_width?: number;
  screen_max_height?: number;
  geoip?: string | boolean;
  block_images?: boolean;
  block_webrtc?: boolean;
  block_webgl?: boolean;
  executable_path?: string;
  fingerprint?: string; // JSON string of the complete fingerprint config
}

// Extended interface for the advanced fingerprint configuration
export interface CamoufoxFingerprintConfig {
  // Navigator properties
  "navigator.userAgent"?: string;
  "navigator.appVersion"?: string;
  "navigator.platform"?: string;
  "navigator.oscpu"?: string;
  "navigator.appCodeName"?: string;
  "navigator.appName"?: string;
  "navigator.product"?: string;
  "navigator.productSub"?: string;
  "navigator.buildID"?: string;
  "navigator.language"?: string;
  "navigator.languages"?: string[];
  "navigator.doNotTrack"?: string;
  "navigator.hardwareConcurrency"?: number;
  "navigator.maxTouchPoints"?: number;
  "navigator.cookieEnabled"?: boolean;
  "navigator.globalPrivacyControl"?: boolean;
  "navigator.onLine"?: boolean;

  // Screen properties
  "screen.height"?: number;
  "screen.width"?: number;
  "screen.availHeight"?: number;
  "screen.availWidth"?: number;
  "screen.availTop"?: number;
  "screen.availLeft"?: number;
  "screen.colorDepth"?: number;
  "screen.pixelDepth"?: number;
  "screen.pageXOffset"?: number;
  "screen.pageYOffset"?: number;

  // Window properties
  "window.outerHeight"?: number;
  "window.outerWidth"?: number;
  "window.innerHeight"?: number;
  "window.innerWidth"?: number;
  "window.screenX"?: number;
  "window.screenY"?: number;
  "window.scrollMinX"?: number;
  "window.scrollMinY"?: number;
  "window.scrollMaxX"?: number;
  "window.scrollMaxY"?: number;
  "window.devicePixelRatio"?: number;
  "window.history.length"?: number;

  // Document properties
  "document.body.clientWidth"?: number;
  "document.body.clientHeight"?: number;
  "document.body.clientTop"?: number;
  "document.body.clientLeft"?: number;

  // Locale and geolocation
  "locale:language"?: string;
  "locale:region"?: string;
  "locale:script"?: string;
  "locale:all"?: string;
  "geolocation:latitude"?: number;
  "geolocation:longitude"?: number;
  "geolocation:accuracy"?: number;
  timezone?: string;

  // Headers
  "headers.Accept-Language"?: string;
  "headers.User-Agent"?: string;
  "headers.Accept-Encoding"?: string;

  // WebRTC
  "webrtc:ipv4"?: string;
  "webrtc:ipv6"?: string;
  "webrtc:localipv4"?: string;
  "webrtc:localipv6"?: string;

  // Battery
  "battery:charging"?: boolean;
  "battery:chargingTime"?: number;
  "battery:dischargingTime"?: number;
  "battery:level"?: number;

  // Fonts
  fonts?: string[];
  "fonts:spacing_seed"?: number;

  // Audio
  "AudioContext:sampleRate"?: number;
  "AudioContext:outputLatency"?: number;
  "AudioContext:maxChannelCount"?: number;

  // Media devices
  "mediaDevices:micros"?: number;
  "mediaDevices:webcams"?: number;
  "mediaDevices:speakers"?: number;
  "mediaDevices:enabled"?: boolean;

  // WebGL
  "webGl:renderer"?: string;
  "webGl:vendor"?: string;
  "webGl:supportedExtensions"?: string[];
  "webGl2:supportedExtensions"?: string[];
  "webGl:contextAttributes"?: {
    alpha?: boolean;
    antialias?: boolean;
    depth?: boolean;
    failIfMajorPerformanceCaveat?: boolean;
    powerPreference?: string;
    premultipliedAlpha?: boolean;
    preserveDrawingBuffer?: boolean;
    stencil?: boolean;
  };
  "webGl2:contextAttributes"?: {
    alpha?: boolean;
    antialias?: boolean;
    depth?: boolean;
    failIfMajorPerformanceCaveat?: boolean;
    powerPreference?: string;
    premultipliedAlpha?: boolean;
    preserveDrawingBuffer?: boolean;
    stencil?: boolean;
  };
  "webGl:parameters"?: Record<string, unknown>;
  "webGl2:parameters"?: Record<string, unknown>;
  "webGl:shaderPrecisionFormats"?: Record<string, unknown>;
  "webGl2:shaderPrecisionFormats"?: Record<string, unknown>;

  // Canvas
  "canvas:aaOffset"?: number;
  "canvas:aaCapOffset"?: boolean;

  // Voices
  voices?: Array<{
    isLocalService?: boolean;
    isDefault?: boolean;
    voiceURI?: string;
    name?: string;
    lang?: string;
  }>;
  "voices:blockIfNotDefined"?: boolean;
  "voices:fakeCompletion"?: boolean;
  "voices:fakeCompletion:charsPerSecond"?: number;

  // Other properties
  humanize?: boolean;
  "humanize:maxTime"?: number;
  "humanize:minTime"?: number;
  showcursor?: boolean;
  allowMainWorld?: boolean;
  forceScopeAccess?: boolean;
  enableRemoteSubframes?: boolean;
  disableTheming?: boolean;
  memorysaver?: boolean;
  addons?: string[];
  certificatePaths?: string[];
  certificates?: string[];
  debug?: boolean;
  pdfViewerEnabled?: boolean;
}

export interface CamoufoxLaunchResult {
  id: string;
  processId?: number;
  profilePath?: string;
  url?: string;
}
