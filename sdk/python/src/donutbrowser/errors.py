"""Exceptions raised by the Donut Browser SDK.

The local REST API answers with a plain-text body and one of a small set of
statuses. Each status means one thing, so each gets its own exception and a
caller can branch on the class instead of on a number:

===  ==========================  ==================================
403  ``Forbidden``               Terms not accepted, or not signed in
400  ``ValidationError``         Malformed request, duplicate name
401  ``Unauthorized``            Missing or wrong bearer token
402  ``PaymentRequired``         Automation needs an active paid plan
404  ``NotFound``                No such profile, group, proxy, ...
408  ``RequestTimeout``          ``agent/pick`` waited and nothing was picked
409  ``Conflict``                Something else holds the profile right now
429  ``RateLimited``             Automation quota spent; see ``retry_after``
500  ``ServerError``             Internal failure
502  ``BadGateway``              The browser or relay answered wrongly
503  ``ServiceUnavailable``      Cloud, fleet or lock service unreachable
===  ==========================  ==================================

Some bodies are the structured ``{"code": ..., "params": {...}}`` strings the
desktop app shares with its own frontend. When one arrives, ``code`` and
``params`` are filled in; otherwise ``code`` is ``None`` and ``body`` holds the
diagnostic text as sent.
"""

from __future__ import annotations

import json
from typing import Any, Mapping, Optional

__all__ = [
    "DonutError",
    "DonutConnectionError",
    "DonutAPIError",
    "ValidationError",
    "Unauthorized",
    "PaymentRequired",
    "Forbidden",
    "NotFound",
    "RequestTimeout",
    "Conflict",
    "RateLimited",
    "ServerError",
    "BadGateway",
    "ServiceUnavailable",
    "error_for_status",
]


class DonutError(Exception):
    """Base class for everything this package raises."""


class DonutConnectionError(DonutError):
    """The app could not be reached at all.

    Usually means the local API is switched off, is listening on another port,
    or the desktop app is not running.
    """


class DonutAPIError(DonutError):
    """The app answered, and the answer was an error status."""

    #: HTTP status this class is raised for. ``None`` on the base class, which
    #: catches every status without a more specific subclass.
    status: Optional[int] = None

    def __init__(
        self,
        status: int,
        body: str,
        *,
        method: str = "",
        path: str = "",
        headers: Optional[Mapping[str, str]] = None,
    ) -> None:
        self.status = status
        self.body = body
        self.method = method
        self.path = path
        self.headers = dict(headers or {})
        self.code: Optional[str] = None
        self.params: dict[str, Any] = {}

        stripped = body.strip()
        if stripped.startswith("{"):
            try:
                decoded = json.loads(stripped)
            except ValueError:
                decoded = None
            if isinstance(decoded, dict) and isinstance(decoded.get("code"), str):
                self.code = decoded["code"]
                params = decoded.get("params")
                if isinstance(params, dict):
                    self.params = params

        where = f"{method} {path}".strip()
        detail = self.code or stripped or "(empty body)"
        super().__init__(f"{status} on {where}: {detail}" if where else f"{status}: {detail}")


class ValidationError(DonutAPIError):
    """400: the request was malformed, duplicated a name, or named something unsupported."""

    status = 400


class Unauthorized(DonutAPIError):
    """401: no bearer token, the wrong one, or the local API has no token stored."""

    status = 401


class PaymentRequired(DonutAPIError):
    """402: this action needs an active paid plan, or the proxy behind it lapsed."""

    status = 402


class Forbidden(DonutAPIError):
    """403: the Wayfern terms are not accepted, or this desktop is not signed in."""

    status = 403


class NotFound(DonutAPIError):
    """404: no entity with that id."""

    status = 404


class RequestTimeout(DonutAPIError):
    """408: ``agent/pick`` waited its whole timeout and nothing was picked."""

    status = 408


class Conflict(DonutAPIError):
    """409: something else holds the profile — a browser, a teammate, a remote session."""

    status = 409


class RateLimited(DonutAPIError):
    """429: the shared automation quota is spent.

    ``retry_after`` is the number of seconds the server asked the caller to
    wait, taken from the ``Retry-After`` response header. It is ``None`` only
    when the header is missing or unreadable.
    """

    status = 429

    def __init__(
        self,
        status: int,
        body: str,
        *,
        method: str = "",
        path: str = "",
        headers: Optional[Mapping[str, str]] = None,
    ) -> None:
        super().__init__(status, body, method=method, path=path, headers=headers)
        self.retry_after: Optional[int] = None
        raw = next(
            (value for key, value in self.headers.items() if key.lower() == "retry-after"),
            None,
        )
        if raw is not None:
            try:
                self.retry_after = int(str(raw).strip())
            except ValueError:
                self.retry_after = None


class ServerError(DonutAPIError):
    """500 and the other 5xx: the app, the fleet or an upstream failed.

    ``BadGateway`` and ``ServiceUnavailable`` derive from this, so one
    ``except ServerError`` catches every server-side failure.
    """

    status = 500


class BadGateway(ServerError):
    """502: the browser or the relay did not answer the way it documents."""

    status = 502


class ServiceUnavailable(ServerError):
    """503: Donut cloud, the remote fleet, or the profile lock service is unreachable.

    Whatever was running keeps running: a 503 from ``kill`` or from stopping a
    remote session means the browser is still up, not that it stopped.
    """

    status = 503


_BY_STATUS: dict[int, type[DonutAPIError]] = {
    cls.status: cls
    for cls in (
        ValidationError,
        Unauthorized,
        PaymentRequired,
        Forbidden,
        NotFound,
        RequestTimeout,
        Conflict,
        RateLimited,
        ServerError,
        BadGateway,
        ServiceUnavailable,
    )
    if cls.status is not None
}


def error_for_status(
    status: int,
    body: str,
    *,
    method: str = "",
    path: str = "",
    headers: Optional[Mapping[str, str]] = None,
) -> DonutAPIError:
    """Build the exception that belongs to ``status``.

    A status with no class of its own becomes a plain :class:`DonutAPIError`,
    so a future status added to the app still raises something a caller can
    catch rather than escaping as a decode failure.
    """
    cls = _BY_STATUS.get(status)
    if cls is None:
        cls = ServerError if status >= 500 else DonutAPIError
    return cls(status, body, method=method, path=path, headers=headers)
