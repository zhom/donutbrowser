# donutbrowser

A thin Python client for the [Donut Browser](https://donutbrowser.com) local
REST API. Every method wraps exactly one documented endpoint; nothing is
invented, cached or retried.

The local API is off by default. Switch it on in the app under **Settings →
Integrations → Local API → "Enable Local API Server"**, then copy the port and
the authentication token from that screen.

```bash
pip install -e .            # from this directory
```

```python
from donutbrowser import DonutClient

with DonutClient(token="...") as client:
    with client.run(profile_id, url="https://example.com", headless=True) as session:
        print(session.cdp_url)
```

The client reads `DONUT_API_TOKEN` and `DONUT_API_PORT` when the token and port
are not passed as arguments.

Full documentation, including the Node package and a worked agent example, is in
[`sdk/README.md`](../README.md).

## Tests

```bash
pip install -e ".[dev]"
pytest
```

The suite runs entirely against a fake HTTP server on loopback. It never reaches
the network and never needs the desktop app.
