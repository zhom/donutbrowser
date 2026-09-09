"""Each status the app documents raises its own exception."""

from __future__ import annotations

import json

import pytest
from fake_donut import FakeDonut, QueuedResponse

from donutbrowser import (
    BadGateway,
    Conflict,
    DonutAPIError,
    DonutClient,
    DonutConnectionError,
    DonutError,
    Forbidden,
    NotFound,
    PaymentRequired,
    RateLimited,
    RequestTimeout,
    ServerError,
    ServiceUnavailable,
    Unauthorized,
    ValidationError,
)

STATUS_TO_ERROR = [
    (400, ValidationError),
    (401, Unauthorized),
    (402, PaymentRequired),
    (403, Forbidden),
    (404, NotFound),
    (408, RequestTimeout),
    (409, Conflict),
    (429, RateLimited),
    (500, ServerError),
    (502, BadGateway),
    (503, ServiceUnavailable),
]


@pytest.mark.parametrize("status,expected", STATUS_TO_ERROR)
def test_status_maps_to_its_exception(
    client: DonutClient, fake: FakeDonut, status: int, expected: type
) -> None:
    fake.enqueue_error(status, "something went wrong")
    with pytest.raises(expected) as raised:
        client.list_profiles()
    assert raised.value.status == status
    assert raised.value.body == "something went wrong"
    assert raised.value.method == "GET"
    assert raised.value.path == "/v1/profiles"


def test_every_error_is_a_donut_error(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_error(404, "PROFILE_NOT_FOUND")
    with pytest.raises(DonutError):
        client.get_profile("nope")


def test_the_five_hundreds_share_one_base(client: DonutClient, fake: FakeDonut) -> None:
    """`except ServerError` has to catch 502 and 503 as well as 500."""
    for status in (500, 502, 503):
        fake.enqueue_error(status, "upstream")
        with pytest.raises(ServerError):
            client.list_profiles()


def test_rate_limited_carries_retry_after(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_error(
        429,
        "automation request rate limit exceeded",
        headers=(("Retry-After", "42"),),
    )
    with pytest.raises(RateLimited) as raised:
        client.run_profile("p1")
    assert raised.value.retry_after == 42


def test_rate_limited_without_the_header_is_still_raised(
    client: DonutClient, fake: FakeDonut
) -> None:
    fake.enqueue_error(429, "slow down")
    with pytest.raises(RateLimited) as raised:
        client.run_profile("p1")
    assert raised.value.retry_after is None


def test_an_unreadable_retry_after_does_not_break_the_error(
    client: DonutClient, fake: FakeDonut
) -> None:
    fake.enqueue_error(429, "slow down", headers=(("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT"),))
    with pytest.raises(RateLimited) as raised:
        client.run_profile("p1")
    assert raised.value.retry_after is None


def test_a_structured_code_body_is_decoded(client: DonutClient, fake: FakeDonut) -> None:
    """The app shares `{"code": ...}` strings with its own frontend."""
    fake.enqueue_error(400, json.dumps({"code": "NAME_CANNOT_BE_EMPTY"}))
    with pytest.raises(ValidationError) as raised:
        client.create_group(name="")
    assert raised.value.code == "NAME_CANNOT_BE_EMPTY"
    assert raised.value.params == {}


def test_a_structured_code_body_keeps_its_params(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_error(409, json.dumps({"code": "PROFILE_LOCKED_BY_MEMBER", "params": {"n": "5"}}))
    with pytest.raises(Conflict) as raised:
        client.run_profile("p1")
    assert raised.value.code == "PROFILE_LOCKED_BY_MEMBER"
    assert raised.value.params == {"n": "5"}


def test_a_plain_text_body_leaves_code_unset(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_error(400, "invalid browser")
    with pytest.raises(ValidationError) as raised:
        client.create_profile(name="x", browser="chromium")
    assert raised.value.code is None
    assert raised.value.body == "invalid browser"


def test_an_undocumented_status_still_raises_something_catchable(
    client: DonutClient, fake: FakeDonut
) -> None:
    fake.enqueue_error(418, "teapot")
    with pytest.raises(DonutAPIError) as raised:
        client.list_profiles()
    assert raised.value.status == 418


def test_an_undocumented_server_status_is_a_server_error(
    client: DonutClient, fake: FakeDonut
) -> None:
    fake.enqueue_error(504, "gateway timeout")
    with pytest.raises(ServerError):
        client.list_profiles()


def test_the_message_names_the_call(client: DonutClient, fake: FakeDonut) -> None:
    fake.enqueue_error(404, "Profile not found")
    with pytest.raises(NotFound) as raised:
        client.get_profile("missing")
    assert "404" in str(raised.value)
    assert "GET /v1/profiles/missing" in str(raised.value)


def test_an_unreachable_app_is_not_an_api_error(fake: FakeDonut) -> None:
    port = fake.port
    fake.stop()
    with DonutClient(token="t", port=port, timeout=2.0, env={}) as client:
        with pytest.raises(DonutConnectionError) as raised:
            client.list_profiles()
    assert "Local API" in str(raised.value)


def test_a_missing_token_fails_before_any_request() -> None:
    with pytest.raises(DonutError) as raised:
        DonutClient(env={})
    assert "DONUT_API_TOKEN" in str(raised.value)


def test_a_non_json_answer_is_reported_as_such(client: DonutClient, fake: FakeDonut) -> None:
    fake.responses.append(QueuedResponse(status=200, body="<html>nope</html>"))
    with pytest.raises(DonutError) as raised:
        client.list_profiles()
    assert "not JSON" in str(raised.value)
