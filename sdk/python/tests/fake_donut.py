"""A stand-in for the desktop app's local REST API.

It records what the client sent, byte for byte, and answers with whatever the
test queued. Nothing here reaches the network: it binds an ephemeral loopback
port and is torn down with the test.
"""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import parse_qsl, urlsplit


@dataclass
class RecordedRequest:
    method: str
    target: str
    headers: Dict[str, str]
    body: bytes

    @property
    def path(self) -> str:
        return urlsplit(self.target).path

    @property
    def query(self) -> Dict[str, str]:
        return dict(parse_qsl(urlsplit(self.target).query, keep_blank_values=True))

    @property
    def json(self) -> Any:
        if not self.body:
            return None
        return json.loads(self.body.decode("utf-8"))

    def header(self, name: str) -> Optional[str]:
        for key, value in self.headers.items():
            if key.lower() == name.lower():
                return value
        return None


@dataclass
class QueuedResponse:
    status: int = 200
    body: str = ""
    headers: Tuple[Tuple[str, str], ...] = ()
    content_type: str = "application/json"


@dataclass
class FakeDonut:
    """Queue responses, then read :attr:`requests` back."""

    requests: List[RecordedRequest] = field(default_factory=list)
    responses: List[QueuedResponse] = field(default_factory=list)
    _server: Optional[ThreadingHTTPServer] = None
    _thread: Optional[threading.Thread] = None

    def enqueue_json(self, payload: Any, status: int = 200) -> None:
        self.responses.append(QueuedResponse(status=status, body=json.dumps(payload)))

    def enqueue_empty(self, status: int = 204) -> None:
        self.responses.append(QueuedResponse(status=status, body=""))

    def enqueue_error(
        self,
        status: int,
        body: str = "",
        headers: Tuple[Tuple[str, str], ...] = (),
    ) -> None:
        self.responses.append(
            QueuedResponse(status=status, body=body, headers=headers, content_type="text/plain")
        )

    @property
    def port(self) -> int:
        assert self._server is not None, "the fake server is not running"
        return self._server.server_address[1]

    @property
    def last(self) -> RecordedRequest:
        assert self.requests, "the client sent nothing"
        return self.requests[-1]

    def start(self) -> "FakeDonut":
        fake = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_args: Any) -> None:
                """Keep the test output clean."""

            def _handle(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                fake.requests.append(
                    RecordedRequest(
                        method=self.command,
                        target=self.path,
                        headers={key: value for key, value in self.headers.items()},
                        body=body,
                    )
                )

                queued = fake.responses.pop(0) if fake.responses else QueuedResponse(body="{}")
                payload = queued.body.encode("utf-8")
                self.send_response(queued.status)
                for name, value in queued.headers:
                    self.send_header(name, value)
                if payload:
                    self.send_header("Content-Type", queued.content_type)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                if payload:
                    self.wfile.write(payload)

            do_GET = _handle
            do_POST = _handle
            do_PUT = _handle
            do_DELETE = _handle
            do_PATCH = _handle

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        # A short poll interval so `shutdown()` returns promptly: the default
        # 0.5s would add half a second to the teardown of every single test.
        self._thread = threading.Thread(
            target=self._server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
        )
        self._thread.start()
        return self

    def stop(self) -> None:
        if self._server is not None:
            self._server.shutdown()
            self._server.server_close()
            self._server = None
        if self._thread is not None:
            self._thread.join(timeout=5)
            self._thread = None
