"""Every client method sends exactly the request the app documents.

The table below is the whole public surface. Each row names a method, the
arguments to call it with, and the request that must appear on the wire: the
verb, the concrete path, the query string and the JSON body. ``operation`` is
the path template the app publishes, which ties this file to
``donutbrowser.coverage.OPERATIONS`` and, through it, to ``sdk/api-paths.json``.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional, Tuple

import pytest
from fake_donut import FakeDonut

from donutbrowser import DonutClient
from donutbrowser.coverage import OPERATIONS

Case = Tuple[
    str,  # client method
    Tuple[Any, ...],  # positional arguments
    Dict[str, Any],  # keyword arguments
    str,  # expected verb
    str,  # expected concrete path
    Optional[Dict[str, Any]],  # expected JSON body, or None for no body
    Dict[str, str],  # expected query string
    str,  # operation template, as published by the app
]

LOCATOR = {"role": "button", "name": "Sign in"}

CASES: List[Case] = [
    # -- profiles ----------------------------------------------------------
    ("list_profiles", (), {}, "GET", "/v1/profiles", None, {}, "/v1/profiles"),
    ("get_profile", ("p1",), {}, "GET", "/v1/profiles/p1", None, {}, "/v1/profiles/{id}"),
    (
        "create_profile",
        (),
        {"name": "Shopper", "browser": "wayfern", "tags": ["eu"], "ephemeral": True},
        "POST",
        "/v1/profiles",
        {"name": "Shopper", "browser": "wayfern", "tags": ["eu"], "ephemeral": True},
        {},
        "/v1/profiles",
    ),
    (
        "create_profile",
        (),
        {"name": "Bare", "browser": "wayfern"},
        "POST",
        "/v1/profiles",
        {"name": "Bare", "browser": "wayfern"},
        {},
        "/v1/profiles",
    ),
    (
        "update_profile",
        ("p1",),
        {"name": "Renamed", "proxy_id": "", "clear_on_close": False},
        "PUT",
        "/v1/profiles/p1",
        {"name": "Renamed", "proxy_id": "", "clear_on_close": False},
        {},
        "/v1/profiles/{id}",
    ),
    ("delete_profile", ("p1",), {}, "DELETE", "/v1/profiles/p1", None, {}, "/v1/profiles/{id}"),
    (
        "run_profile",
        ("p1",),
        {"url": "https://example.com", "headless": True},
        "POST",
        "/v1/profiles/p1/run",
        {"url": "https://example.com", "headless": True},
        {},
        "/v1/profiles/{id}/run",
    ),
    (
        "run_profile_remote",
        ("p1",),
        {"url": "https://example.com"},
        "POST",
        "/v1/profiles/p1/run-remote",
        {"url": "https://example.com"},
        {},
        "/v1/profiles/{id}/run-remote",
    ),
    (
        "set_profile_cloud_sync",
        ("p1",),
        {"mode": "Regular"},
        "POST",
        "/v1/profiles/p1/cloud-sync",
        {"mode": "Regular"},
        {},
        "/v1/profiles/{id}/cloud-sync",
    ),
    (
        "open_url",
        ("p1", "https://example.com/page"),
        {},
        "POST",
        "/v1/profiles/p1/open-url",
        {"url": "https://example.com/page"},
        {},
        "/v1/profiles/{id}/open-url",
    ),
    (
        "kill_profile",
        ("p1",),
        {},
        "POST",
        "/v1/profiles/p1/kill",
        None,
        {},
        "/v1/profiles/{id}/kill",
    ),
    (
        "batch_run_profiles",
        (["p1", "p2"],),
        {"headless": False},
        "POST",
        "/v1/profiles/batch/run",
        {"profile_ids": ["p1", "p2"], "headless": False},
        {},
        "/v1/profiles/batch/run",
    ),
    (
        "batch_stop_profiles",
        (["p1", "p2"],),
        {},
        "POST",
        "/v1/profiles/batch/stop",
        {"profile_ids": ["p1", "p2"]},
        {},
        "/v1/profiles/batch/stop",
    ),
    (
        "distribute_proxies",
        ([{"profile_id": "p1", "proxy_id": "x1"}, {"profile_id": "p2", "proxy_id": "x2"}],),
        {},
        "POST",
        "/v1/profiles/distribute-proxies",
        {
            "pairs": [
                {"profile_id": "p1", "proxy_id": "x1"},
                {"profile_id": "p2", "proxy_id": "x2"},
            ]
        },
        {},
        "/v1/profiles/distribute-proxies",
    ),
    (
        "detect_import_profiles",
        (),
        {"folder": "/Users/x/Chrome"},
        "GET",
        "/v1/profiles/import/detect",
        None,
        {"folder": "/Users/x/Chrome"},
        "/v1/profiles/import/detect",
    ),
    (
        "detect_import_profiles",
        (),
        {},
        "GET",
        "/v1/profiles/import/detect",
        None,
        {},
        "/v1/profiles/import/detect",
    ),
    (
        "import_profiles",
        ([{"source_path": "/tmp/src", "new_profile_name": "Imported"}],),
        {"duplicate_strategy": "skip"},
        "POST",
        "/v1/profiles/import",
        {
            "items": [{"source_path": "/tmp/src", "new_profile_name": "Imported"}],
            "duplicate_strategy": "skip",
        },
        {},
        "/v1/profiles/import",
    ),
    (
        "import_profile_cookies",
        ("p1",),
        {"content": "[]"},
        "POST",
        "/v1/profiles/p1/cookies/import",
        {"content": "[]"},
        {},
        "/v1/profiles/{id}/cookies/import",
    ),
    # -- agent -------------------------------------------------------------
    (
        "agent_perceive",
        ("p1",),
        {"viewport_only": True, "max_bytes": 2048},
        "POST",
        "/v1/profiles/p1/agent/perceive",
        {"max_bytes": 2048, "viewport_only": True},
        {},
        "/v1/profiles/{id}/agent/perceive",
    ),
    (
        "agent_perceive",
        ("p1",),
        {},
        "POST",
        "/v1/profiles/p1/agent/perceive",
        {},
        {},
        "/v1/profiles/{id}/agent/perceive",
    ),
    (
        "agent_resolve_locator",
        ("p1",),
        {"locator": LOCATOR, "candidate_limit": 5},
        "POST",
        "/v1/profiles/p1/agent/resolve-locator",
        {"locator": LOCATOR, "candidate_limit": 5},
        {},
        "/v1/profiles/{id}/agent/resolve-locator",
    ),
    (
        "agent_click",
        ("p1",),
        {"locator": LOCATOR, "button": "right", "click_count": 2},
        "POST",
        "/v1/profiles/p1/agent/click",
        {"locator": LOCATOR, "button": "right", "click_count": 2},
        {},
        "/v1/profiles/{id}/agent/click",
    ),
    (
        "agent_type",
        ("p1",),
        {"locator": LOCATOR, "text": "hello", "clear_first": False, "wpm": 55.0},
        "POST",
        "/v1/profiles/p1/agent/type",
        {"locator": LOCATOR, "text": "hello", "clear_first": False, "wpm": 55.0},
        {},
        "/v1/profiles/{id}/agent/type",
    ),
    (
        "agent_extract",
        ("p1",),
        {
            "container": {"role": "listitem"},
            "field_map": [{"key": "title", "locator": {"role": "heading"}, "source": "text"}],
            "max_pages": 3,
        },
        "POST",
        "/v1/profiles/p1/agent/extract",
        {
            "container": {"role": "listitem"},
            "field_map": [{"key": "title", "locator": {"role": "heading"}, "source": "text"}],
            "max_pages": 3,
        },
        {},
        "/v1/profiles/{id}/agent/extract",
    ),
    (
        "agent_pick",
        ("p1",),
        {"timeout_ms": 15000},
        "POST",
        "/v1/profiles/p1/agent/pick",
        {"timeout_ms": 15000},
        {},
        "/v1/profiles/{id}/agent/pick",
    ),
    # -- remote sessions ---------------------------------------------------
    (
        "list_remote_sessions",
        (),
        {},
        "GET",
        "/v1/remote-sessions",
        None,
        {},
        "/v1/remote-sessions",
    ),
    (
        "get_remote_session",
        ("s1",),
        {},
        "GET",
        "/v1/remote-sessions/s1",
        None,
        {},
        "/v1/remote-sessions/{id}",
    ),
    (
        "stop_remote_session",
        ("s1",),
        {},
        "DELETE",
        "/v1/remote-sessions/s1",
        None,
        {},
        "/v1/remote-sessions/{id}",
    ),
    ("get_remote_hours", (), {}, "GET", "/v1/remote-hours", None, {}, "/v1/remote-hours"),
    # -- cookie bot --------------------------------------------------------
    (
        "list_cookie_bot_schedules",
        (),
        {"scope": "team"},
        "GET",
        "/v1/cookie-bot/schedules",
        None,
        {"scope": "team"},
        "/v1/cookie-bot/schedules",
    ),
    (
        "get_cookie_bot_schedule",
        ("p1",),
        {},
        "GET",
        "/v1/cookie-bot/schedules/p1",
        None,
        {},
        "/v1/cookie-bot/schedules/{profile_id}",
    ),
    (
        "set_cookie_bot_schedule",
        ("p1",),
        {
            "enabled": True,
            "run_at_minute": 120,
            "days_mask": 31,
            "timezone": "Europe/Berlin",
            "preset": "steady",
            "max_minutes": 45,
            "sites": ["https://example.com"],
            "acknowledge_conflict": True,
        },
        "PUT",
        "/v1/cookie-bot/schedules/p1",
        {
            "enabled": True,
            "run_at_minute": 120,
            "days_mask": 31,
            "timezone": "Europe/Berlin",
            "preset": "steady",
            "max_minutes": 45,
            "sites": ["https://example.com"],
            "acknowledge_conflict": True,
        },
        {},
        "/v1/cookie-bot/schedules/{profile_id}",
    ),
    (
        "delete_cookie_bot_schedule",
        ("p1",),
        {},
        "DELETE",
        "/v1/cookie-bot/schedules/p1",
        None,
        {},
        "/v1/cookie-bot/schedules/{profile_id}",
    ),
    (
        "get_cookie_bot_conflicts",
        ("p1",),
        {"run_at_minute": 90, "timezone": "UTC", "days_mask": 7},
        "GET",
        "/v1/cookie-bot/conflicts",
        None,
        {"profile_id": "p1", "run_at_minute": "90", "timezone": "UTC", "days_mask": "7"},
        "/v1/cookie-bot/conflicts",
    ),
    (
        "list_cookie_bot_runs",
        (),
        {"profile_id": "p1", "limit": 10, "before": "cursor-1"},
        "GET",
        "/v1/cookie-bot/runs",
        None,
        {"profile_id": "p1", "limit": "10", "before": "cursor-1"},
        "/v1/cookie-bot/runs",
    ),
    (
        "start_cookie_bot_run",
        (),
        {"profile_id": "p1", "max_minutes": 30},
        "POST",
        "/v1/cookie-bot/runs",
        {"profile_id": "p1", "max_minutes": 30},
        {},
        "/v1/cookie-bot/runs",
    ),
    (
        "cancel_cookie_bot_run",
        ("r1",),
        {},
        "DELETE",
        "/v1/cookie-bot/runs/r1",
        None,
        {},
        "/v1/cookie-bot/runs/{run_id}",
    ),
    (
        "list_cookie_bot_presets",
        (),
        {},
        "GET",
        "/v1/cookie-bot/presets",
        None,
        {},
        "/v1/cookie-bot/presets",
    ),
    (
        "get_cookie_bot_usage",
        (),
        {"period": "2026-08"},
        "GET",
        "/v1/cookie-bot/usage",
        None,
        {"period": "2026-08"},
        "/v1/cookie-bot/usage",
    ),
    # -- groups and tags ---------------------------------------------------
    ("list_groups", (), {}, "GET", "/v1/groups", None, {}, "/v1/groups"),
    ("get_group", ("g1",), {}, "GET", "/v1/groups/g1", None, {}, "/v1/groups/{id}"),
    ("create_group", (), {"name": "Retail"}, "POST", "/v1/groups", {"name": "Retail"}, {}, "/v1/groups"),
    (
        "update_group",
        ("g1",),
        {"name": "Retail EU"},
        "PUT",
        "/v1/groups/g1",
        {"name": "Retail EU"},
        {},
        "/v1/groups/{id}",
    ),
    ("delete_group", ("g1",), {}, "DELETE", "/v1/groups/g1", None, {}, "/v1/groups/{id}"),
    ("list_tags", (), {}, "GET", "/v1/tags", None, {}, "/v1/tags"),
    # -- proxies -----------------------------------------------------------
    ("list_proxies", (), {}, "GET", "/v1/proxies", None, {}, "/v1/proxies"),
    ("get_proxy", ("x1",), {}, "GET", "/v1/proxies/x1", None, {}, "/v1/proxies/{id}"),
    (
        "create_proxy",
        (),
        {"name": "EU", "proxy_settings": {"proxy_type": "http", "host": "h", "port": 8080}},
        "POST",
        "/v1/proxies",
        {"name": "EU", "proxy_settings": {"proxy_type": "http", "host": "h", "port": 8080}},
        {},
        "/v1/proxies",
    ),
    (
        "update_proxy",
        ("x1",),
        {"name": "EU 2"},
        "PUT",
        "/v1/proxies/x1",
        {"name": "EU 2"},
        {},
        "/v1/proxies/{id}",
    ),
    ("delete_proxy", ("x1",), {}, "DELETE", "/v1/proxies/x1", None, {}, "/v1/proxies/{id}"),
    (
        "import_proxies",
        (),
        {"format": "txt", "content": "h:1:u:p", "name_prefix": "EU"},
        "POST",
        "/v1/proxies/import",
        {"format": "txt", "content": "h:1:u:p", "name_prefix": "EU"},
        {},
        "/v1/proxies/import",
    ),
    # -- vpns --------------------------------------------------------------
    ("list_vpns", (), {}, "GET", "/v1/vpns", None, {}, "/v1/vpns"),
    ("get_vpn", ("v1",), {}, "GET", "/v1/vpns/v1", None, {}, "/v1/vpns/{id}"),
    ("export_vpn", ("v1",), {}, "GET", "/v1/vpns/v1/export", None, {}, "/v1/vpns/{id}/export"),
    (
        "import_vpn",
        (),
        {"content": "[Interface]", "filename": "eu.conf"},
        "POST",
        "/v1/vpns/import",
        {"content": "[Interface]", "filename": "eu.conf"},
        {},
        "/v1/vpns/import",
    ),
    (
        "create_vpn",
        (),
        {"name": "EU", "vpn_type": "WireGuard", "config_data": "[Interface]"},
        "POST",
        "/v1/vpns",
        {"name": "EU", "vpn_type": "WireGuard", "config_data": "[Interface]"},
        {},
        "/v1/vpns",
    ),
    (
        "update_vpn",
        ("v1",),
        {"name": "EU 2"},
        "PUT",
        "/v1/vpns/v1",
        {"name": "EU 2"},
        {},
        "/v1/vpns/{id}",
    ),
    ("delete_vpn", ("v1",), {}, "DELETE", "/v1/vpns/v1", None, {}, "/v1/vpns/{id}"),
    # -- extensions --------------------------------------------------------
    ("list_extensions", (), {}, "GET", "/v1/extensions", None, {}, "/v1/extensions"),
    ("get_extension", ("e1",), {}, "GET", "/v1/extensions/e1", None, {}, "/v1/extensions/{id}"),
    (
        "create_extension",
        (),
        {"name": "Blocker", "file_name": "b.crx", "file_data_base64": "AAAA"},
        "POST",
        "/v1/extensions",
        {"name": "Blocker", "file_name": "b.crx", "file_data_base64": "AAAA"},
        {},
        "/v1/extensions",
    ),
    (
        "update_extension",
        ("e1",),
        {"name": "Blocker 2", "link": True},
        "PUT",
        "/v1/extensions/e1",
        {"name": "Blocker 2", "link": True},
        {},
        "/v1/extensions/{id}",
    ),
    (
        "delete_extension",
        ("e1",),
        {},
        "DELETE",
        "/v1/extensions/e1",
        None,
        {},
        "/v1/extensions/{id}",
    ),
    (
        "list_extension_groups",
        (),
        {},
        "GET",
        "/v1/extension-groups",
        None,
        {},
        "/v1/extension-groups",
    ),
    (
        "get_extension_group",
        ("eg1",),
        {},
        "GET",
        "/v1/extension-groups/eg1",
        None,
        {},
        "/v1/extension-groups/{id}",
    ),
    (
        "create_extension_group",
        (),
        {"name": "Adblock set"},
        "POST",
        "/v1/extension-groups",
        {"name": "Adblock set"},
        {},
        "/v1/extension-groups",
    ),
    (
        "update_extension_group",
        ("eg1",),
        {"extension_ids": ["e1", "e2"]},
        "PUT",
        "/v1/extension-groups/eg1",
        {"extension_ids": ["e1", "e2"]},
        {},
        "/v1/extension-groups/{id}",
    ),
    (
        "delete_extension_group",
        ("eg1",),
        {},
        "DELETE",
        "/v1/extension-groups/eg1",
        None,
        {},
        "/v1/extension-groups/{id}",
    ),
    (
        "add_extension_to_group",
        ("eg1", "e1"),
        {},
        "POST",
        "/v1/extension-groups/eg1/extensions/e1",
        None,
        {},
        "/v1/extension-groups/{id}/extensions/{extension_id}",
    ),
    (
        "remove_extension_from_group",
        ("eg1", "e1"),
        {},
        "DELETE",
        "/v1/extension-groups/eg1/extensions/e1",
        None,
        {},
        "/v1/extension-groups/{id}/extensions/{extension_id}",
    ),
    # -- browsers ----------------------------------------------------------
    (
        "download_browser",
        (),
        {"browser": "wayfern", "version": "152.0.1"},
        "POST",
        "/v1/browsers/download",
        {"browser": "wayfern", "version": "152.0.1"},
        {},
        "/v1/browsers/download",
    ),
    (
        "list_browser_versions",
        ("wayfern",),
        {},
        "GET",
        "/v1/browsers/wayfern/versions",
        None,
        {},
        "/v1/browsers/{browser}/versions",
    ),
    (
        "is_browser_downloaded",
        ("wayfern", "152.0.1"),
        {},
        "GET",
        "/v1/browsers/wayfern/versions/152.0.1/downloaded",
        None,
        {},
        "/v1/browsers/{browser}/versions/{version}/downloaded",
    ),
]


@pytest.mark.parametrize(
    "case", CASES, ids=[f"{case[0]}[{index}]" for index, case in enumerate(CASES)]
)
def test_method_sends_the_documented_request(
    client: DonutClient, fake: FakeDonut, case: Case
) -> None:
    name, args, kwargs, verb, path, body, query, operation = case

    getattr(client, name)(*args, **kwargs)

    sent = fake.last
    assert sent.method == verb
    assert sent.path == path
    assert sent.query == query
    assert sent.json == body
    assert OPERATIONS[(verb, operation)] == name


def test_every_client_method_is_exercised_here() -> None:
    """No method may be added to the table of operations without a case above."""
    covered = {case[0] for case in CASES}
    missing = sorted(set(OPERATIONS.values()) - covered)
    assert not missing, f"these wrapped operations have no request test: {missing}"


def test_the_token_travels_as_a_bearer_header(client: DonutClient, fake: FakeDonut) -> None:
    client.list_profiles()
    sent = fake.last
    assert sent.header("Authorization") == "Bearer test-token-abc123"
    assert sent.header("Accept") == "application/json"
    assert sent.header("Content-Type") is None, "a GET must not claim to carry JSON"


def test_a_body_is_sent_as_json(client: DonutClient, fake: FakeDonut) -> None:
    client.create_group(name="Retail")
    sent = fake.last
    assert sent.header("Content-Type") == "application/json"
    assert json.loads(sent.body.decode()) == {"name": "Retail"}


def test_path_ids_are_escaped(client: DonutClient, fake: FakeDonut) -> None:
    """An id can never break out of its own path segment."""
    client.get_profile("a/b c?d")
    assert fake.last.path == "/v1/profiles/a%2Fb%20c%3Fd"


def test_none_arguments_are_left_out_of_the_body(client: DonutClient, fake: FakeDonut) -> None:
    client.update_profile("p1", name="Only this")
    assert fake.last.json == {"name": "Only this"}


def test_an_empty_string_still_reaches_the_app(client: DonutClient, fake: FakeDonut) -> None:
    """`proxy_id=""` is how the app is told to detach a proxy, so it must survive."""
    client.update_profile("p1", proxy_id="")
    assert fake.last.json == {"proxy_id": ""}


def test_a_no_content_answer_becomes_none(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_empty(204)
    assert client.delete_profile("p1") is None


def test_a_json_answer_is_returned_as_sent(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_json({"profiles": [{"id": "p1", "name": "Shopper"}], "total": 1})
    assert client.list_profiles() == {
        "profiles": [{"id": "p1", "name": "Shopper"}],
        "total": 1,
    }


def test_a_bare_boolean_answer_is_returned(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_json(True)
    assert client.is_browser_downloaded("wayfern", "152.0.1") is True
