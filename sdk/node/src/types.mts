/**
 * Response shapes, spelled exactly the way the local API sends them.
 *
 * Every interface here mirrors a `ToSchema` struct in `src-tauri` field for
 * field. A Rust `Option<T>` becomes an optional property.
 *
 * Two spellings live side by side because the app sends both. Most bodies are
 * snake_case; the browser-facing agent types (`LocatorDescription`,
 * `LocatorCandidate`, `PerceptionPage` and friends) carry the browser's own
 * camelCase, because they are handed through from the browser rather than
 * restated. `AgentClick` and `AgentTyping` are the exceptions inside the agent
 * surface: they are snake_case with a single `match` key. These types follow
 * the wire rather than tidying it, so a value read from one call can be passed
 * straight into the next.
 */

/** The app's own JSON for a proxy's settings, declared `Object` in the spec. */
export type ProxySettings = Record<string, unknown>;

/** A Wayfern fingerprint/config blob, also declared `Object` in the spec. */
export type WayfernConfig = Record<string, unknown>;

/** Which implementation answered: the browser's native domains, or the fallback. */
export type Engine = "wayfern" | "fallback";

export interface ApiProfile {
  id: string;
  name: string;
  browser: string;
  version: string;
  proxy_id?: string | null;
  launch_hook?: string | null;
  process_id?: number | null;
  last_launch?: number | null;
  release_type: string;
  group_id?: string | null;
  tags: string[];
  is_running: boolean;
  proxy_bypass_rules: string[];
  vpn_id?: string | null;
  extension_group_id?: string | null;
  ephemeral: boolean;
  temporary: boolean;
  clear_on_close: boolean;
  /** `"Disabled"`, `"Regular"` or `"Encrypted"`. */
  sync_mode: string;
  cloud_sync_enabled: boolean;
  host_os?: string | null;
  /** A profile from another OS can only ever run on a remote host of that OS. */
  is_cross_os: boolean;
  fingerprint_os?: string | null;
}

export interface ApiProfilesResponse {
  profiles: ApiProfile[];
  total: number;
}

export interface ApiProfileResponse {
  profile: ApiProfile;
}

export interface ApiGroupResponse {
  id: string;
  name: string;
  profile_count: number;
}

export interface ApiProxyResponse {
  id: string;
  name: string;
  proxy_settings: ProxySettings;
}

export interface ApiVpnResponse {
  id: string;
  name: string;
  /** Always `"WireGuard"`. */
  vpn_type: string;
  created_at: number;
  last_used?: number | null;
}

export interface ApiVpnExportResponse {
  id: string;
  name: string;
  vpn_type: string;
  /** Raw, decrypted `.conf` content. Treat it as a secret. */
  config_data: string;
}

export interface DownloadBrowserResponse {
  browser: string;
  version: string;
  status: string;
}

export interface RunProfileResponse {
  profile_id: string;
  remote_debugging_port: number;
  headless: boolean;
}

export interface RunRemoteResponse {
  profile_id: string;
  session_id: string;
  /** Always the profile's own operating system. */
  platform: string;
  status: string;
}

export interface StopRemoteResponse {
  session_id: string;
  status: string;
  billed_seconds: number;
}

export interface SetCloudSyncResponse {
  profile_id: string;
  mode: string;
  remote_launchable: boolean;
  remote_blocked_reason?: string | null;
}

export interface RemoteSessionState {
  session_id: string;
  profile_id?: string | null;
  platform?: string | null;
  /** `provisioning` | `ready` | `live` | `closed` | `error`. */
  state: string;
  cdp_ready?: boolean;
  /** `interactive` or `cookie_bot`. */
  kind?: string | null;
  run_id?: string | null;
  team_id?: string | null;
  started_at?: string | null;
  ended_at?: string | null;
  close_reason?: string | null;
  billed_seconds?: number | null;
}

export interface ApiRemoteSessionsResponse {
  sessions: RemoteSessionState[];
}

export interface RemoteHoursBreakdown {
  interactive_hours?: number;
  bot_hours?: number;
}

export interface RemoteHoursMember {
  user_id: string;
  email: string;
  role?: string | null;
  used_hours?: number;
  interactive_hours?: number;
  bot_hours?: number;
}

export interface RemoteHoursQuota {
  granted_hours: number;
  remaining_hours: number;
  used_hours?: number;
  period_start?: string | null;
  period_end?: string | null;
  /** `user` or `team`. */
  scope?: string | null;
  team_id?: string | null;
  seats?: number;
  per_seat_hours?: number;
  breakdown?: RemoteHoursBreakdown | null;
  members?: RemoteHoursMember[];
}

export interface CookieBotSlot {
  run_at_minute?: number;
  days_mask?: number;
}

export interface CookieBotSchedule {
  profile_id: string;
  profile_name: string;
  platform: string;
  enabled: boolean;
  run_at_minute: number;
  days_mask: number;
  /**
   * Every time-of-day this enrolment fires. An older server sends only the
   * mirrored `run_at_minute`/`days_mask` pair above, so an empty list means
   * "fall back to the pair", never "fires at no time".
   */
  slots?: CookieBotSlot[];
  timezone: string;
  preset: string;
  template_id?: string | null;
  max_minutes: number;
  sites?: string[];
  jitter_seconds?: number;
  sync_enabled?: boolean;
  encrypted_sync?: boolean;
  has_proxy?: boolean;
  proxy_remote_reachable?: boolean;
  touch_fingerprint?: boolean;
  sticky_exit?: boolean;
  profile_state_at?: string | null;
  /** Why tonight would be refused, or absent. */
  blocked_by?: string | null;
  next_run_at?: string | null;
  last_run_at?: string | null;
  last_run_id?: string | null;
  owner_user_id?: string | null;
  owner_email?: string | null;
  updated_at?: string | null;
}

export interface CookieBotScheduleList {
  schedules?: CookieBotSchedule[];
  team_id?: string | null;
  scope?: string | null;
}

export interface CookieBotConflict {
  user_id: string;
  email: string;
  run_at_minute: number;
  timezone: string;
  days_mask: number;
  enabled: boolean;
  overlaps?: boolean;
}

export interface CookieBotScheduleSaved {
  schedule: CookieBotSchedule;
  conflicts?: CookieBotConflict[];
}

export interface CookieBotConflictCheck {
  profile_id: string;
  conflicts?: CookieBotConflict[];
}

export interface CookieBotScheduleDeleted {
  profile_id: string;
  deleted: boolean;
}

export interface CookieBotRun {
  id: string;
  profile_id: string;
  profile_name?: string | null;
  user_id?: string | null;
  email?: string | null;
  team_id?: string | null;
  /** `schedule` or `manual`. */
  trigger: string;
  /** `pending` | `running` | `succeeded` | `partial` | `failed` | `skipped` | `cancelled`. */
  status: string;
  scheduled_for: string;
  dispatch_after?: string | null;
  started_at?: string | null;
  ended_at?: string | null;
  max_minutes?: number;
  chunks_total?: number;
  chunk_index?: number;
  sites_total?: number;
  sites_visited?: number;
  sites_failed?: number;
  consent_dismissed?: number;
  billed_seconds?: number;
  outcome_code?: string | null;
  session_id?: string | null;
}

export interface CookieBotRunPage {
  runs?: CookieBotRun[];
  /** Keyset cursor; absent on the last page. */
  next_before?: string | null;
}

export interface CookieBotRunStarted {
  run: CookieBotRun;
  session_id?: string | null;
}

export interface CookieBotPreset {
  id: string;
  typical_minutes?: number | null;
  recommended?: boolean;
  name?: string | null;
  description?: string | null;
}

export interface CookieBotPresetList {
  presets?: CookieBotPreset[];
  default_preset?: string | null;
  /** Whatever the server publishes; the app forwards it without narrowing. */
  templates?: Record<string, unknown>[];
  limits?: Record<string, unknown> | null;
}

export interface CookieBotUsageMember {
  user_id: string;
  email: string;
  role?: string | null;
  interactive_hours?: number;
  bot_hours?: number;
  used_hours?: number;
  sessions?: number;
  bot_runs?: number;
  bot_runs_failed?: number;
}

export interface CookieBotUsageProfile {
  profile_id: string;
  profile_name?: string | null;
  owner_email?: string | null;
  bot_hours?: number;
  runs?: number;
  runs_failed?: number;
  last_run_at?: string | null;
  last_status?: string | null;
}

export interface CookieBotUsage {
  period: string;
  period_start?: string | null;
  period_end?: string | null;
  team_id?: string | null;
  seats?: number;
  granted_hours?: number;
  used_hours?: number;
  remaining_hours?: number;
  members?: CookieBotUsageMember[];
  profiles?: CookieBotUsageProfile[];
}

export interface BatchRunResult {
  profile_id: string;
  ok: boolean;
  remote_debugging_port?: number | null;
  error?: string | null;
}

export interface BatchRunResponse {
  results: BatchRunResult[];
}

export interface BatchStopResult {
  profile_id: string;
  ok: boolean;
  error?: string | null;
}

export interface BatchStopResponse {
  results: BatchStopResult[];
}

/** One profile, one proxy. The distribution applies exactly these pairs. */
export interface ProxyPair {
  profile_id: string;
  proxy_id: string;
}

export interface ProxyAssignmentResult {
  profile_id: string;
  proxy_id: string;
  ok: boolean;
  /** A `{"code": ...}` payload when `ok` is false, otherwise null. */
  error?: string | null;
}

export interface DistributeProxiesResponse {
  results: ProxyAssignmentResult[];
}

export interface ImportCookiesResponse {
  cookies_imported: number;
  cookies_replaced: number;
  errors: string[];
}

export interface ImportProxiesResponse {
  imported_count: number;
  skipped_count: number;
  errors: string[];
  proxies: ApiProxyResponse[];
}

export interface DetectedProfile {
  browser: string;
  mapped_browser: string;
  name: string;
  path: string;
  description: string;
}

export interface DetectedProfilesResponse {
  profiles: DetectedProfile[];
  total: number;
}

export interface ImportProfileItem {
  source_path: string;
  /**
   * The source browser family (`chromium`, `brave`, `edge`, ...). Load-bearing:
   * it picks which keychain entry unlocks the source's cookies and passwords.
   */
  browser_type?: string;
  new_profile_name: string;
  proxy_id?: string | null;
  vpn_id?: string | null;
  allow_running?: boolean | null;
}

export interface ProfileImportItemResult {
  name: string;
  source_path: string;
  /** `"imported"` | `"skipped"` | `"failed"`. */
  status: string;
  profile_id?: string | null;
  error?: string | null;
  report?: Record<string, unknown> | null;
}

export interface ProfileImportBatchResult {
  imported_count: number;
  skipped_count: number;
  failed_count: number;
  results: ProfileImportItemResult[];
}

export interface Extension {
  id: string;
  name: string;
  manifest_name?: string | null;
  file_name: string;
  file_type: string;
  browser_compatibility: string[];
  created_at: number;
  updated_at: number;
  sync_enabled?: boolean;
  last_sync?: number | null;
  version?: string | null;
  description?: string | null;
  author?: string | null;
  homepage_url?: string | null;
  /** `archive` or `unpacked`. */
  source_kind: string;
  /** Set when the extension is loaded from a folder in place. Never synced. */
  linked_path?: string | null;
}

export interface ExtensionGroup {
  id: string;
  name: string;
  extension_ids: string[];
  created_at: number;
  updated_at: number;
  sync_enabled?: boolean;
  last_sync?: number | null;
}

export interface LocatorAttribute {
  name: string;
  value: string;
}

/**
 * How an element is named without a CSS selector.
 *
 * At least one property must be set. Keys are the browser's own camelCase; the
 * app also accepts `name_contains` and `text_contains` on input, but a locator
 * handed back by `agentPick` uses the spellings below, so reusing one verbatim
 * is the reliable path.
 */
export interface LocatorDescription {
  /** AX role token, matched case- and separator-insensitively. */
  role?: string;
  /** Computed accessible name, exact after whitespace collapse. */
  name?: string;
  nameContains?: string;
  /** Visible text content, from the live layout. */
  text?: string;
  textContains?: string;
  attributes?: LocatorAttribute[];
}

export interface LocatorBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface LocatorCandidate {
  /** Absent on the fallback engine, which has no DOM agent behind it. */
  backendNodeId?: number;
  role: string;
  name: string;
  text: string;
  /** Omitted, never blanked, for a control the page marked protected. */
  value?: string;
  url?: string;
  /** Per-profile deterministic identifier for the node's structural position. */
  signature: string;
  attributes?: LocatorAttribute[];
  bounds: LocatorBounds;
}

export interface LocatorResolution {
  backendNodeId?: number;
  /** Always 1: present so a caller can assert it rather than infer it. */
  matchCount: number;
  match: LocatorCandidate;
  locator: LocatorDescription;
  engine: Engine;
}

export interface PerceptionNode {
  /** Short, stable, frame-qualified handle. */
  id: string;
  frameId: string;
  role: string;
  x: number;
  y: number;
  width: number;
  height: number;
  inViewport: boolean;
  visible: boolean;
  focused: boolean;
  disabled: boolean;
  parentId?: string;
  name?: string;
  text?: string;
  value?: string;
  /** `"true"`, `"false"` or `"mixed"`; absent for anything not checkable. */
  checked?: string;
  expanded?: boolean;
  scrollable?: boolean;
  scrollContainerId?: string;
}

export interface PerceptionFrame {
  frameId: string;
  url: string;
  crossOrigin: boolean;
  parentFrameId?: string;
}

export interface PerceptionStats {
  totalNodes: number;
  returnedNodes: number;
  bytes: number;
  elapsedMs: number;
  framesVisited: number;
  /** Frames whose renderer did not answer within the budget. */
  framesFailed: number;
}

export interface PerceptionPage {
  snapshotId: string;
  nodes: PerceptionNode[];
  frames: PerceptionFrame[];
  /** Readable text for exactly the nodes returned. */
  text: string;
  truncated: boolean;
  stats: PerceptionStats;
  /** Present when `truncated`: pass it back to continue. */
  cursor?: string;
  engine: Engine;
}

export interface ExtractionField {
  /** The key this column appears under in each row's values. */
  key: string;
  /** Evaluated inside each container; the first match wins. */
  locator: LocatorDescription;
  /** `"text"`, `"attribute"` or `"link"`. */
  source: string;
  /** Required when `source` is `"attribute"`. */
  attribute?: string;
}

export interface ExtractionRow {
  /** Global across pages. */
  index: number;
  /** Zero-based page this row came from. */
  page: number;
  values: Record<string, unknown>;
}

export interface Extraction {
  rows: ExtractionRow[];
  rowCount: number;
  pageCount: number;
  byteSize: number;
  truncated: boolean;
  /**
   * `complete` | `no-container` | `no-next` | `page-cap` | `row-cap` |
   * `byte-cap` | `time-budget`. A missing container is `no-container`, not an
   * error.
   */
  stopReason: string;
  engine: Engine;
}

export interface PickedElement {
  backendNodeId: number;
  /** The smallest description that still resolves to this node. */
  locator: LocatorDescription;
  matchCount: number;
  node: LocatorCandidate;
  engine: Engine;
}

/** What a click did. Note the snake_case body and the `match` key. */
export interface AgentClick {
  clicked: boolean;
  match: LocatorCandidate;
  engine: Engine;
  /** Whether a page load followed the click. */
  navigated: boolean;
}

/** What a typing call did. */
export interface AgentTyping {
  typed: boolean;
  characters: number;
  /** Absent on the fallback engine, which does not count its own mistypes. */
  corrections?: number;
  duration_ms: number;
  engine: Engine;
  match: LocatorCandidate;
}
