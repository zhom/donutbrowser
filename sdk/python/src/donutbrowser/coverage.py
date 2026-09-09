"""Which app operation each client method wraps.

This table is the SDK's half of a two-sided check. ``sdk/api-paths.json`` holds
every operation the desktop app publishes, generated from
``src-tauri/src/api_server.rs``. The test suite asserts the two agree exactly in
both directions, so:

* an endpoint added to the app fails the SDK tests until it is wrapped here, or
  listed in :data:`OMITTED` with a reason, and
* an entry here that the app no longer publishes fails too.

The same table is mirrored in the Node package, and the same snapshot proves it.
"""

from __future__ import annotations

from typing import Dict, Tuple

__all__ = ["OPERATIONS", "OMITTED"]

Operation = Tuple[str, str]

#: ``(method, path template)`` to the name of the :class:`~donutbrowser.DonutClient`
#: method that calls it.
OPERATIONS: Dict[Operation, str] = {
    ("POST", "/v1/browsers/download"): "download_browser",
    ("GET", "/v1/browsers/{browser}/versions"): "list_browser_versions",
    ("GET", "/v1/browsers/{browser}/versions/{version}/downloaded"): "is_browser_downloaded",
    ("GET", "/v1/cookie-bot/conflicts"): "get_cookie_bot_conflicts",
    ("GET", "/v1/cookie-bot/presets"): "list_cookie_bot_presets",
    ("GET", "/v1/cookie-bot/runs"): "list_cookie_bot_runs",
    ("POST", "/v1/cookie-bot/runs"): "start_cookie_bot_run",
    ("DELETE", "/v1/cookie-bot/runs/{run_id}"): "cancel_cookie_bot_run",
    ("GET", "/v1/cookie-bot/schedules"): "list_cookie_bot_schedules",
    ("DELETE", "/v1/cookie-bot/schedules/{profile_id}"): "delete_cookie_bot_schedule",
    ("GET", "/v1/cookie-bot/schedules/{profile_id}"): "get_cookie_bot_schedule",
    ("PUT", "/v1/cookie-bot/schedules/{profile_id}"): "set_cookie_bot_schedule",
    ("GET", "/v1/cookie-bot/usage"): "get_cookie_bot_usage",
    ("GET", "/v1/extension-groups"): "list_extension_groups",
    ("POST", "/v1/extension-groups"): "create_extension_group",
    ("DELETE", "/v1/extension-groups/{id}"): "delete_extension_group",
    ("GET", "/v1/extension-groups/{id}"): "get_extension_group",
    ("PUT", "/v1/extension-groups/{id}"): "update_extension_group",
    (
        "DELETE",
        "/v1/extension-groups/{id}/extensions/{extension_id}",
    ): "remove_extension_from_group",
    ("POST", "/v1/extension-groups/{id}/extensions/{extension_id}"): "add_extension_to_group",
    ("GET", "/v1/extensions"): "list_extensions",
    ("POST", "/v1/extensions"): "create_extension",
    ("DELETE", "/v1/extensions/{id}"): "delete_extension",
    ("GET", "/v1/extensions/{id}"): "get_extension",
    ("PUT", "/v1/extensions/{id}"): "update_extension",
    ("GET", "/v1/groups"): "list_groups",
    ("POST", "/v1/groups"): "create_group",
    ("DELETE", "/v1/groups/{id}"): "delete_group",
    ("GET", "/v1/groups/{id}"): "get_group",
    ("PUT", "/v1/groups/{id}"): "update_group",
    ("GET", "/v1/profiles"): "list_profiles",
    ("POST", "/v1/profiles"): "create_profile",
    ("POST", "/v1/profiles/batch/run"): "batch_run_profiles",
    ("POST", "/v1/profiles/batch/stop"): "batch_stop_profiles",
    ("POST", "/v1/profiles/distribute-proxies"): "distribute_proxies",
    ("POST", "/v1/profiles/import"): "import_profiles",
    ("GET", "/v1/profiles/import/detect"): "detect_import_profiles",
    ("DELETE", "/v1/profiles/{id}"): "delete_profile",
    ("GET", "/v1/profiles/{id}"): "get_profile",
    ("PUT", "/v1/profiles/{id}"): "update_profile",
    ("POST", "/v1/profiles/{id}/agent/click"): "agent_click",
    ("POST", "/v1/profiles/{id}/agent/extract"): "agent_extract",
    ("POST", "/v1/profiles/{id}/agent/perceive"): "agent_perceive",
    ("POST", "/v1/profiles/{id}/agent/pick"): "agent_pick",
    ("POST", "/v1/profiles/{id}/agent/resolve-locator"): "agent_resolve_locator",
    ("POST", "/v1/profiles/{id}/agent/type"): "agent_type",
    ("POST", "/v1/profiles/{id}/cloud-sync"): "set_profile_cloud_sync",
    ("POST", "/v1/profiles/{id}/cookies/import"): "import_profile_cookies",
    ("POST", "/v1/profiles/{id}/kill"): "kill_profile",
    ("POST", "/v1/profiles/{id}/open-url"): "open_url",
    ("POST", "/v1/profiles/{id}/run"): "run_profile",
    ("POST", "/v1/profiles/{id}/run-remote"): "run_profile_remote",
    ("GET", "/v1/proxies"): "list_proxies",
    ("POST", "/v1/proxies"): "create_proxy",
    ("POST", "/v1/proxies/import"): "import_proxies",
    ("DELETE", "/v1/proxies/{id}"): "delete_proxy",
    ("GET", "/v1/proxies/{id}"): "get_proxy",
    ("PUT", "/v1/proxies/{id}"): "update_proxy",
    ("GET", "/v1/remote-hours"): "get_remote_hours",
    ("GET", "/v1/remote-sessions"): "list_remote_sessions",
    ("DELETE", "/v1/remote-sessions/{id}"): "stop_remote_session",
    ("GET", "/v1/remote-sessions/{id}"): "get_remote_session",
    ("GET", "/v1/tags"): "list_tags",
    ("GET", "/v1/vpns"): "list_vpns",
    ("POST", "/v1/vpns"): "create_vpn",
    ("POST", "/v1/vpns/import"): "import_vpn",
    ("DELETE", "/v1/vpns/{id}"): "delete_vpn",
    ("GET", "/v1/vpns/{id}"): "get_vpn",
    ("PUT", "/v1/vpns/{id}"): "update_vpn",
    ("GET", "/v1/vpns/{id}/export"): "export_vpn",
}

#: Operations this SDK deliberately does not call, and why.
OMITTED: Dict[Operation, str] = {
    (
        "GET",
        "/v1/remote-sessions/{id}/cdp",
    ): (
        "A WebSocket upgrade, not a request. An HTTP client cannot speak it, and "
        "bundling a websocket implementation would end this package's zero-dependency "
        "promise for one endpoint. DonutClient.remote_session_cdp_url() builds the "
        "ws:// address so a websocket library of the caller's choosing can connect, "
        "sending the same Authorization: Bearer header on the handshake."
    ),
}
