"""Where the token and the port come from, and in what order."""

from __future__ import annotations

import pytest
from fake_donut import FakeDonut

from donutbrowser import DEFAULT_HOST, DEFAULT_PORT, DonutClient, DonutError


def test_arguments_are_used_as_given() -> None:
    client = DonutClient(token="from-argument", port=12345, env={})
    assert client.token == "from-argument"
    assert client.port == 12345
    assert client.host == DEFAULT_HOST
    assert client.base_url == "http://127.0.0.1:12345"


def test_the_environment_fills_in_what_was_not_passed() -> None:
    client = DonutClient(env={"DONUT_API_TOKEN": "from-env", "DONUT_API_PORT": "13579"})
    assert client.token == "from-env"
    assert client.port == 13579


def test_arguments_win_over_the_environment() -> None:
    client = DonutClient(
        token="from-argument",
        port=111,
        env={"DONUT_API_TOKEN": "from-env", "DONUT_API_PORT": "222"},
    )
    assert client.token == "from-argument"
    assert client.port == 111


def test_the_port_falls_back_to_the_app_default() -> None:
    client = DonutClient(env={"DONUT_API_TOKEN": "t"})
    assert client.port == DEFAULT_PORT == 10108


def test_a_base_url_overrides_host_and_port() -> None:
    client = DonutClient(
        base_url="http://127.0.0.1:9999/donut",
        token="t",
        env={"DONUT_API_PORT": "222"},
    )
    assert client.port == 9999
    assert client.base_url == "http://127.0.0.1:9999/donut"


def test_a_base_url_prefix_is_kept_on_every_path(fake: FakeDonut) -> None:
    with DonutClient(
        base_url=f"http://127.0.0.1:{fake.port}/donut", token="t", timeout=5.0, env={}
    ) as client:
        client.list_profiles()
    assert fake.last.path == "/donut/v1/profiles"


def test_an_unusable_port_in_the_environment_is_reported() -> None:
    with pytest.raises(DonutError) as raised:
        DonutClient(env={"DONUT_API_TOKEN": "t", "DONUT_API_PORT": "not-a-number"})
    assert "DONUT_API_PORT" in str(raised.value)


def test_an_unsupported_scheme_is_refused() -> None:
    with pytest.raises(DonutError):
        DonutClient(base_url="ftp://127.0.0.1:9999", token="t", env={})


def test_the_websocket_address_is_built_from_the_same_base() -> None:
    client = DonutClient(token="t", port=10108, env={})
    assert (
        client.remote_session_cdp_url("s 1")
        == "ws://127.0.0.1:10108/v1/remote-sessions/s%201/cdp"
    )


def test_a_reopened_client_still_works(fake: FakeDonut) -> None:
    """`close()` drops the socket; the next call has to open a new one."""
    with DonutClient(token="t", port=fake.port, timeout=5.0, env={}) as client:
        client.list_profiles()
        client.close()
        client.list_profiles()
    assert len(fake.requests) == 2
