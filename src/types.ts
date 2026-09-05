import type { CookieBotSchedule } from "@/lib/cookie-bot";
import type { RemoteSessionState } from "@/lib/remote-sessions";

export interface ProxySettings {
  proxy_type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
  vless_uri?: string;
}

export interface TableSortingSettings {
  column: string; // "name", "note", "status"
  direction: string; // "asc" or "desc"
}

export interface BrowserProfile {
  id: string; // UUID of the profile
  name: string;
  browser: string;
  version: string;
  proxy_id?: string; // Reference to stored proxy
  vpn_id?: string; // Reference to stored VPN config
  launch_hook?: string;
  process_id?: number;
  last_launch?: number;
  release_type: string;
  wayfern_config?: WayfernConfig; // Wayfern configuration
  group_id?: string; // Reference to profile group
  tags?: string[];
  note?: string; // User note
  window_color?: string; // Per-profile window frame color "#RRGGBB"; auto-derived from the id when unset
  sync_mode?: SyncMode;
  encryption_salt?: string;
  last_sync?: number; // Timestamp of last successful sync (epoch seconds)
  host_os?: string; // OS where profile was created ("macos", "windows", "linux")
  ephemeral?: boolean;
  clear_on_close?: boolean;
  extension_group_id?: string;
  proxy_bypass_rules?: string[];
  created_by_id?: string;
  created_by_email?: string;
  /** Profile creation timestamp (epoch seconds, UTC). Undefined for legacy
   * profiles created before this field existed. */
  created_at?: number;
  dns_blocklist?: string;
  password_protected?: boolean;
}

export interface Extension {
  id: string;
  name: string;
  file_name: string;
  file_type: string;
  browser_compatibility: string[];
  created_at: number;
  updated_at: number;
  sync_enabled?: boolean;
  last_sync?: number;
  version?: string;
  description?: string;
  author?: string;
  homepage_url?: string;
  /** How the payload was imported: a `.crx`/`.zip` archive, or a folder. */
  source_kind: "archive" | "unpacked";
  /** Absolute folder the extension is loaded from in place. Set means nothing
   * was copied into Donut, so the extension is machine-local and never syncs. */
  linked_path?: string;
}

export interface ExtensionGroup {
  id: string;
  name: string;
  extension_ids: string[];
  created_at: number;
  updated_at: number;
  sync_enabled?: boolean;
  last_sync?: number;
}

export type SyncMode = "Disabled" | "Regular" | "Encrypted";

export type SyncStatus = "Disabled" | "Syncing" | "Synced" | "Error";

export interface SyncSettings {
  sync_server_url?: string;
  sync_token?: string;
}

/**
 * Result of `check_sync_server_connection`. Files upload straight to the
 * storage host named in the presigned URL rather than through the sync server,
 * so a healthy server is not evidence that sync works: `storage_reachable`
 * false means every transfer will fail at connect.
 *
 * `null` means "not known", which is not the same as false — a server that
 * predates `/readyz`, or a cloud deployment that withholds its storage host,
 * discloses nothing to probe.
 */
export interface SyncServerCheck {
  server_reachable: boolean;
  storage_ready: boolean | null;
  storage_endpoint: string | null;
  storage_reachable: boolean | null;
  storage_error: string | null;
}

/**
 * Capability/limit set derived from the plan by the backend. Features are gated
 * on these flags instead of a single "is paid?" check, so a plan like "solo"
 * (cloud backup + nightly cookie bot, no automation, no fingerprint editing, no
 * hands-on remote session) is just data. Mirrors
 * `apps/backend/src/plans/entitlements.ts`. Resolve via `getEntitlements()` —
 * the desktop populates it, but it stays optional for safety on older state.
 */
export interface Entitlements {
  active: boolean;
  browserAutomation: boolean;
  crossOsFingerprints: boolean;
  cloudBackup: boolean;
  teamCollaboration: boolean;
  /** Overnight profile warming on a leased remote host. */
  cookieBot: boolean;
  /**
   * May open a HANDS-ON remote session. Not implied by `cookieBot` or by a
   * non-zero `remoteBrowserHours`: solo funds a nightly bot out of its hours and
   * may not drive a remote browser itself, so any UI offering interactive remote
   * control must read this flag.
   */
  remoteInteractive: boolean;
  profileLimit: number;
  requestsPerHour: number;
  /**
   * Per-seat monthly allowance for remote sessions. Reporting only: the
   * spendable figure is whatever `get_remote_hours_quota` returns, because a
   * team pools this across its seats and only the server knows the seat count.
   */
  remoteBrowserHours: number;
}

/**
 * What a backend older than the current release actually sends. Read it through
 * `getEntitlements()`, which fills the gaps — never off `CloudUser` directly, or
 * a paying customer's Cookie Bot silently reads `false`.
 *
 * `remoteInteractive` joins the optional set for the same reason `cookieBot`
 * did: a backend predating the solo tier omits it, and reading the absent key as
 * `false` would take interactive remote sessions away from a Pro customer whose
 * only mistake was a stale cached login.
 */
export type ServerEntitlements = Omit<
  Entitlements,
  "cookieBot" | "remoteBrowserHours" | "remoteInteractive"
> &
  Partial<
    Pick<Entitlements, "cookieBot" | "remoteBrowserHours" | "remoteInteractive">
  >;

export interface CloudUser {
  id: string;
  email: string;
  plan: string;
  planPeriod: string | null;
  subscriptionStatus: string;
  profileLimit: number;
  cloudProfilesUsed: number;
  proxyBandwidthLimitMb: number;
  proxyBandwidthUsedMb: number;
  proxyBandwidthExtraMb: number;
  teamId?: string;
  teamName?: string;
  teamRole?: string;
  // This device's position among the user's active devices (oldest = 1).
  // Ordinal 1 / isPrimaryDevice === true is the only device that can run
  // browser automation. Optional: older backends omit them.
  deviceOrdinal?: number | null;
  deviceCount?: number | null;
  isPrimaryDevice?: boolean | null;
  // Plan-derived capabilities. The desktop resolves this before handing CloudUser
  // to the UI; optional to stay safe on older cached state.
  entitlements?: ServerEntitlements;
}

/**
 * Cookie Bot and remote-session wire types. Defined next to the transport that
 * owns them (`src/lib/cookie-bot.ts`, `src/lib/remote-sessions.ts`) and
 * re-exported here so a component reads one module. Type-only, so nothing is
 * pulled into the bundle.
 */
export type {
  CookieBotConflict,
  CookieBotPreset,
  CookieBotPresetList,
  CookieBotRun,
  CookieBotRunPage,
  CookieBotRunStarted,
  CookieBotSchedule,
  CookieBotScheduleInput,
  CookieBotScheduleList,
  CookieBotScheduleSaved,
  CookieBotScope,
  CookieBotUsage,
  CookieBotUsageMember,
  CookieBotUsageProfile,
  RemoteHoursMember,
  RemoteHoursQuota,
} from "@/lib/cookie-bot";
export type {
  RemoteSessionPhase,
  RemoteSessionSnapshot,
  RemoteSessionState,
} from "@/lib/remote-sessions";

/** Where a profile stands with the bot, as one row of the profile table reads it. */
export interface ProfileBotState {
  /** The stored enrolment, or null when the profile is not enrolled. */
  schedule: CookieBotSchedule | null;
  /** A remote session for this profile that has not closed yet. */
  liveSession: RemoteSessionState | null;
}

export interface ProfileLockInfo {
  profileId: string;
  lockedBy: string;
  lockedByEmail: string;
  lockedAt: string;
  expiresAt?: string;
}

export interface CloudAuthState {
  user: CloudUser;
  logged_in_at: string;
}

export interface ProfileSyncStatusEvent {
  profile_id: string;
  status: "disabled" | "syncing" | "synced" | "error" | "pending";
}

export interface ProxyCheckResult {
  ip: string;
  city?: string;
  country?: string;
  country_code?: string;
  timestamp: number;
  is_valid: boolean;
}

export function isSyncEnabled(profile: BrowserProfile): boolean {
  return profile.sync_mode != null && profile.sync_mode !== "Disabled";
}

export const CLOUD_PROXY_ID = "cloud-included-proxy";

export interface StoredProxy {
  id: string;
  name: string;
  proxy_settings: ProxySettings;
  sync_enabled?: boolean;
  last_sync?: number;
  is_cloud_managed?: boolean;
  is_cloud_derived?: boolean;
  geo_country?: string;
  geo_state?: string;
  geo_region?: string;
  geo_city?: string;
  geo_isp?: string;
}

export interface LocationItem {
  code: string;
  name: string;
}

export interface ProfileGroup {
  id: string;
  name: string;
  sync_enabled?: boolean;
  last_sync?: number;
}

export interface GroupWithCount {
  id: string;
  name: string;
  count: number;
  sync_enabled?: boolean;
  last_sync?: number;
}

export interface DetectedProfile {
  browser: string;
  name: string;
  path: string;
  description: string;
  mapped_browser: string;
}

export interface ImportProfileItem {
  source_path: string;
  /**
   * Source browser family. Selects which OS keychain entry holds the key that
   * unlocks the source's cookies and passwords, so it decides whether secrets
   * survive the import.
   */
  browser_type?: string;
  new_profile_name: string;
  /** Mutually exclusive with `vpn_id`; the importer rejects setting both. */
  proxy_id?: string | null;
  vpn_id?: string | null;
  /** Import even though the source browser is still running. */
  allow_running?: boolean;
}

/** Stable warning codes; each maps to `importProfile.warnings.*`. */
export type ProfileImportWarning =
  | "secretsNotMigrated"
  | "appBoundEncrypted"
  | "storeTooOld"
  | "storeTooNew"
  | "sourceBrowserRunning"
  | "securePreferencesReset"
  | "extensionsPartial"
  | "storeUnreadable";

export interface ProfileImportReport {
  cookies_migrated: number;
  cookies_unrecoverable: number;
  passwords_migrated: number;
  passwords_unrecoverable: number;
  payment_methods_migrated: number;
  payment_methods_unrecoverable: number;
  extensions_migrated: number;
  history_entries: number;
  bookmarks: number;
  local_storage_origins: number;
  bytes_copied: number;
  warnings: ProfileImportWarning[];
}

export interface ProfileImportItemResult {
  name: string;
  source_path: string;
  status: "imported" | "skipped" | "failed";
  profile_id: string | null;
  error: string | null;
  /** What actually came across. Present when status is "imported". */
  report?: ProfileImportReport | null;
}

export interface ProfileImportBatchResult {
  imported_count: number;
  skipped_count: number;
  failed_count: number;
  results: ProfileImportItemResult[];
}

export interface ArchiveScanResult {
  extracted_dir: string;
  profiles: DetectedProfile[];
}

export interface ProfileImportProgress {
  total: number;
  completed: number;
  index: number;
  name: string;
  status: "importing" | "imported" | "skipped" | "failed";
}

export interface BrowserReleaseTypes {
  stable?: string;
}

export interface AppUpdateInfo {
  current_version: string;
  new_version: string;
  release_notes: string;
  download_url: string;
  is_nightly: boolean;
  published_at: string;
  manual_update_required: boolean;
  release_page_url?: string;
  repo_update: boolean;
  /** URL of the release's SHA256SUMS.txt; downloads are verified against it. */
  checksums_url?: string | null;
  /** GitHub-computed digest of the chosen asset ("sha256:<hex>"). */
  asset_digest?: string | null;
}

export interface AppUpdateProgress {
  stage: string; // "downloading", "extracting", "installing", "completed"
  percentage?: number;
  speed?: string; // MB/s
  eta?: string; // estimated time remaining
  message: string;
}

export type WayfernOS = "windows" | "macos" | "linux" | "android" | "ios";

export interface WayfernConfig {
  proxy?: string;
  screen_max_width?: number;
  screen_max_height?: number;
  screen_min_width?: number;
  screen_min_height?: number;
  geoip?: string | boolean; // For compatibility with shared config form
  block_images?: boolean; // For compatibility with shared config form
  block_webrtc?: boolean;
  block_webgl?: boolean;
  executable_path?: string;
  fingerprint?: string; // JSON string of the complete fingerprint config
  randomize_fingerprint_on_launch?: boolean; // Generate new fingerprint on every launch
  os?: WayfernOS; // Operating system for fingerprint generation
  geo_proxy_signature?: string; // Internal: routing the fingerprint's location was computed for
  identity_id?: string; // Internal: UUID the device is derived from on browsers with the identity API
  identity_baseline?: string; // Legacy: read once by the migration to identity-only storage, never written
  identity_overrides?: string; // JSON object of the user's own edits to an identity-backed device
  location?: string; // JSON object of the exit-derived location fields (timezone, language, coordinates)
}

// Wayfern fingerprint config - matches the C++ FingerprintData structure
export interface WayfernFingerprintConfig {
  // User agent and platform
  userAgent?: string;
  platform?: string;
  platformVersion?: string;
  brand?: string;
  brandVersion?: string;

  // Hardware
  hardwareConcurrency?: number;
  maxTouchPoints?: number;
  deviceMemory?: number;

  // Screen
  screenWidth?: number;
  screenHeight?: number;
  screenAvailWidth?: number;
  screenAvailHeight?: number;
  screenColorDepth?: number;
  screenPixelDepth?: number;
  devicePixelRatio?: number;

  // Window
  windowOuterWidth?: number;
  windowOuterHeight?: number;
  windowInnerWidth?: number;
  windowInnerHeight?: number;
  screenX?: number;
  screenY?: number;

  // Language
  language?: string;
  languages?: string[];

  // Browser features
  doNotTrack?: string;
  cookieEnabled?: boolean;
  webdriver?: boolean;
  pdfViewerEnabled?: boolean;

  // WebGL
  webglVendor?: string;
  webglRenderer?: string;
  webglVersion?: string;
  webglShadingLanguageVersion?: string;
  webglParameters?: string; // JSON string
  webgl2Parameters?: string; // JSON string
  webglShaderPrecisionFormats?: string; // JSON string
  webgl2ShaderPrecisionFormats?: string; // JSON string

  // Timezone and geolocation
  timezone?: string;
  timezoneOffset?: number;
  latitude?: number;
  longitude?: number;
  accuracy?: number;

  // Media queries / preferences
  prefersReducedMotion?: boolean;
  prefersDarkMode?: boolean;
  prefersContrast?: string;
  prefersReducedData?: boolean;

  // Color/HDR
  colorGamutSrgb?: boolean;
  colorGamutP3?: boolean;
  colorGamutRec2020?: boolean;
  hdrSupport?: boolean;

  // Audio
  audioSampleRate?: number;
  audioMaxChannelCount?: number;

  // Storage
  localStorage?: boolean;
  sessionStorage?: boolean;
  indexedDb?: boolean;

  // Canvas
  canvasNoiseSeed?: string;

  // Fonts, plugins, mime types (JSON strings)
  fonts?: string; // JSON array string
  plugins?: string; // JSON array string
  mimeTypes?: string; // JSON array string

  // Battery (optional)
  batteryCharging?: boolean;
  batteryChargingTime?: number;
  batteryDischargingTime?: number;
  batteryLevel?: number;

  // Voices
  voices?: string; // JSON array string

  // Vendor info
  vendor?: string;
  vendorSub?: string;
  productSub?: string;

  // Network (optional)
  connectionEffectiveType?: string;
  connectionDownlink?: number;
  connectionRtt?: number;

  // Performance
  performanceMemory?: number;
}

export interface WayfernLaunchResult {
  id: string;
  processId?: number;
  profilePath?: string;
  url?: string;
  cdp_port?: number;
}

// Synchronizer types
export interface SyncFollowerState {
  profile_id: string;
  profile_name: string;
  failed_at_url: string | null;
}

export interface SyncSessionInfo {
  id: string;
  leader_profile_id: string;
  leader_profile_name: string;
  followers: SyncFollowerState[];
}

// Traffic stats types
export interface BandwidthDataPoint {
  timestamp: number;
  bytes_sent: number;
  bytes_received: number;
}

export interface DomainAccess {
  domain: string;
  request_count: number;
  bytes_sent: number;
  bytes_received: number;
  first_access: number;
  last_access: number;
}

export interface TrafficStats {
  proxy_id: string;
  profile_id?: string;
  session_start: number;
  last_update: number;
  total_bytes_sent: number;
  total_bytes_received: number;
  total_requests: number;
  bandwidth_history: BandwidthDataPoint[];
  domains: Record<string, DomainAccess>;
  unique_ips: string[];
}

export interface TrafficSnapshot {
  profile_id?: string;
  session_start: number;
  last_update: number;
  total_bytes_sent: number;
  total_bytes_received: number;
  total_requests: number;
  current_bytes_sent: number;
  current_bytes_received: number;
  recent_bandwidth: BandwidthDataPoint[];
}

export interface FilteredTrafficStats {
  profile_id?: string;
  session_start: number;
  last_update: number;
  total_bytes_sent: number;
  total_bytes_received: number;
  total_requests: number;
  bandwidth_history: BandwidthDataPoint[];
  period_bytes_sent: number;
  period_bytes_received: number;
  period_requests: number;
  domains: Record<string, DomainAccess>;
  unique_ips: string[];
}

// Cookie copy types
export interface UnifiedCookie {
  name: string;
  value: string;
  domain: string;
  path: string;
  expires: number;
  is_secure: boolean;
  is_http_only: boolean;
  same_site: number;
  creation_time: number;
  last_accessed: number;
}

export interface DomainCookies {
  domain: string;
  cookies: UnifiedCookie[];
  cookie_count: number;
}

export interface CookieReadResult {
  profile_id: string;
  browser_type: string;
  domains: DomainCookies[];
  total_count: number;
}

export interface SelectedCookie {
  domain: string;
  name: string;
}

export interface CookieCopyRequest {
  source_profile_id: string;
  target_profile_ids: string[];
  selected_cookies: SelectedCookie[];
}

export interface CookieCopyResult {
  target_profile_id: string;
  cookies_copied: number;
  cookies_replaced: number;
  errors: string[];
}

// Cookie paste types. Unlike the copy types above these are serialized with
// `rename_all = "camelCase"`, so the field names differ from the Rust structs.
export type CookieIssueSeverity = "error" | "warning" | "info";

export interface CookieIssue {
  code: string;
  severity: CookieIssueSeverity;
  source: string | null;
  params: Record<string, string>;
}

export type CookiePasteFormat = "json" | "netscape" | "nameValue";

export type CookieWriteMode = "merge" | "replaceMatchingSites";

/** Carries no `value`: the value is the credential and never leaves Rust. */
export interface PastedCookiePreview {
  name: string;
  domain: string;
  path: string;
  expires: number;
  isSecure: boolean;
  isHttpOnly: boolean;
  sameSite: number;
}

export interface CookieAnalysis {
  format: CookiePasteFormat | null;
  cookies: PastedCookiePreview[];
  issues: CookieIssue[];
  siteRequired: boolean;
  expiredCount: number;
  /** `null` when the store cannot be read, which is not the same as zero. */
  replaceDeleteCount: number | null;
  clearsOnClose: boolean;
  /** A `{"code":…}` string for `translateBackendError`, or `null` to proceed. */
  blockedBy: string | null;
}

export interface CookiePasteImportResult {
  added: number;
  overwritten: number;
  deleted: number;
  skipped: number;
  issues: CookieIssue[];
}

// Proxy import/export types
export interface ProxyExportData {
  version: string;
  proxies: ExportedProxy[];
  exported_at: string;
  source: string;
}

export interface ExportedProxy {
  name: string;
  type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
}

export interface ProxyImportResult {
  imported_count: number;
  skipped_count: number;
  errors: string[];
  proxies: StoredProxy[];
}

export interface ParsedProxyLine {
  proxy_type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
  vless_uri?: string;
  original_line: string;
}

export type ProxyParseResult =
  | ({ status: "parsed" } & ParsedProxyLine)
  | { status: "ambiguous"; line: string; possible_formats: string[] }
  | { status: "invalid"; line: string; reason: string };

// VPN types
export type VpnType = "WireGuard";

export interface VpnConfig {
  id: string;
  name: string;
  vpn_type: VpnType;
  config_data: string; // Raw config content (may be empty in list view)
  created_at: number;
  last_used?: number;
  sync_enabled?: boolean;
  last_sync?: number;
}

export interface VpnImportResult {
  success: boolean;
  vpn_id?: string;
  vpn_type?: VpnType;
  name: string;
  error?: string;
}

export interface VpnStatus {
  connected: boolean;
  vpn_id: string;
  connected_at?: number;
  bytes_sent?: number;
  bytes_received?: number;
  last_handshake?: number;
}

/** Result of comparing a proxy's exit node against a profile's fingerprint. */
export interface ConsistencyResult {
  consistent: boolean;
  checked: boolean;
  exit_ip: string | null;
  exit_country_code: string | null;
  exit_timezone: string | null;
  fingerprint_timezone: string | null;
  fingerprint_language: string | null;
  /** Which dimensions disagree: "timezone", "language". */
  mismatches: string[];
}

/**
 * How strongly an extension is believed to be a VPN or proxy tool.
 * "capability" is not such a claim: it means only that the extension holds
 * Chromium's `proxy` permission, which download managers do too.
 */
export type VpnExtensionConfidence = "confirmed" | "likely" | "capability";

/** How much of a profile's extension set could be read. */
export type ExtensionScanState =
  | "scanned"
  | "partial"
  | "encrypted"
  | "ephemeral"
  | "missing";

/** An extension found in a profile that could change where the browser connects. */
export interface DetectedVpnExtension {
  /** Acknowledgement identity: `donut:<uuid>` or `crx:<id>`. */
  key: string;
  name: string;
  version: string | null;
  /** "donut" (managed by Donut) or "browser" (installed in the profile). */
  source: string;
  confidence: VpnExtensionConfidence;
  /** Holds the `proxy` permission outright, so it can change the proxy today. */
  proxy_control: boolean;
  signals: string[];
}

/** Local-only checks answered before a launch starts any worker. */
export interface PreLaunchChecks {
  vpn_extensions: DetectedVpnExtension[];
  scan_state: ExtensionScanState;
  consistency: ConsistencyResult;
  exit_probe_pending: boolean;
  exit_measurement_unreliable: boolean;
  consent_token: string | null;
}

/**
 * What happened when the user asked Donut to become the default browser.
 *
 * macOS and Linux let a program make the change itself, so the answer there is
 * always "set". Windows reserves the final choice for its own settings page:
 * the app registers itself, Windows Settings opens, and the user finishes the
 * job. Treating that case as plain success is how the button used to report a
 * change that had not happened.
 */
export type SetDefaultBrowserOutcome =
  | { status: "set" }
  | { status: "awaitingSystemSettings" };
