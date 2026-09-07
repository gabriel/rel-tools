from __future__ import annotations

import json
import threading
import unittest
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from rel_crawler import RelClient, RelRpcError


class _Server(ThreadingHTTPServer):
    responses: deque[tuple[int, dict[str, Any]]]
    requests: list[tuple[str, str, dict[str, Any] | None]]


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self._handle()

    def do_POST(self) -> None:
        self._handle()

    def _handle(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b""
        payload = json.loads(raw) if raw else None
        self.server.requests.append(  # type: ignore[attr-defined]
            (self.command, self.path, payload)
        )
        status, response = self.server.responses.popleft()  # type: ignore[attr-defined]
        body = json.dumps(response).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return None


class RelClientTests(unittest.TestCase):
    def setUp(self) -> None:
        self.server = _Server(("127.0.0.1", 0), _Handler)
        self.server.responses = deque()
        self.server.requests = []
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        host, port = self.server.server_address
        self.client = RelClient(f"http://{host}:{port}/v1")

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def test_navigate_uses_rpc_v1_and_parses_page_operation(self) -> None:
        self.server.responses.append(
            (
                200,
                {
                    "status": "ok",
                    "request_id": "req_1",
                    "data": {
                        "page": {
                            "id": "page_1",
                            "session_id": "Session2",
                            "url": "https://example.com/",
                        },
                        "capture": {
                            "output_path": "/tmp/page.html",
                            "bytesize": 42,
                            "target_http_status": 200,
                        },
                    },
                },
            )
        )

        result = self.client.navigate(
            url="https://example.com",
            session_id="Session2",
            output=Path("/tmp/page.html"),
            timeout=10,
            wait=0,
        )

        self.assertEqual(result.page_id, "page_1")
        self.assertEqual(result.bytesize, 42)
        method, path, payload = self.server.requests[0]
        self.assertEqual((method, path), ("POST", "/v1/navigate"))
        self.assertEqual(payload["session_id"], "Session2")  # type: ignore[index]

    def test_structured_http_error_is_preserved(self) -> None:
        self.server.responses.append(
            (
                503,
                {
                    "status": "error",
                    "request_id": "req_busy",
                    "error": {
                        "id": "BROWSER_BUSY",
                        "code": 10201,
                        "message": "busy",
                        "retryable": True,
                        "details": {"session_id": "Session2"},
                    },
                },
            )
        )

        with self.assertRaises(RelRpcError) as raised:
            self.client.health()

        self.assertEqual(raised.exception.id, "BROWSER_BUSY")
        self.assertEqual(raised.exception.code, 10201)
        self.assertTrue(raised.exception.retryable)
        self.assertEqual(raised.exception.request_id, "req_busy")

    def test_observe_links_keeps_only_enabled_links_with_usable_bounds(self) -> None:
        self.server.responses.append(
            (
                200,
                {
                    "status": "ok",
                    "request_id": "req_observe",
                    "data": {
                        "page": {
                            "id": "page_1",
                            "session_id": "Session2",
                            "url": "https://example.com/new/",
                        },
                        "observation": {
                            "id": "11111111-1111-4111-8111-111111111111",
                            "truncated": False,
                            "omitted_node_count": 0,
                            "visited_node_count": 40,
                            "elements": [
                                {
                                    "ref": "e1",
                                    "role": "link",
                                    "name": "Visible offscreen link",
                                    "states": ["enabled"],
                                    "destination": "https://example.com/release/one",
                                    "in_viewport": False,
                                    "bounds": {
                                        "x": 10,
                                        "y": 900,
                                        "width": 120,
                                        "height": 24,
                                    },
                                },
                                {
                                    "ref": "e2",
                                    "role": "link",
                                    "name": "No layout box",
                                    "states": ["enabled"],
                                    "destination": "https://example.com/release/hidden",
                                    "in_viewport": False,
                                    "bounds": {
                                        "x": 0,
                                        "y": 0,
                                        "width": 0,
                                        "height": 0,
                                    },
                                },
                                {
                                    "ref": "e3",
                                    "role": "link",
                                    "name": "Disabled link",
                                    "states": ["disabled"],
                                    "destination": (
                                        "https://example.com/release/disabled"
                                    ),
                                    "in_viewport": True,
                                    "bounds": {
                                        "x": 10,
                                        "y": 10,
                                        "width": 120,
                                        "height": 24,
                                    },
                                },
                                {
                                    "ref": "e4",
                                    "role": "button",
                                    "name": "Not a link",
                                    "states": ["enabled"],
                                    "in_viewport": True,
                                    "bounds": {
                                        "x": 10,
                                        "y": 40,
                                        "width": 120,
                                        "height": 24,
                                    },
                                },
                            ],
                        },
                    },
                },
            )
        )

        result = self.client.observe_links(
            session_id="Session2", timeout=10, wait=0
        )

        self.assertEqual(len(result.links), 1)
        self.assertEqual(result.links[0].url, "https://example.com/release/one")
        self.assertFalse(result.links[0].in_viewport)
        self.assertEqual(result.element_count, 4)
        method, path, payload = self.server.requests[0]
        self.assertEqual((method, path), ("POST", "/v1/observe"))
        self.assertEqual(
            payload,
            {
                "session_id": "Session2",
                "mode": "semantic",
                "timeout": 10,
                "wait": 0,
            },
        )

    def test_create_session_sends_profile_and_group(self) -> None:
        self.server.responses.append(
            (
                200,
                {
                    "status": "ok",
                    "request_id": "req_session",
                    "data": {
                        "session": {
                            "id": "Session9",
                            "profile": "oxylabs",
                        }
                    },
                },
            )
        )

        session_id = self.client.create_session(profile="oxylabs", group="crawler")

        self.assertEqual(session_id, "Session9")
        method, path, payload = self.server.requests[0]
        self.assertEqual((method, path), ("POST", "/v1/sessions"))
        self.assertEqual(payload, {"profile": "oxylabs", "group": "crawler"})

    def test_create_session_omits_unspecified_profile(self) -> None:
        self.server.responses.append(
            (200, {
                "status": "ok",
                "request_id": "req_session",
                "data": {"session": {"id": "Session9", "profile": "Research"}},
            })
        )

        session_id = self.client.create_session(profile=None, group="crawler")

        self.assertEqual(session_id, "Session9")
        method, path, payload = self.server.requests[0]
        self.assertEqual((method, path), ("POST", "/v1/sessions"))
        self.assertEqual(payload, {"group": "crawler"})

    def test_rejects_non_loopback_base_url(self) -> None:
        with self.assertRaises(ValueError):
            RelClient("https://api.example.com/v1")


if __name__ == "__main__":
    unittest.main()
