# Donut Browser SDKs

Two thin clients for the REST API that Donut Browser serves on this machine:
[`python/`](python) (`donutbrowser`) and [`node/`](node) (`@donutbrowser/sdk`).

They are deliberately thin. Every method is one request to one path that the
app publishes in its own `/openapi.json`, with the request and response shapes
taken from the Rust handlers in `src-tauri/src/api_server.rs`. Nothing is
cached, nothing is retried, and no endpoint is invented. What the two add on top
of a bare HTTP call is the part that is tedious to redo in every script:

- the bearer token and the port, read from arguments or the environment,
- one exception class per documented status, with `Retry-After` parsed and the
  app's `{"code": ...}` error bodies decoded,
- a launch-and-stop helper, so a script cannot leave a browser running,
- a drift check that fails the tests when the app grows an endpoint the SDK
  does not cover.

Neither package is part of the pnpm workspace. They build, test and publish on
their own, so they never slow the desktop app's own checks down.

## Switch the API on first

**The local REST API is off by default. It must be enabled in the app under
Settings → Integrations → Local API → "Enable Local API Server".**

That screen also shows the two things a client needs:

- the **port**, `10108` unless it was already taken or you changed it, and
- the **authentication token**, sent as `Authorization: Bearer <token>`.

The server binds `127.0.0.1` only, so it is never reachable from another
machine. Requests are also refused with `403` until the Wayfern terms have been
accepted in the app.

Both SDKs read arguments first, then the environment:

| Setting | Argument | Environment | Default |
| --- | --- | --- | --- |
| Token | `token` | `DONUT_API_TOKEN` | none; required |
| Port | `port` | `DONUT_API_PORT` | `10108` |
| Host | `host` | — | `127.0.0.1` |

`base_url` / `baseUrl` overrides host and port entirely, for the rare case of a
tunnel or a path prefix in front of the app.

## Python

Requires Python 3.10 or newer. **No runtime dependencies:** the client talks to
a loopback server on the same machine, so `http.client` from the standard
library is enough. That keeps `pip install donutbrowser` from dragging anything
into an automation environment, and it sidesteps a real trap — `urllib.request`
honours `http_proxy` from the environment, which would send calls meant for the
local app through whatever proxy the shell happens to have set.

```bash
cd sdk/python
pip install -e .
```

A worked example: launch a profile, drive the page through the agent endpoints,
and stop the browser.

```python
from donutbrowser import Conflict, DonutClient, NotFound, RateLimited

PROFILE_ID = "your-profile-id"

with DonutClient(token="...") as client:
    # `run` starts the browser on entry and stops it on exit, even if the body
    # raises. `session.cdp_url` is the DevTools endpoint the launch returned.
    with client.run(PROFILE_ID, url="https://example.com", headless=True) as session:
        print("CDP:", session.cdp_url)

        # Read the page the way the agent sees it: roles, names, text, bounds.
        page = client.agent_perceive(PROFILE_ID, viewport_only=True)
        print(page["stats"]["returnedNodes"], "nodes,", len(page["text"]), "characters")

        # Name an element without a selector, and check it is unambiguous.
        search = {"role": "textbox", "nameContains": "Search"}
        resolved = client.agent_resolve_locator(PROFILE_ID, locator=search)
        assert resolved["matchCount"] == 1

        client.agent_type(PROFILE_ID, locator=search, text="donut browser")
        client.agent_click(PROFILE_ID, locator={"role": "button", "name": "Search"})

        # Pull a table out of whatever came back.
        rows = client.agent_extract(
            PROFILE_ID,
            container={"role": "listitem"},
            field_map=[
                {"key": "title", "locator": {"role": "heading"}, "source": "text"},
                {"key": "link", "locator": {"role": "link"}, "source": "link"},
            ],
            max_pages=3,
        )
        for row in rows["rows"]:
            print(row["values"])
    # The browser is stopped here.
```

Errors are classes, not status codes:

```python
try:
    client.run_profile(PROFILE_ID)
except Conflict as busy:
    print("someone else has it:", busy.code)      # PROFILE_LOCKED_BY_MEMBER, ...
except RateLimited as limited:
    print("wait", limited.retry_after, "seconds")
except NotFound:
    print("no such profile")
```

### Tests

```bash
cd sdk/python
pip install -e ".[dev]"
pytest
```

## Node

Requires Node 22 or newer, for the built-in `fetch`. **No runtime
dependencies**; `typescript` is a development dependency and is needed only to
build `dist/` for publishing. The tests run straight from the TypeScript
sources through Node's own type stripping, so `npm test` works with nothing
installed at all.

```bash
cd sdk/node
npm install   # only needed for `npm run build`
npm run build
```

The convenience helper is `withProfile(profileId, options, work)`, a callback
rather than `await using`. `await using` is not yet syntax any released V8
understands, so TypeScript has to down-level it — which would stop the sources
running under Node's type stripping, and with it `npm test` on a clean
checkout. The callback form works on every Node 22. A `RunSession` does also
implement `Symbol.asyncDispose`, so `await using` is there for anyone whose
toolchain already handles it.

```ts
import { Conflict, DonutClient, NotFound, RateLimited } from "@donutbrowser/sdk";

const PROFILE_ID = "your-profile-id";
const client = new DonutClient({ token: "..." });

// The browser starts before `work` runs and is stopped after it, even when it
// throws. `session.cdpUrl` is the DevTools endpoint the launch returned.
const titles = await client.withProfile(
  PROFILE_ID,
  { url: "https://example.com", headless: true },
  async (session) => {
    console.log("CDP:", session.cdpUrl);

    const page = await client.agentPerceive(PROFILE_ID, { viewport_only: true });
    console.log(page.stats.returnedNodes, "nodes,", page.text.length, "characters");

    const search = { role: "textbox", nameContains: "Search" };
    const resolved = await client.agentResolveLocator(PROFILE_ID, { locator: search });
    if (resolved.matchCount !== 1) {
      throw new Error("the search box is ambiguous");
    }

    await client.agentType(PROFILE_ID, { locator: search, text: "donut browser" });
    await client.agentClick(PROFILE_ID, {
      locator: { role: "button", name: "Search" },
    });

    const extraction = await client.agentExtract(PROFILE_ID, {
      container: { role: "listitem" },
      field_map: [
        { key: "title", locator: { role: "heading" }, source: "text" },
        { key: "link", locator: { role: "link" }, source: "link" },
      ],
      max_pages: 3,
    });
    return extraction.rows.map((row) => row.values.title);
  },
);
// The browser is stopped here.

try {
  await client.runProfile(PROFILE_ID);
} catch (error) {
  if (error instanceof Conflict) {
    console.log("someone else has it:", error.code);
  } else if (error instanceof RateLimited) {
    console.log("wait", error.retryAfter, "seconds");
  } else if (error instanceof NotFound) {
    console.log("no such profile");
  } else {
    throw error;
  }
}
```

### Tests

```bash
cd sdk/node
npm test
```

`npm test` runs the TypeScript sources directly, which needs Node 22.18 or
newer (type stripping is unflagged from that release). The published package
ships compiled `.mjs`, so consumers only need Node 22.

## Errors

Both packages map the app's documented statuses onto the same set of classes.
The 5xx classes share one base, so a single `ServerError` branch catches every
server-side failure.

| Status | Python | Node | Meaning |
| ---: | --- | --- | --- |
| 400 | `ValidationError` | `ValidationError` | Malformed request, duplicate name, unsupported input |
| 401 | `Unauthorized` | `Unauthorized` | Missing or wrong bearer token |
| 402 | `PaymentRequired` | `PaymentRequired` | Automation needs an active paid plan |
| 403 | `Forbidden` | `Forbidden` | Wayfern terms not accepted, or not signed in |
| 404 | `NotFound` | `NotFound` | No entity with that id |
| 408 | `RequestTimeout` | `RequestTimeout` | `agent/pick` waited and nothing was picked |
| 409 | `Conflict` | `Conflict` | A browser, a teammate or a remote session holds the profile |
| 429 | `RateLimited` | `RateLimited` | Automation quota spent; `retry_after` / `retryAfter` |
| 500 | `ServerError` | `ServerError` | Internal failure |
| 502 | `BadGateway` | `BadGateway` | The browser or the relay answered wrongly |
| 503 | `ServiceUnavailable` | `ServiceUnavailable` | Cloud, fleet or lock service unreachable |

Anything else becomes `DonutAPIError` / `DonutApiError` (a `ServerError` for an
unrecognised 5xx), so a status added to the app later still arrives as
something a caller can catch. A transport failure — the app not running, the
API switched off, the wrong port — is `DonutConnectionError`, never an API
error, so "Donut is not there" is never confused with "Donut said no".

Every error carries `status`, `body`, `method` and `path`. When the body is one
of the app's structured `{"code": ..., "params": {...}}` strings, `code` and
`params` are filled in too.

A `503` from stopping something means the fleet could not be reached and the
remote browser is **still running**, not that it stopped.

## Staying in step with the app

`api-paths.json` in this directory lists every operation the app publishes. It
is generated from the `#[utoipa::path]` annotations and the `ApiDoc` `paths(...)`
list in `src-tauri/src/api_server.rs` — the two things the served
`/openapi.json` is actually built from — and the generator fails if a handler is
annotated but missing from `ApiDoc`, which is exactly how an endpoint silently
disappears from the spec.

```bash
python3 sdk/tools/extract-api-paths.py
```

Each SDK keeps its own table of operation to method (`donutbrowser.coverage` and
`OPERATIONS` in the Node package), and both test suites hold that table against
the snapshot in **both** directions:

- an operation in the snapshot that the SDK neither wraps nor lists as omitted
  fails the suite, so a new endpoint cannot slip past unnoticed;
- an entry the app no longer publishes fails too, so a removed endpoint cannot
  linger as a dead method;
- every wrapped operation must name a method that really exists, no two
  operations may claim the same method, and every omission must carry a reason.

On top of that, one parameterised test per method drives it against a fake
server and asserts the exact verb, path, query string and JSON body it sends.
That is what ties the table to reality rather than to a comment.

Of the 71 published operations, 70 are wrapped. The one omission:

- `GET /v1/remote-sessions/{id}/cdp` is a WebSocket upgrade, not a request an
  HTTP client can make, and bundling a websocket implementation would end the
  zero-dependency promise for one endpoint. `remote_session_cdp_url()` /
  `remoteSessionCdpUrl()` builds the `ws://` address instead, so a websocket
  library of your choosing can connect — send the same `Authorization: Bearer`
  header on the handshake.

## Tests

Both suites run offline against a fake HTTP server on an ephemeral loopback
port. Neither needs the desktop app, a browser, a network, or credentials.
