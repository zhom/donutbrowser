"""Donut Browser SDK: a thin client for the app's local REST API.

The local API is off by default. Switch it on in the app under **Settings,
Integrations, Local API, "Enable Local API Server"**, and copy the port and the
authentication token from that screen.

::

    from donutbrowser import DonutClient

    with DonutClient(token="...") as client:
        with client.run(profile_id, url="https://example.com") as session:
            client.agent_click(profile_id, locator={"role": "button", "name": "Sign in"})
"""

from .client import DEFAULT_HOST, DEFAULT_PORT, DonutClient, RunSession
from .coverage import OMITTED, OPERATIONS
from .errors import (
    BadGateway,
    Conflict,
    DonutAPIError,
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

__version__ = "0.1.0"

__all__ = [
    "DonutClient",
    "RunSession",
    "DEFAULT_HOST",
    "DEFAULT_PORT",
    "OPERATIONS",
    "OMITTED",
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
    "__version__",
]
