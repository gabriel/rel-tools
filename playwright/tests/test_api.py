from __future__ import annotations

import json
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from rel_playwright.async_api import async_playwright
from rel_playwright.sync_api import Error, UnsupportedError, sync_playwright


class _Server(ThreadingHTTPServer):
    requests: list[tuple[str, str, dict[str, Any] | None]]
    html_path: Path
    screenshot_path: Path
    session_sequence: int


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self._handle()

    def do_POST(self) -> None:
        self._handle()

    def do_DELETE(self) -> None:
        self._handle()

    def _handle(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b""
        payload = json.loads(raw) if raw else None
        server: _Server = self.server  # type: ignore[assignment]
        server.requests.append((self.command, self.path, payload))

        status = 200
        if self.command == "GET" and self.path == "/v1/health":
            data: dict[str, Any] = {"version": "test", "overall_status": "ok"}
        elif self.command == "GET" and self.path.startswith("/v1/sessions/"):
            session_id = self.path.rsplit("/", 1)[-1]
            data = {"session": {"id": session_id, "profile": "Research"}}
        elif self.command == "POST" and self.path == "/v1/sessions":
            server.session_sequence += 1
            data = {
                "session": {
                    "id": f"Session{server.session_sequence}",
                    "profile": payload.get("profile", "Configured default"),
                    "group": payload["group"],
                }
            }
        elif self.command == "DELETE" and self.path.startswith("/v1/sessions/"):
            data = {"deleted_id": self.path.rsplit("/", 1)[-1]}
        elif self.command == "POST" and self.path == "/v1/navigate/observe":
            data = {
                "page": {
                    "id": "page_test",
                    "session_id": payload["session_id"],
                    "url": "https://example.com/back",
                },
                "observation": {
                    "id": "11111111-1111-4111-8111-111111111111",
                    "elements": [],
                },
            }
        elif self.command == "POST" and self.path == "/v1/screenshot":
            output = Path(payload.get("output", server.screenshot_path))
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(server.screenshot_path.read_bytes())
            data = {
                "page": {
                    "id": "page_test",
                    "session_id": payload["session_id"],
                    "url": "https://example.com/",
                },
                "screenshot": {
                    "output_path": str(output),
                    "bytesize": output.stat().st_size,
                    "format": payload["format"],
                    "mime_type": f"image/{payload['format']}",
                    "width": 100,
                    "height": 200,
                },
            }
        elif self.command == "POST" and self.path in {
            "/v1/navigate",
            "/v1/capture",
            "/v1/perform",
        }:
            if self.path == "/v1/navigate" and payload["url"].endswith("/missing"):
                status = 502
                body = {
                    "status": "error",
                    "request_id": "req_missing",
                    "error": {
                        "id": "UPSTREAM_UNAVAILABLE",
                        "code": 10302,
                        "message": "The target page returned HTTP 404.",
                        "retryable": True,
                        "details": {
                            "target_http_status": 404,
                            "url": "https://example.com/missing",
                        },
                    },
                }
                self._write(status, body)
                return
            url = (
                payload.get("url", "https://example.com/")
                if isinstance(payload, dict)
                else "https://example.com/"
            )
            data = {
                "page": {
                    "id": "page_test",
                    "session_id": payload["session_id"],
                    "url": url,
                },
                "capture": {
                    "output_path": str(server.html_path),
                    "bytesize": server.html_path.stat().st_size,
                    "target_http_status": 200,
                },
            }
        else:
            status = 404
            body = {
                "status": "error",
                "request_id": "req_unknown",
                "error": {
                    "id": "NOT_FOUND",
                    "code": 10001,
                    "message": self.path,
                    "retryable": False,
                },
            }
            self._write(status, body)
            return

        self._write(
            status,
            {"status": "ok", "request_id": f"req_{len(server.requests)}", "data": data},
        )

    def _write(self, status: int, body: dict[str, Any]) -> None:
        encoded = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, _format: str, *_args: object) -> None:
        return None


class _RelServerMixin:
    server: _Server
    thread: threading.Thread
    temporary: tempfile.TemporaryDirectory[str]
    base_url: str

    def start_server(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        html_path = root / "page.html"
        html_path.write_text(
            """<!doctype html><html><head><title>Example Domain</title></head>
            <body><main>
              <article><h2>First story</h2><a href=\"/one\">One</a></article>
              <article><h2>Second story</h2><a href=\"/two\">Two</a></article>
              <form><input name=\"q\" value=\"before\"><button id=\"submit\">Go</button></form>
            </main></body></html>""",
            encoding="utf-8",
        )
        screenshot_path = root / "source.png"
        screenshot_path.write_bytes(b"fake-png")
        self.server = _Server(("127.0.0.1", 0), _Handler)
        self.server.requests = []
        self.server.html_path = html_path
        self.server.screenshot_path = screenshot_path
        self.server.session_sequence = 8
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        host, port = self.server.server_address
        self.base_url = f"http://{host}:{port}/v1"

    def stop_server(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.temporary.cleanup()


class SyncApiTests(_RelServerMixin, unittest.TestCase):
    def setUp(self) -> None:
        self.start_server()

    def tearDown(self) -> None:
        self.stop_server()

    def test_launch_omits_profile_to_use_the_configured_default(self) -> None:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(rel_base_url=self.base_url)
            browser.new_page()
            browser.close()

        create = next(
            request
            for request in self.server.requests
            if request[:2] == ("POST", "/v1/sessions")
        )
        self.assertNotIn("profile", create[2])  # type: ignore[index]

    def test_playwright_shaped_scraping_uses_rel_profile_and_native_actions(
        self,
    ) -> None:
        screenshot_path = Path(self.temporary.name) / "capture.webp"
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                profile="Research", rel_base_url=self.base_url
            )
            page = browser.new_page()
            response = page.goto("https://example.com/")

            self.assertEqual(response.status, 200)
            self.assertTrue(response.ok)
            self.assertEqual(page.title(), "Example Domain")
            self.assertEqual(
                page.locator("article h2").all_inner_texts(),
                ["First story", "Second story"],
            )
            self.assertEqual(page.locator("article").first.inner_html().count("<a"), 1)
            self.assertEqual(
                page.locator("article a").nth(1).get_attribute("href"), "/two"
            )
            self.assertEqual(
                page.locator("input[name=q]").get_attribute("value"), "before"
            )
            with self.assertRaises(UnsupportedError):
                page.locator("input[name=q]").input_value()
            page.locator("input[name=q]").fill("after")
            page.locator("#submit").click()
            self.assertEqual(
                page.screenshot(path=screenshot_path, full_page=True), b"fake-png"
            )
            browser.close()

        create = next(
            request
            for request in self.server.requests
            if request[:2] == ("POST", "/v1/sessions")
        )
        self.assertEqual(create[2]["profile"], "Research")  # type: ignore[index]
        action_payloads = [
            payload
            for method, path, payload in self.server.requests
            if method == "POST" and path == "/v1/perform"
        ]
        self.assertEqual(
            action_payloads[0]["actions"],  # type: ignore[index]
            [
                {"action": "clear", "selector": "input[name=q]"},
                {"action": "type", "selector": "input[name=q]", "text": "after"},
            ],
        )
        self.assertTrue(
            any(method == "DELETE" for method, _path, _payload in self.server.requests)
        )

    def test_existing_session_is_reused_and_never_deleted(self) -> None:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                session_id="Session42", rel_base_url=self.base_url
            )
            page = browser.new_page()
            page.goto("https://example.com/")
            browser.close()

        self.assertFalse(
            any(method == "DELETE" for method, _path, _payload in self.server.requests)
        )
        self.assertFalse(
            any(
                path == "/v1/sessions"
                for _method, path, _payload in self.server.requests
            )
        )

    def test_http_error_becomes_playwright_response(self) -> None:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(rel_base_url=self.base_url)
            page = browser.new_page()
            response = page.goto("https://example.com/missing")
            self.assertEqual(response.status, 404)
            self.assertFalse(response.ok)
            self.assertEqual(response.url, "https://example.com/missing")

    def test_unsupported_operations_and_strict_locators_fail_explicitly(self) -> None:
        with sync_playwright() as playwright:
            with self.assertRaises(UnsupportedError):
                playwright.firefox.launch(rel_base_url=self.base_url)
            with self.assertRaises(UnsupportedError):
                playwright.chromium.launch(headless=True, rel_base_url=self.base_url)

            browser = playwright.chromium.launch(rel_base_url=self.base_url)
            page = browser.new_page()
            page.goto("https://example.com/")
            with self.assertRaises(Error):
                page.locator("article a").click()
            with self.assertRaises(UnsupportedError):
                page.evaluate("document.title")


class AsyncApiTests(_RelServerMixin, unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self.start_server()

    def tearDown(self) -> None:
        self.stop_server()

    async def test_async_surface_matches_common_playwright_flow(self) -> None:
        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(
                profile="Research", rel_base_url=self.base_url
            )
            page = await browser.new_page()
            response = await page.goto("https://example.com/")
            self.assertEqual(response.status, 200)
            self.assertEqual(
                await page.locator("article h2").all_inner_texts(),
                ["First story", "Second story"],
            )
            await browser.close()


if __name__ == "__main__":
    unittest.main()
