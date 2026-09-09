"""Response shapes, spelled exactly the way the local API sends them.

Every entry here mirrors a ``ToSchema`` struct in ``src-tauri`` field for field.
A Rust ``Option<T>`` becomes a key that may be absent, expressed with the
``total=False`` half of each pair of classes, so ``dict.get`` is the honest way
to read one.

Two spellings live side by side because the app sends both. Most bodies are
snake_case; the browser-facing agent types (``LocatorDescription``,
``LocatorCandidate``, ``PerceptionPage`` and friends) carry the browser's own
camelCase, because they are handed through from the browser rather than
restated. ``AgentClick`` and ``AgentTyping`` are the exceptions inside the
agent surface: they are snake_case with a single ``match`` key. The types below
follow the wire rather than tidying it, so a value read from one call can be
passed straight into the next.
"""

from __future__ import annotations

from typing import Any, Dict, List, TypedDict

__all__ = [
    "ApiProfile",
    "ApiProfilesResponse",
    "ApiProfileResponse",
    "ApiGroupResponse",
    "ApiProxyResponse",
    "ApiVpnResponse",
    "ApiVpnExportResponse",
    "DownloadBrowserResponse",
    "RunProfileResponse",
    "RunRemoteResponse",
    "StopRemoteResponse",
    "SetCloudSyncResponse",
    "RemoteSessionState",
    "ApiRemoteSessionsResponse",
    "RemoteHoursBreakdown",
    "RemoteHoursMember",
    "RemoteHoursQuota",
    "CookieBotSlot",
    "CookieBotSchedule",
    "CookieBotScheduleList",
    "CookieBotConflict",
    "CookieBotScheduleSaved",
    "CookieBotConflictCheck",
    "CookieBotScheduleDeleted",
    "CookieBotRun",
    "CookieBotRunPage",
    "CookieBotRunStarted",
    "CookieBotPreset",
    "CookieBotPresetList",
    "CookieBotUsageMember",
    "CookieBotUsageProfile",
    "CookieBotUsage",
    "BatchRunResult",
    "BatchRunResponse",
    "BatchStopResult",
    "BatchStopResponse",
    "ProxyPair",
    "ProxyAssignmentResult",
    "DistributeProxiesResponse",
    "ImportCookiesResponse",
    "ImportProxiesResponse",
    "DetectedProfile",
    "DetectedProfilesResponse",
    "ImportProfileItem",
    "ProfileImportItemResult",
    "ProfileImportBatchResult",
    "Extension",
    "ExtensionGroup",
    "LocatorAttribute",
    "LocatorDescription",
    "LocatorBounds",
    "LocatorCandidate",
    "LocatorResolution",
    "PerceptionNode",
    "PerceptionFrame",
    "PerceptionStats",
    "PerceptionPage",
    "ExtractionField",
    "ExtractionRow",
    "Extraction",
    "PickedElement",
    "AgentClick",
    "AgentTyping",
]

# The app's own JSON for a proxy's settings. Declared `Object` in the OpenAPI
# document rather than a struct, so it is passed through untouched.
ProxySettings = Dict[str, Any]

# A Wayfern fingerprint/config blob. Also declared `Object` in the document.
WayfernConfig = Dict[str, Any]


class _ApiProfileRequired(TypedDict):
    id: str
    name: str
    browser: str
    version: str
    release_type: str
    tags: List[str]
    is_running: bool
    proxy_bypass_rules: List[str]
    ephemeral: bool
    temporary: bool
    clear_on_close: bool
    sync_mode: str
    cloud_sync_enabled: bool
    is_cross_os: bool


class ApiProfile(_ApiProfileRequired, total=False):
    proxy_id: str
    launch_hook: str
    process_id: int
    last_launch: int
    group_id: str
    vpn_id: str
    extension_group_id: str
    host_os: str
    fingerprint_os: str


class ApiProfilesResponse(TypedDict):
    profiles: List[ApiProfile]
    total: int


class ApiProfileResponse(TypedDict):
    profile: ApiProfile


class ApiGroupResponse(TypedDict):
    id: str
    name: str
    profile_count: int


class ApiProxyResponse(TypedDict):
    id: str
    name: str
    proxy_settings: ProxySettings


class _ApiVpnRequired(TypedDict):
    id: str
    name: str
    vpn_type: str
    created_at: int


class ApiVpnResponse(_ApiVpnRequired, total=False):
    last_used: int


class ApiVpnExportResponse(TypedDict):
    id: str
    name: str
    vpn_type: str
    config_data: str


class DownloadBrowserResponse(TypedDict):
    browser: str
    version: str
    status: str


class RunProfileResponse(TypedDict):
    profile_id: str
    remote_debugging_port: int
    headless: bool


class RunRemoteResponse(TypedDict):
    profile_id: str
    session_id: str
    platform: str
    status: str


class StopRemoteResponse(TypedDict):
    session_id: str
    status: str
    billed_seconds: int


class _SetCloudSyncRequired(TypedDict):
    profile_id: str
    mode: str
    remote_launchable: bool


class SetCloudSyncResponse(_SetCloudSyncRequired, total=False):
    remote_blocked_reason: str


class _RemoteSessionStateRequired(TypedDict):
    session_id: str
    state: str


class RemoteSessionState(_RemoteSessionStateRequired, total=False):
    profile_id: str
    platform: str
    cdp_ready: bool
    kind: str
    run_id: str
    team_id: str
    started_at: str
    ended_at: str
    close_reason: str
    billed_seconds: int


class ApiRemoteSessionsResponse(TypedDict):
    sessions: List[RemoteSessionState]


class RemoteHoursBreakdown(TypedDict, total=False):
    interactive_hours: float
    bot_hours: float


class _RemoteHoursMemberRequired(TypedDict):
    user_id: str
    email: str


class RemoteHoursMember(_RemoteHoursMemberRequired, total=False):
    role: str
    used_hours: float
    interactive_hours: float
    bot_hours: float


class _RemoteHoursQuotaRequired(TypedDict):
    granted_hours: float
    remaining_hours: float


class RemoteHoursQuota(_RemoteHoursQuotaRequired, total=False):
    used_hours: float
    period_start: str
    period_end: str
    scope: str
    team_id: str
    seats: int
    per_seat_hours: float
    breakdown: RemoteHoursBreakdown
    members: List[RemoteHoursMember]


class CookieBotSlot(TypedDict, total=False):
    run_at_minute: int
    days_mask: int


class _CookieBotScheduleRequired(TypedDict):
    profile_id: str
    profile_name: str
    platform: str
    enabled: bool
    run_at_minute: int
    days_mask: int
    timezone: str
    preset: str
    max_minutes: int


class CookieBotSchedule(_CookieBotScheduleRequired, total=False):
    slots: List[CookieBotSlot]
    template_id: str
    sites: List[str]
    jitter_seconds: int
    sync_enabled: bool
    encrypted_sync: bool
    has_proxy: bool
    proxy_remote_reachable: bool
    touch_fingerprint: bool
    sticky_exit: bool
    profile_state_at: str
    blocked_by: str
    next_run_at: str
    last_run_at: str
    last_run_id: str
    owner_user_id: str
    owner_email: str
    updated_at: str


class CookieBotScheduleList(TypedDict, total=False):
    schedules: List[CookieBotSchedule]
    team_id: str
    scope: str


class _CookieBotConflictRequired(TypedDict):
    user_id: str
    email: str
    run_at_minute: int
    timezone: str
    days_mask: int
    enabled: bool


class CookieBotConflict(_CookieBotConflictRequired, total=False):
    overlaps: bool


class _CookieBotScheduleSavedRequired(TypedDict):
    schedule: CookieBotSchedule


class CookieBotScheduleSaved(_CookieBotScheduleSavedRequired, total=False):
    conflicts: List[CookieBotConflict]


class _CookieBotConflictCheckRequired(TypedDict):
    profile_id: str


class CookieBotConflictCheck(_CookieBotConflictCheckRequired, total=False):
    conflicts: List[CookieBotConflict]


class CookieBotScheduleDeleted(TypedDict):
    profile_id: str
    deleted: bool


class _CookieBotRunRequired(TypedDict):
    id: str
    profile_id: str
    trigger: str
    status: str
    scheduled_for: str


class CookieBotRun(_CookieBotRunRequired, total=False):
    profile_name: str
    user_id: str
    email: str
    team_id: str
    dispatch_after: str
    started_at: str
    ended_at: str
    max_minutes: int
    chunks_total: int
    chunk_index: int
    sites_total: int
    sites_visited: int
    sites_failed: int
    consent_dismissed: int
    billed_seconds: int
    outcome_code: str
    session_id: str


class CookieBotRunPage(TypedDict, total=False):
    runs: List[CookieBotRun]
    next_before: str


class _CookieBotRunStartedRequired(TypedDict):
    run: CookieBotRun


class CookieBotRunStarted(_CookieBotRunStartedRequired, total=False):
    session_id: str


class _CookieBotPresetRequired(TypedDict):
    id: str


class CookieBotPreset(_CookieBotPresetRequired, total=False):
    typical_minutes: int
    recommended: bool
    name: str
    description: str


class CookieBotPresetList(TypedDict, total=False):
    presets: List[CookieBotPreset]
    default_preset: str
    # `templates` and `limits` are whatever the server publishes; the app
    # forwards them without narrowing, so neither is spelled out here.
    templates: List[Dict[str, Any]]
    limits: Dict[str, Any]


class _CookieBotUsageMemberRequired(TypedDict):
    user_id: str
    email: str


class CookieBotUsageMember(_CookieBotUsageMemberRequired, total=False):
    role: str
    interactive_hours: float
    bot_hours: float
    used_hours: float
    sessions: int
    bot_runs: int
    bot_runs_failed: int


class _CookieBotUsageProfileRequired(TypedDict):
    profile_id: str


class CookieBotUsageProfile(_CookieBotUsageProfileRequired, total=False):
    profile_name: str
    owner_email: str
    bot_hours: float
    runs: int
    runs_failed: int
    last_run_at: str
    last_status: str


class _CookieBotUsageRequired(TypedDict):
    period: str


class CookieBotUsage(_CookieBotUsageRequired, total=False):
    period_start: str
    period_end: str
    team_id: str
    seats: int
    granted_hours: float
    used_hours: float
    remaining_hours: float
    members: List[CookieBotUsageMember]
    profiles: List[CookieBotUsageProfile]


class _BatchRunResultRequired(TypedDict):
    profile_id: str
    ok: bool


class BatchRunResult(_BatchRunResultRequired, total=False):
    remote_debugging_port: int
    error: str


class BatchRunResponse(TypedDict):
    results: List[BatchRunResult]


class _BatchStopResultRequired(TypedDict):
    profile_id: str
    ok: bool


class BatchStopResult(_BatchStopResultRequired, total=False):
    error: str


class BatchStopResponse(TypedDict):
    results: List[BatchStopResult]


class _ProxyAssignmentResultRequired(TypedDict):
    profile_id: str
    proxy_id: str
    ok: bool


class ProxyAssignmentResult(_ProxyAssignmentResultRequired, total=False):
    """``error`` is a ``{"code": ...}`` payload when ``ok`` is false."""

    error: str


class ProxyPair(TypedDict):
    """One profile, one proxy. The distribution applies exactly these pairs."""

    profile_id: str
    proxy_id: str


class DistributeProxiesResponse(TypedDict):
    results: List[ProxyAssignmentResult]


class ImportCookiesResponse(TypedDict):
    cookies_imported: int
    cookies_replaced: int
    errors: List[str]


class ImportProxiesResponse(TypedDict):
    imported_count: int
    skipped_count: int
    errors: List[str]
    proxies: List[ApiProxyResponse]


class DetectedProfile(TypedDict):
    browser: str
    mapped_browser: str
    name: str
    path: str
    description: str


class DetectedProfilesResponse(TypedDict):
    profiles: List[DetectedProfile]
    total: int


class _ImportProfileItemRequired(TypedDict):
    source_path: str
    new_profile_name: str


class ImportProfileItem(_ImportProfileItemRequired, total=False):
    """One item of ``import_profiles``.

    ``browser_type`` defaults to the app's own default when absent, and it is
    load-bearing: it picks which keychain entry unlocks the source's cookies
    and passwords.
    """

    browser_type: str
    proxy_id: str
    vpn_id: str
    allow_running: bool


class _ProfileImportItemResultRequired(TypedDict):
    name: str
    source_path: str
    status: str


class ProfileImportItemResult(_ProfileImportItemResultRequired, total=False):
    profile_id: str
    error: str
    report: Dict[str, Any]


class ProfileImportBatchResult(TypedDict):
    imported_count: int
    skipped_count: int
    failed_count: int
    results: List[ProfileImportItemResult]


class _ExtensionRequired(TypedDict):
    id: str
    name: str
    file_name: str
    file_type: str
    browser_compatibility: List[str]
    created_at: int
    updated_at: int
    source_kind: str


class Extension(_ExtensionRequired, total=False):
    manifest_name: str
    sync_enabled: bool
    last_sync: int
    version: str
    description: str
    author: str
    homepage_url: str
    linked_path: str


class _ExtensionGroupRequired(TypedDict):
    id: str
    name: str
    extension_ids: List[str]
    created_at: int
    updated_at: int


class ExtensionGroup(_ExtensionGroupRequired, total=False):
    sync_enabled: bool
    last_sync: int


class LocatorAttribute(TypedDict):
    name: str
    value: str


class LocatorDescription(TypedDict, total=False):
    """How an element is named without a CSS selector.

    At least one key must be set. Keys are the browser's own camelCase; the
    app also accepts ``name_contains`` and ``text_contains`` on input, but a
    locator handed back by ``agent_pick`` uses the spellings below, so reusing
    one verbatim is the reliable path.
    """

    role: str
    name: str
    nameContains: str
    text: str
    textContains: str
    attributes: List[LocatorAttribute]


class LocatorBounds(TypedDict):
    x: float
    y: float
    width: float
    height: float


class _LocatorCandidateRequired(TypedDict):
    role: str
    name: str
    text: str
    signature: str
    bounds: LocatorBounds


class LocatorCandidate(_LocatorCandidateRequired, total=False):
    backendNodeId: int
    value: str
    url: str
    attributes: List[LocatorAttribute]


class _LocatorResolutionRequired(TypedDict):
    matchCount: int
    # `match` is the key the app sends. It is a soft keyword in Python, so it
    # is spelled here exactly as it arrives.
    match: LocatorCandidate
    locator: LocatorDescription
    engine: str


class LocatorResolution(_LocatorResolutionRequired, total=False):
    backendNodeId: int


class _PerceptionNodeRequired(TypedDict):
    id: str
    frameId: str
    role: str
    x: float
    y: float
    width: float
    height: float
    inViewport: bool
    visible: bool
    focused: bool
    disabled: bool


class PerceptionNode(_PerceptionNodeRequired, total=False):
    parentId: str
    name: str
    text: str
    value: str
    checked: str
    expanded: bool
    scrollable: bool
    scrollContainerId: str


class _PerceptionFrameRequired(TypedDict):
    frameId: str
    url: str
    crossOrigin: bool


class PerceptionFrame(_PerceptionFrameRequired, total=False):
    parentFrameId: str


class PerceptionStats(TypedDict):
    totalNodes: int
    returnedNodes: int
    bytes: int
    elapsedMs: int
    framesVisited: int
    framesFailed: int


class _PerceptionPageRequired(TypedDict):
    snapshotId: str
    nodes: List[PerceptionNode]
    frames: List[PerceptionFrame]
    text: str
    truncated: bool
    stats: PerceptionStats
    engine: str


class PerceptionPage(_PerceptionPageRequired, total=False):
    cursor: str


class _ExtractionFieldRequired(TypedDict):
    key: str
    locator: LocatorDescription
    source: str


class ExtractionField(_ExtractionFieldRequired, total=False):
    """One output column. ``attribute`` is required when ``source`` is ``"attribute"``."""

    attribute: str


class ExtractionRow(TypedDict):
    index: int
    page: int
    values: Dict[str, Any]


class Extraction(TypedDict):
    rows: List[ExtractionRow]
    rowCount: int
    pageCount: int
    byteSize: int
    truncated: bool
    stopReason: str
    engine: str


class PickedElement(TypedDict):
    backendNodeId: int
    locator: LocatorDescription
    matchCount: int
    node: LocatorCandidate
    engine: str


class AgentClick(TypedDict):
    """What a click did. Note the snake_case body and the ``match`` key."""

    clicked: bool
    match: LocatorCandidate
    engine: str
    navigated: bool


class _AgentTypingRequired(TypedDict):
    typed: bool
    characters: int
    duration_ms: float
    engine: str
    match: LocatorCandidate


class AgentTyping(_AgentTypingRequired, total=False):
    """What a typing call did.

    ``corrections`` is absent on the fallback engine, which does not count its
    own mistypes.
    """

    corrections: int
