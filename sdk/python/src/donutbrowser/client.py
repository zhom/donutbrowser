"""A thin client for the Donut Browser local REST API.

Every method here is one request to one documented path. Nothing is cached,
nothing is retried, and nothing is invented: if a method exists below, the app
publishes that operation in its ``/openapi.json``.

Only the standard library is used, deliberately. See ``sdk/README.md``.
"""

from __future__ import annotations

import http.client
import json
import os
import socket
import threading
from types import TracebackType
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Type, Union
from urllib.parse import quote, urlencode, urlsplit

from .errors import DonutConnectionError, DonutError, error_for_status
from .models import (
    AgentClick,
    AgentTyping,
    ApiGroupResponse,
    ApiProfileResponse,
    ApiProfilesResponse,
    ApiProxyResponse,
    ApiRemoteSessionsResponse,
    ApiVpnExportResponse,
    ApiVpnResponse,
    BatchRunResponse,
    BatchStopResponse,
    CookieBotConflictCheck,
    CookieBotPresetList,
    CookieBotRun,
    CookieBotRunPage,
    CookieBotRunStarted,
    CookieBotSchedule,
    CookieBotScheduleDeleted,
    CookieBotScheduleList,
    CookieBotScheduleSaved,
    CookieBotUsage,
    DetectedProfilesResponse,
    DistributeProxiesResponse,
    DownloadBrowserResponse,
    Extension,
    ExtensionGroup,
    Extraction,
    ExtractionField,
    ImportCookiesResponse,
    ImportProfileItem,
    ImportProxiesResponse,
    LocatorDescription,
    LocatorResolution,
    PerceptionPage,
    PickedElement,
    ProfileImportBatchResult,
    ProxyPair,
    ProxySettings,
    RemoteHoursQuota,
    RemoteSessionState,
    RunProfileResponse,
    RunRemoteResponse,
    SetCloudSyncResponse,
    StopRemoteResponse,
    WayfernConfig,
)

__all__ = ["DonutClient", "RunSession", "DEFAULT_PORT", "DEFAULT_HOST"]

#: The port the app offers by default in Settings, Integrations, Local API.
DEFAULT_PORT = 10108

#: The API binds loopback only. It is never reachable from another machine.
DEFAULT_HOST = "127.0.0.1"

_JSON = "application/json"

QueryValue = Union[str, int, bool, None]


def _body(**fields: Any) -> Dict[str, Any]:
    """Drop every key the caller left unset.

    The app reads a missing key and an explicit ``null`` the same way, so
    omitting is always the faithful encoding of "the caller said nothing".
    Where a value has to be cleared, the app documents an empty string for it
    (``proxy_id=""`` detaches a proxy), and an empty string survives this.
    """
    return {name: value for name, value in fields.items() if value is not None}


def _query(**fields: QueryValue) -> Dict[str, str]:
    encoded: Dict[str, str] = {}
    for name, value in fields.items():
        if value is None:
            continue
        encoded[name] = "true" if value is True else "false" if value is False else str(value)
    return encoded


def _segment(value: str) -> str:
    """Escape one path segment so an id with a slash or a space cannot forge a path."""
    return quote(str(value), safe="")


class DonutClient:
    """A connection to one running Donut Browser.

    The local API must be switched on first: **Settings, Integrations, Local
    API, "Enable Local API Server"**. That screen shows the port and the
    authentication token to use here.

    Arguments win over the environment:

    * ``token`` falls back to ``DONUT_API_TOKEN``.
    * ``port`` falls back to ``DONUT_API_PORT``, then to ``10108``.
    * ``base_url``, when given, overrides ``host`` and ``port`` entirely.

    The client keeps one connection open and is safe to share between threads;
    requests on it are serialised.
    """

    def __init__(
        self,
        base_url: Optional[str] = None,
        token: Optional[str] = None,
        timeout: float = 30.0,
        *,
        host: Optional[str] = None,
        port: Optional[int] = None,
        env: Optional[Mapping[str, str]] = None,
    ) -> None:
        environment = os.environ if env is None else env

        resolved_token = token if token is not None else environment.get("DONUT_API_TOKEN")
        if not resolved_token:
            raise DonutError(
                "No API token. Pass token=..., or set DONUT_API_TOKEN. The token is "
                "shown in the app under Settings, Integrations, Local API."
            )

        if base_url:
            parts = urlsplit(base_url if "//" in base_url else f"http://{base_url}")
            if parts.scheme not in ("http", "https"):
                raise DonutError(f"base_url must be http or https, got {parts.scheme!r}")
            self.scheme = parts.scheme
            self.host = parts.hostname or DEFAULT_HOST
            self.port = parts.port or (443 if parts.scheme == "https" else 80)
            self._prefix = parts.path.rstrip("/")
        else:
            resolved_port = port
            if resolved_port is None:
                raw_port = environment.get("DONUT_API_PORT")
                if raw_port:
                    try:
                        resolved_port = int(raw_port)
                    except ValueError as invalid:
                        raise DonutError(
                            f"DONUT_API_PORT is not a number: {raw_port!r}"
                        ) from invalid
            self.scheme = "http"
            self.host = host or DEFAULT_HOST
            self.port = resolved_port if resolved_port is not None else DEFAULT_PORT
            self._prefix = ""

        self.token = resolved_token
        self.timeout = timeout
        self._lock = threading.Lock()
        self._connection: Optional[http.client.HTTPConnection] = None

    @property
    def base_url(self) -> str:
        return f"{self.scheme}://{self.host}:{self.port}{self._prefix}"

    def __enter__(self) -> "DonutClient":
        return self

    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        traceback: Optional[TracebackType],
    ) -> None:
        self.close()

    def close(self) -> None:
        """Drop the kept-alive connection. Calling a method again reopens one."""
        with self._lock:
            if self._connection is not None:
                self._connection.close()
                self._connection = None

    # ------------------------------------------------------------------
    # Transport
    # ------------------------------------------------------------------

    def _open(self) -> http.client.HTTPConnection:
        if self.scheme == "https":
            return http.client.HTTPSConnection(self.host, self.port, timeout=self.timeout)
        return http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)

    def _request(
        self,
        method: str,
        path: str,
        *,
        body: Optional[Any] = None,
        query: Optional[Mapping[str, str]] = None,
    ) -> Any:
        target = f"{self._prefix}{path}"
        if query:
            target = f"{target}?{urlencode(query)}"

        payload = None if body is None else json.dumps(body).encode("utf-8")
        headers = {"Authorization": f"Bearer {self.token}", "Accept": _JSON}
        if payload is not None:
            headers["Content-Type"] = _JSON

        with self._lock:
            # One retry, and only for a connection that was already open: a
            # kept-alive socket the app closed between calls fails on send,
            # and reporting that as "Donut is unreachable" would be a lie.
            for attempt in (0, 1):
                reused = self._connection is not None
                if self._connection is None:
                    self._connection = self._open()
                try:
                    self._connection.request(method, target, body=payload, headers=headers)
                    response = self._connection.getresponse()
                    status = response.status
                    raw = response.read()
                    response_headers = dict(response.getheaders())
                    if response.will_close:
                        self._connection.close()
                        self._connection = None
                    break
                except (http.client.HTTPException, socket.error) as failure:
                    self._connection.close()
                    self._connection = None
                    if reused and attempt == 0:
                        continue
                    raise DonutConnectionError(
                        f"Could not reach Donut Browser at {self.base_url} "
                        f"({method} {path}): {failure}. Is the app running with "
                        "Settings, Integrations, Local API switched on?"
                    ) from failure

        text = raw.decode("utf-8", errors="replace")
        if status >= 400:
            raise error_for_status(
                status, text, method=method, path=path, headers=response_headers
            )
        if status == 204 or not text.strip():
            return None
        try:
            return json.loads(text)
        except ValueError as invalid:
            raise DonutError(
                f"{method} {path} answered {status} with a body that is not JSON: {text[:200]!r}"
            ) from invalid

    # ------------------------------------------------------------------
    # Profiles
    # ------------------------------------------------------------------

    def list_profiles(self) -> ApiProfilesResponse:
        """GET /v1/profiles"""
        return self._request("GET", "/v1/profiles")

    def get_profile(self, profile_id: str) -> ApiProfileResponse:
        """GET /v1/profiles/{id}"""
        return self._request("GET", f"/v1/profiles/{_segment(profile_id)}")

    def create_profile(
        self,
        *,
        name: str,
        browser: str,
        version: Optional[str] = None,
        proxy_id: Optional[str] = None,
        vpn_id: Optional[str] = None,
        launch_hook: Optional[str] = None,
        release_type: Optional[str] = None,
        wayfern_config: Optional[WayfernConfig] = None,
        group_id: Optional[str] = None,
        tags: Optional[Sequence[str]] = None,
        ephemeral: Optional[bool] = None,
        temporary: Optional[bool] = None,
    ) -> ApiProfileResponse:
        """POST /v1/profiles

        ``browser`` must be ``"wayfern"``; anything else is refused with 400.
        ``version`` must already be downloaded, so omit it (or pass
        ``"latest"``) to take the newest local build.
        """
        return self._request(
            "POST",
            "/v1/profiles",
            body=_body(
                name=name,
                browser=browser,
                version=version,
                proxy_id=proxy_id,
                vpn_id=vpn_id,
                launch_hook=launch_hook,
                release_type=release_type,
                wayfern_config=wayfern_config,
                group_id=group_id,
                tags=list(tags) if tags is not None else None,
                ephemeral=ephemeral,
                temporary=temporary,
            ),
        )

    def update_profile(
        self,
        profile_id: str,
        *,
        name: Optional[str] = None,
        version: Optional[str] = None,
        proxy_id: Optional[str] = None,
        vpn_id: Optional[str] = None,
        launch_hook: Optional[str] = None,
        release_type: Optional[str] = None,
        group_id: Optional[str] = None,
        tags: Optional[Sequence[str]] = None,
        extension_group_id: Optional[str] = None,
        proxy_bypass_rules: Optional[Sequence[str]] = None,
        sync_mode: Optional[str] = None,
        clear_on_close: Optional[bool] = None,
    ) -> ApiProfileResponse:
        """PUT /v1/profiles/{id}

        A profile's browser engine is fixed at creation, so there is no
        ``browser`` argument. Pass ``proxy_id=""`` or ``vpn_id=""`` to detach
        one; leaving either unset changes nothing.
        """
        return self._request(
            "PUT",
            f"/v1/profiles/{_segment(profile_id)}",
            body=_body(
                name=name,
                version=version,
                proxy_id=proxy_id,
                vpn_id=vpn_id,
                launch_hook=launch_hook,
                release_type=release_type,
                group_id=group_id,
                tags=list(tags) if tags is not None else None,
                extension_group_id=extension_group_id,
                proxy_bypass_rules=(
                    list(proxy_bypass_rules) if proxy_bypass_rules is not None else None
                ),
                sync_mode=sync_mode,
                clear_on_close=clear_on_close,
            ),
        )

    def delete_profile(self, profile_id: str) -> None:
        """DELETE /v1/profiles/{id}"""
        self._request("DELETE", f"/v1/profiles/{_segment(profile_id)}")

    def run_profile(
        self,
        profile_id: str,
        *,
        url: Optional[str] = None,
        headless: Optional[bool] = None,
    ) -> RunProfileResponse:
        """POST /v1/profiles/{id}/run

        Prefer :meth:`run`, which stops the browser again when the block ends.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/run",
            body=_body(url=url, headless=headless),
        )

    def run_profile_remote(
        self, profile_id: str, *, url: Optional[str] = None
    ) -> RunRemoteResponse:
        """POST /v1/profiles/{id}/run-remote"""
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/run-remote",
            body=_body(url=url),
        )

    def set_profile_cloud_sync(self, profile_id: str, *, mode: str) -> SetCloudSyncResponse:
        """POST /v1/profiles/{id}/cloud-sync

        ``mode`` is ``"Disabled"``, ``"Regular"`` or ``"Encrypted"``. An
        encrypted profile cannot be launched remotely: its key never leaves
        this machine, so a remote host would download ciphertext.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/cloud-sync",
            body={"mode": mode},
        )

    def open_url(self, profile_id: str, url: str) -> None:
        """POST /v1/profiles/{id}/open-url"""
        self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/open-url",
            body={"url": url},
        )

    def kill_profile(self, profile_id: str) -> None:
        """POST /v1/profiles/{id}/kill

        A 503 here means the fleet could not be reached and the remote browser
        is *still running*, not that it stopped.
        """
        self._request("POST", f"/v1/profiles/{_segment(profile_id)}/kill")

    def batch_run_profiles(
        self,
        profile_ids: Sequence[str],
        *,
        url: Optional[str] = None,
        headless: Optional[bool] = None,
    ) -> BatchRunResponse:
        """POST /v1/profiles/batch/run

        Answers 200 even when some profiles failed; read ``results[].ok``.
        """
        return self._request(
            "POST",
            "/v1/profiles/batch/run",
            body=_body(profile_ids=list(profile_ids), url=url, headless=headless),
        )

    def batch_stop_profiles(self, profile_ids: Sequence[str]) -> BatchStopResponse:
        """POST /v1/profiles/batch/stop"""
        return self._request(
            "POST",
            "/v1/profiles/batch/stop",
            body={"profile_ids": list(profile_ids)},
        )

    def detect_import_profiles(self, *, folder: Optional[str] = None) -> DetectedProfilesResponse:
        """GET /v1/profiles/import/detect

        Without ``folder`` the app scans the default browser locations.
        """
        return self._request(
            "GET", "/v1/profiles/import/detect", query=_query(folder=folder)
        )

    def import_profiles(
        self,
        items: Iterable[ImportProfileItem],
        *,
        group_id: Optional[str] = None,
        duplicate_strategy: Optional[str] = None,
        wayfern_config: Optional[WayfernConfig] = None,
    ) -> ProfileImportBatchResult:
        """POST /v1/profiles/import

        ``duplicate_strategy`` is ``"skip"`` or ``"rename"`` (the default).
        Each item is isolated: one failure does not stop the rest.
        """
        return self._request(
            "POST",
            "/v1/profiles/import",
            body=_body(
                items=[dict(item) for item in items],
                group_id=group_id,
                duplicate_strategy=duplicate_strategy,
                wayfern_config=wayfern_config,
            ),
        )

    def import_profile_cookies(self, profile_id: str, *, content: str) -> ImportCookiesResponse:
        """POST /v1/profiles/{id}/cookies/import

        ``content`` is a raw cookie file. The format is detected: a JSON array
        in the Puppeteer style, or a Netscape ``cookies.txt``.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/cookies/import",
            body={"content": content},
        )

    def distribute_proxies(self, pairs: Sequence[ProxyPair]) -> DistributeProxiesResponse:
        """POST /v1/profiles/distribute-proxies

        Applies one proxy per profile. Configuration rather than automation, so
        it costs no automation quota. Answers 200 even when some pairs failed:
        read ``results[].ok``, and note that a profile whose browser is running
        is refused rather than moved.
        """
        return self._request(
            "POST",
            "/v1/profiles/distribute-proxies",
            body={"pairs": [dict(pair) for pair in pairs]},
        )

    # ------------------------------------------------------------------
    # Agent: reading and driving a running profile
    # ------------------------------------------------------------------

    def agent_perceive(
        self,
        profile_id: str,
        *,
        max_bytes: Optional[int] = None,
        budget_ms: Optional[int] = None,
        max_nodes: Optional[int] = None,
        include_text: Optional[bool] = None,
        viewport_only: Optional[bool] = None,
        text_order: Optional[str] = None,
        cursor: Optional[str] = None,
    ) -> PerceptionPage:
        """POST /v1/profiles/{id}/agent/perceive

        When the answer says ``truncated``, pass its ``cursor`` back to
        continue where it stopped.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/agent/perceive",
            body=_body(
                max_bytes=max_bytes,
                budget_ms=budget_ms,
                max_nodes=max_nodes,
                include_text=include_text,
                viewport_only=viewport_only,
                text_order=text_order,
                cursor=cursor,
            ),
        )

    def agent_resolve_locator(
        self,
        profile_id: str,
        *,
        locator: LocatorDescription,
        candidate_limit: Optional[int] = None,
    ) -> LocatorResolution:
        """POST /v1/profiles/{id}/agent/resolve-locator

        Succeeds only when the locator matches exactly one element.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/agent/resolve-locator",
            body=_body(locator=dict(locator), candidate_limit=candidate_limit),
        )

    def agent_click(
        self,
        profile_id: str,
        *,
        locator: LocatorDescription,
        button: Optional[str] = None,
        click_count: Optional[int] = None,
    ) -> AgentClick:
        """POST /v1/profiles/{id}/agent/click

        ``button`` is ``"left"`` (the default), ``"middle"``, ``"right"``,
        ``"back"`` or ``"forward"``.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/agent/click",
            body=_body(locator=dict(locator), button=button, click_count=click_count),
        )

    def agent_type(
        self,
        profile_id: str,
        *,
        locator: LocatorDescription,
        text: str,
        clear_first: Optional[bool] = None,
        typos: Optional[bool] = None,
        wpm: Optional[float] = None,
    ) -> AgentTyping:
        """POST /v1/profiles/{id}/agent/type

        ``wpm`` is honoured by the fallback engine only; a recent Wayfern types
        at the profile's own rhythm.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/agent/type",
            body=_body(
                locator=dict(locator),
                text=text,
                clear_first=clear_first,
                typos=typos,
                wpm=wpm,
            ),
        )

    def agent_extract(
        self,
        profile_id: str,
        *,
        container: LocatorDescription,
        field_map: Sequence[ExtractionField],
        next_page: Optional[LocatorDescription] = None,
        max_pages: Optional[int] = None,
        max_rows: Optional[int] = None,
        max_bytes: Optional[int] = None,
        max_nodes: Optional[int] = None,
        time_budget_ms: Optional[int] = None,
    ) -> Extraction:
        """POST /v1/profiles/{id}/agent/extract

        A container that matches nothing is a result with ``stopReason`` set
        to ``"no-container"``, not an error.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/agent/extract",
            body=_body(
                container=dict(container),
                field_map=[dict(field) for field in field_map],
                next_page=dict(next_page) if next_page is not None else None,
                max_pages=max_pages,
                max_rows=max_rows,
                max_bytes=max_bytes,
                max_nodes=max_nodes,
                time_budget_ms=time_budget_ms,
            ),
        )

    def agent_pick(self, profile_id: str, *, timeout_ms: Optional[int] = None) -> PickedElement:
        """POST /v1/profiles/{id}/agent/pick

        Arms a picker in the visible browser and waits for a human to click
        something. Nothing picked inside ``timeout_ms`` raises
        :class:`~donutbrowser.errors.RequestTimeout`.
        """
        return self._request(
            "POST",
            f"/v1/profiles/{_segment(profile_id)}/agent/pick",
            body=_body(timeout_ms=timeout_ms),
        )

    # ------------------------------------------------------------------
    # Remote sessions
    # ------------------------------------------------------------------

    def list_remote_sessions(self) -> ApiRemoteSessionsResponse:
        """GET /v1/remote-sessions"""
        return self._request("GET", "/v1/remote-sessions")

    def get_remote_session(self, session_id: str) -> RemoteSessionState:
        """GET /v1/remote-sessions/{id}"""
        return self._request("GET", f"/v1/remote-sessions/{_segment(session_id)}")

    def stop_remote_session(self, session_id: str) -> StopRemoteResponse:
        """DELETE /v1/remote-sessions/{id}"""
        return self._request("DELETE", f"/v1/remote-sessions/{_segment(session_id)}")

    def remote_session_cdp_url(self, session_id: str) -> str:
        """The websocket address of ``GET /v1/remote-sessions/{id}/cdp``.

        That path is a WebSocket upgrade, not a request this client can make,
        so it builds the address and leaves the socket to a websocket library.
        Send the same ``Authorization: Bearer`` header on the handshake.
        """
        scheme = "wss" if self.scheme == "https" else "ws"
        return (
            f"{scheme}://{self.host}:{self.port}{self._prefix}"
            f"/v1/remote-sessions/{_segment(session_id)}/cdp"
        )

    def get_remote_hours(self) -> RemoteHoursQuota:
        """GET /v1/remote-hours"""
        return self._request("GET", "/v1/remote-hours")

    # ------------------------------------------------------------------
    # Cookie bot
    # ------------------------------------------------------------------

    def list_cookie_bot_schedules(self, *, scope: Optional[str] = None) -> CookieBotScheduleList:
        """GET /v1/cookie-bot/schedules

        ``scope`` is ``"mine"`` (the default) or ``"team"``.
        """
        return self._request(
            "GET", "/v1/cookie-bot/schedules", query=_query(scope=scope)
        )

    def get_cookie_bot_schedule(self, profile_id: str) -> CookieBotSchedule:
        """GET /v1/cookie-bot/schedules/{profile_id}"""
        return self._request("GET", f"/v1/cookie-bot/schedules/{_segment(profile_id)}")

    def set_cookie_bot_schedule(
        self,
        profile_id: str,
        *,
        enabled: bool,
        run_at_minute: int,
        days_mask: int,
        timezone: str,
        preset: str,
        max_minutes: int,
        profile_name: Optional[str] = None,
        platform: Optional[str] = None,
        sites: Optional[Sequence[str]] = None,
        jitter_seconds: Optional[int] = None,
        acknowledge_conflict: Optional[bool] = None,
    ) -> CookieBotScheduleSaved:
        """PUT /v1/cookie-bot/schedules/{profile_id}

        ``run_at_minute`` is minutes past local midnight (0 to 1439) and
        ``days_mask`` is a weekday bitmask with bit 0 as Monday. A teammate
        already enrolling this profile makes the write 409 until
        ``acknowledge_conflict=True``.
        """
        return self._request(
            "PUT",
            f"/v1/cookie-bot/schedules/{_segment(profile_id)}",
            body=_body(
                profile_name=profile_name,
                platform=platform,
                enabled=enabled,
                run_at_minute=run_at_minute,
                days_mask=days_mask,
                timezone=timezone,
                preset=preset,
                max_minutes=max_minutes,
                sites=list(sites) if sites is not None else None,
                jitter_seconds=jitter_seconds,
                acknowledge_conflict=acknowledge_conflict,
            ),
        )

    def delete_cookie_bot_schedule(self, profile_id: str) -> CookieBotScheduleDeleted:
        """DELETE /v1/cookie-bot/schedules/{profile_id}"""
        return self._request("DELETE", f"/v1/cookie-bot/schedules/{_segment(profile_id)}")

    def get_cookie_bot_conflicts(
        self,
        profile_id: str,
        *,
        run_at_minute: Optional[int] = None,
        timezone: Optional[str] = None,
        days_mask: Optional[int] = None,
    ) -> CookieBotConflictCheck:
        """GET /v1/cookie-bot/conflicts

        A dry run: asks who else enrols this profile, without writing.
        """
        return self._request(
            "GET",
            "/v1/cookie-bot/conflicts",
            query=_query(
                profile_id=profile_id,
                run_at_minute=run_at_minute,
                timezone=timezone,
                days_mask=days_mask,
            ),
        )

    def list_cookie_bot_runs(
        self,
        *,
        profile_id: Optional[str] = None,
        scope: Optional[str] = None,
        limit: Optional[int] = None,
        before: Optional[str] = None,
    ) -> CookieBotRunPage:
        """GET /v1/cookie-bot/runs

        Newest first. ``before`` is the ``next_before`` of the previous page.
        """
        return self._request(
            "GET",
            "/v1/cookie-bot/runs",
            query=_query(profile_id=profile_id, scope=scope, limit=limit, before=before),
        )

    def start_cookie_bot_run(
        self, *, profile_id: str, max_minutes: Optional[int] = None
    ) -> CookieBotRunStarted:
        """POST /v1/cookie-bot/runs

        Answers 202: the run keeps going for minutes after this returns. The
        profile must already have a schedule, which is where the preset and
        the site list live.
        """
        return self._request(
            "POST",
            "/v1/cookie-bot/runs",
            body=_body(profile_id=profile_id, max_minutes=max_minutes),
        )

    def cancel_cookie_bot_run(self, run_id: str) -> CookieBotRun:
        """DELETE /v1/cookie-bot/runs/{run_id}"""
        return self._request("DELETE", f"/v1/cookie-bot/runs/{_segment(run_id)}")

    def list_cookie_bot_presets(self) -> CookieBotPresetList:
        """GET /v1/cookie-bot/presets"""
        return self._request("GET", "/v1/cookie-bot/presets")

    def get_cookie_bot_usage(self, *, period: Optional[str] = None) -> CookieBotUsage:
        """GET /v1/cookie-bot/usage

        ``period`` is ``YYYY-MM``, defaulting to the current UTC month.
        """
        return self._request("GET", "/v1/cookie-bot/usage", query=_query(period=period))

    # ------------------------------------------------------------------
    # Groups and tags
    # ------------------------------------------------------------------

    def list_groups(self) -> List[ApiGroupResponse]:
        """GET /v1/groups"""
        return self._request("GET", "/v1/groups")

    def get_group(self, group_id: str) -> ApiGroupResponse:
        """GET /v1/groups/{id}"""
        return self._request("GET", f"/v1/groups/{_segment(group_id)}")

    def create_group(self, *, name: str) -> ApiGroupResponse:
        """POST /v1/groups"""
        return self._request("POST", "/v1/groups", body={"name": name})

    def update_group(self, group_id: str, *, name: str) -> ApiGroupResponse:
        """PUT /v1/groups/{id}"""
        return self._request("PUT", f"/v1/groups/{_segment(group_id)}", body={"name": name})

    def delete_group(self, group_id: str) -> None:
        """DELETE /v1/groups/{id}"""
        self._request("DELETE", f"/v1/groups/{_segment(group_id)}")

    def list_tags(self) -> List[str]:
        """GET /v1/tags"""
        return self._request("GET", "/v1/tags")

    # ------------------------------------------------------------------
    # Proxies
    # ------------------------------------------------------------------

    def list_proxies(self) -> List[ApiProxyResponse]:
        """GET /v1/proxies"""
        return self._request("GET", "/v1/proxies")

    def get_proxy(self, proxy_id: str) -> ApiProxyResponse:
        """GET /v1/proxies/{id}"""
        return self._request("GET", f"/v1/proxies/{_segment(proxy_id)}")

    def create_proxy(self, *, name: str, proxy_settings: ProxySettings) -> ApiProxyResponse:
        """POST /v1/proxies"""
        return self._request(
            "POST",
            "/v1/proxies",
            body={"name": name, "proxy_settings": dict(proxy_settings)},
        )

    def update_proxy(
        self,
        proxy_id: str,
        *,
        name: Optional[str] = None,
        proxy_settings: Optional[ProxySettings] = None,
    ) -> ApiProxyResponse:
        """PUT /v1/proxies/{id}"""
        return self._request(
            "PUT",
            f"/v1/proxies/{_segment(proxy_id)}",
            body=_body(
                name=name,
                proxy_settings=dict(proxy_settings) if proxy_settings is not None else None,
            ),
        )

    def delete_proxy(self, proxy_id: str) -> None:
        """DELETE /v1/proxies/{id}"""
        self._request("DELETE", f"/v1/proxies/{_segment(proxy_id)}")

    def import_proxies(
        self,
        *,
        format: str,
        content: str,
        name_prefix: Optional[str] = None,
    ) -> ImportProxiesResponse:
        """POST /v1/proxies/import

        ``format`` is ``"txt"`` (one proxy per line) or ``"json"`` (a Donut
        proxy export).
        """
        return self._request(
            "POST",
            "/v1/proxies/import",
            body=_body(format=format, content=content, name_prefix=name_prefix),
        )

    # ------------------------------------------------------------------
    # VPNs
    # ------------------------------------------------------------------

    def list_vpns(self) -> List[ApiVpnResponse]:
        """GET /v1/vpns"""
        return self._request("GET", "/v1/vpns")

    def get_vpn(self, vpn_id: str) -> ApiVpnResponse:
        """GET /v1/vpns/{id}"""
        return self._request("GET", f"/v1/vpns/{_segment(vpn_id)}")

    def export_vpn(self, vpn_id: str) -> ApiVpnExportResponse:
        """GET /v1/vpns/{id}/export

        Returns the decrypted ``.conf`` text. Treat it as a secret.
        """
        return self._request("GET", f"/v1/vpns/{_segment(vpn_id)}/export")

    def import_vpn(
        self, *, content: str, filename: str, name: Optional[str] = None
    ) -> ApiVpnResponse:
        """POST /v1/vpns/import"""
        return self._request(
            "POST",
            "/v1/vpns/import",
            body=_body(content=content, filename=filename, name=name),
        )

    def create_vpn(self, *, name: str, vpn_type: str, config_data: str) -> ApiVpnResponse:
        """POST /v1/vpns

        ``vpn_type`` must be ``"WireGuard"``.
        """
        return self._request(
            "POST",
            "/v1/vpns",
            body={"name": name, "vpn_type": vpn_type, "config_data": config_data},
        )

    def update_vpn(self, vpn_id: str, *, name: str) -> ApiVpnResponse:
        """PUT /v1/vpns/{id}"""
        return self._request("PUT", f"/v1/vpns/{_segment(vpn_id)}", body={"name": name})

    def delete_vpn(self, vpn_id: str) -> None:
        """DELETE /v1/vpns/{id}"""
        self._request("DELETE", f"/v1/vpns/{_segment(vpn_id)}")

    # ------------------------------------------------------------------
    # Extensions
    # ------------------------------------------------------------------

    def list_extensions(self) -> List[Extension]:
        """GET /v1/extensions"""
        return self._request("GET", "/v1/extensions")

    def get_extension(self, extension_id: str) -> Extension:
        """GET /v1/extensions/{id}"""
        return self._request("GET", f"/v1/extensions/{_segment(extension_id)}")

    def create_extension(
        self,
        *,
        name: Optional[str] = None,
        file_name: Optional[str] = None,
        file_data_base64: Optional[str] = None,
        source_path: Optional[str] = None,
        link: Optional[bool] = None,
    ) -> Extension:
        """POST /v1/extensions

        Either upload bytes (``file_name`` plus ``file_data_base64``) or point
        at a path on this machine (``source_path``). Answers 201.
        """
        return self._request(
            "POST",
            "/v1/extensions",
            body=_body(
                name=name,
                file_name=file_name,
                file_data_base64=file_data_base64,
                source_path=source_path,
                link=link,
            ),
        )

    def update_extension(
        self,
        extension_id: str,
        *,
        name: Optional[str] = None,
        file_name: Optional[str] = None,
        file_data_base64: Optional[str] = None,
        source_path: Optional[str] = None,
        link: Optional[bool] = None,
    ) -> Extension:
        """PUT /v1/extensions/{id}"""
        return self._request(
            "PUT",
            f"/v1/extensions/{_segment(extension_id)}",
            body=_body(
                name=name,
                file_name=file_name,
                file_data_base64=file_data_base64,
                source_path=source_path,
                link=link,
            ),
        )

    def delete_extension(self, extension_id: str) -> None:
        """DELETE /v1/extensions/{id}"""
        self._request("DELETE", f"/v1/extensions/{_segment(extension_id)}")

    def list_extension_groups(self) -> List[ExtensionGroup]:
        """GET /v1/extension-groups"""
        return self._request("GET", "/v1/extension-groups")

    def get_extension_group(self, group_id: str) -> ExtensionGroup:
        """GET /v1/extension-groups/{id}"""
        return self._request("GET", f"/v1/extension-groups/{_segment(group_id)}")

    def create_extension_group(self, *, name: str) -> ExtensionGroup:
        """POST /v1/extension-groups. Answers 201."""
        return self._request("POST", "/v1/extension-groups", body={"name": name})

    def update_extension_group(
        self,
        group_id: str,
        *,
        name: Optional[str] = None,
        extension_ids: Optional[Sequence[str]] = None,
    ) -> ExtensionGroup:
        """PUT /v1/extension-groups/{id}

        ``extension_ids`` replaces the whole membership list. To change one
        member, use :meth:`add_extension_to_group` or
        :meth:`remove_extension_from_group`.
        """
        return self._request(
            "PUT",
            f"/v1/extension-groups/{_segment(group_id)}",
            body=_body(
                name=name,
                extension_ids=list(extension_ids) if extension_ids is not None else None,
            ),
        )

    def delete_extension_group(self, group_id: str) -> None:
        """DELETE /v1/extension-groups/{id}"""
        self._request("DELETE", f"/v1/extension-groups/{_segment(group_id)}")

    def add_extension_to_group(self, group_id: str, extension_id: str) -> ExtensionGroup:
        """POST /v1/extension-groups/{id}/extensions/{extension_id}"""
        return self._request(
            "POST",
            f"/v1/extension-groups/{_segment(group_id)}/extensions/{_segment(extension_id)}",
        )

    def remove_extension_from_group(self, group_id: str, extension_id: str) -> ExtensionGroup:
        """DELETE /v1/extension-groups/{id}/extensions/{extension_id}"""
        return self._request(
            "DELETE",
            f"/v1/extension-groups/{_segment(group_id)}/extensions/{_segment(extension_id)}",
        )

    # ------------------------------------------------------------------
    # Browsers
    # ------------------------------------------------------------------

    def download_browser(self, *, browser: str, version: str) -> DownloadBrowserResponse:
        """POST /v1/browsers/download

        Returns once the build is on disk, so give this a long ``timeout``.
        A 409 means the same version is already downloading.
        """
        return self._request(
            "POST",
            "/v1/browsers/download",
            body={"browser": browser, "version": version},
        )

    def list_browser_versions(self, browser: str) -> List[str]:
        """GET /v1/browsers/{browser}/versions"""
        return self._request("GET", f"/v1/browsers/{_segment(browser)}/versions")

    def is_browser_downloaded(self, browser: str, version: str) -> bool:
        """GET /v1/browsers/{browser}/versions/{version}/downloaded"""
        return self._request(
            "GET",
            f"/v1/browsers/{_segment(browser)}/versions/{_segment(version)}/downloaded",
        )

    # ------------------------------------------------------------------
    # Convenience
    # ------------------------------------------------------------------

    def run(
        self,
        profile_id: str,
        *,
        url: Optional[str] = None,
        headless: Optional[bool] = None,
    ) -> "RunSession":
        """Launch a profile for the length of a ``with`` block, then stop it.

        ::

            with client.run(profile_id, url="https://example.com", headless=True) as session:
                print(session.cdp_url)

        The browser starts when the block is entered and is stopped when it
        ends, including when the body raises.
        """
        return RunSession(self, profile_id, url=url, headless=headless)


class RunSession:
    """A profile launched for one ``with`` block.

    Nothing starts until the block is entered, so a session that is built but
    never entered leaves no browser behind.
    """

    def __init__(
        self,
        client: DonutClient,
        profile_id: str,
        *,
        url: Optional[str] = None,
        headless: Optional[bool] = None,
    ) -> None:
        self.client = client
        self.profile_id = profile_id
        self._url = url
        self._headless = headless

        #: The whole body of ``POST /v1/profiles/{id}/run``, once entered.
        self.response: Optional[RunProfileResponse] = None
        #: The browser's CDP port, once entered.
        self.remote_debugging_port: Optional[int] = None
        #: Whether the browser actually started headless.
        self.headless: Optional[bool] = None
        #: A failure while stopping the browser, kept rather than raised when
        #: the block itself was already failing.
        self.cleanup_error: Optional[DonutError] = None

    @property
    def cdp_url(self) -> str:
        """The browser's DevTools endpoint, e.g. ``http://127.0.0.1:9222``.

        ``GET {cdp_url}/json/version`` returns the ``webSocketDebuggerUrl`` a
        CDP library connects to.
        """
        if self.remote_debugging_port is None:
            raise DonutError("The session is not running: enter the `with` block first.")
        return f"http://{self.client.host}:{self.remote_debugging_port}"

    def __enter__(self) -> "RunSession":
        response = self.client.run_profile(
            self.profile_id, url=self._url, headless=self._headless
        )
        self.response = response
        self.remote_debugging_port = response["remote_debugging_port"]
        self.headless = response["headless"]
        return self

    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        traceback: Optional[TracebackType],
    ) -> bool:
        try:
            self.client.kill_profile(self.profile_id)
        except DonutError as failure:
            self.cleanup_error = failure
            # A failure to stop must never hide why the block failed. When the
            # block was fine, the failure is the only news there is, so it is
            # raised; otherwise it stays readable on `cleanup_error`.
            if exc_type is None:
                raise
        return False
