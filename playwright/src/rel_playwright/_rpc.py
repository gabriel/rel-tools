"""Small synchronous client for the REL RPC v1 operations used by the adapter."""

from __future__ import annotations

import json
import os
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlsplit
from urllib.request import Request, urlopen


class RpcTransportError(Exception):
    """The loopback REL agent could not be reached or decoded."""


class RpcProtocolError(Exception):
    """The REL response did not match the documented RPC v1 envelope."""


class RpcError(Exception):
    """A structured failure returned by REL RPC v1."""

    def __init__(
        self,
        *,
        error_id: str,
        code: int,
        message: str,
        retryable: bool,
        details: dict[str, Any] | None,
        request_id: str | None,
        http_status: int | None,
    ) -> None:
        super().__init__(f"{error_id}: {message}")
        self.id = error_id
        self.code = code
        self.message = message
        self.retryable = retryable
        self.details = details
        self.request_id = request_id
        self.http_status = http_status


@dataclass(frozen=True, slots=True)
class PageResult:
    page_id: str
    session_id: str
    url: str
    output_path: Path
    bytesize: int
    status: int | None


@dataclass(frozen=True, slots=True)
class ScreenshotResult:
    page_id: str
    session_id: str
    url: str
    output_path: Path
    bytesize: int
    format: str
    mime_type: str
    width: int
    height: int


class RelRpcClient:
    """Dependency-free REL client kept private to the compatibility package."""

    def __init__(self, base_url: str | None = None) -> None:
        if base_url is None:
            port = os.environ.get("REL_AGENT_PORT", "17319")
            base_url = f"http://127.0.0.1:{port}/v1"
        self.base_url = _validate_base_url(base_url)
        self._temporary = tempfile.TemporaryDirectory(prefix="rel-playwright-")
        self._temporary_lock = threading.Lock()
        self._temporary_sequence = 0

    def close(self) -> None:
        self._temporary.cleanup()

    def health(self, *, timeout: float = 5.0) -> dict[str, Any]:
        return self._request("GET", "/health", None, timeout=timeout)

    def get_session(self, session_id: str) -> dict[str, Any]:
        data = self._request(
            "GET", f"/sessions/{quote(session_id, safe='')}", None, timeout=5.0
        )
        session = data.get("session")
        if not isinstance(session, dict):
            raise RpcProtocolError("REL session response is missing data.session")
        return session

    def create_session(self, *, profile: str | None, group: str) -> str:
        data = self._request(
            "POST",
            "/sessions",
            {"group": group, **({"profile": profile} if profile is not None else {})},
            timeout=10.0,
        )
        session = data.get("session")
        session_id = session.get("id") if isinstance(session, dict) else None
        if not isinstance(session_id, str) or not session_id:
            raise RpcProtocolError("REL session response is missing data.session.id")
        return session_id

    def delete_session(self, session_id: str) -> None:
        self._request(
            "DELETE", f"/sessions/{quote(session_id, safe='')}", None, timeout=10.0
        )

    def navigate(
        self, *, url: str, session_id: str, timeout: float, wait: float
    ) -> PageResult:
        output = self._temporary_output("html")
        return _page_result(
            self._request(
                "POST",
                "/navigate",
                {
                    "url": url,
                    "session_id": session_id,
                    "output": str(output),
                    "timeout": timeout,
                    "wait": wait,
                },
                timeout=_browser_timeout(timeout, wait),
            )
        )

    def capture(self, *, session_id: str, timeout: float, wait: float) -> PageResult:
        output = self._temporary_output("html")
        return _page_result(
            self._request(
                "POST",
                "/capture",
                {
                    "session_id": session_id,
                    "output": str(output),
                    "timeout": timeout,
                    "wait": wait,
                },
                timeout=_browser_timeout(timeout, wait),
            )
        )

    def perform(
        self,
        *,
        actions: list[dict[str, Any]],
        session_id: str,
        timeout: float,
        wait: float,
    ) -> PageResult:
        output = self._temporary_output("html")
        return _page_result(
            self._request(
                "POST",
                "/perform",
                {
                    "actions": actions,
                    "session_id": session_id,
                    "output": str(output),
                    "timeout": timeout,
                    "wait": wait,
                },
                timeout=_browser_timeout(timeout, wait),
            )
        )

    def history(
        self,
        *,
        navigation: str,
        session_id: str,
        timeout: float,
        wait: float,
    ) -> PageResult:
        self._request(
            "POST",
            "/navigate/observe",
            {
                "navigation": navigation,
                "session_id": session_id,
                "mode": "semantic",
                "timeout": timeout,
                "wait": wait,
            },
            timeout=_browser_timeout(timeout, wait),
        )
        return self.capture(session_id=session_id, timeout=timeout, wait=0)

    def screenshot(
        self,
        *,
        session_id: str,
        output: Path | None,
        format: str,
        quality: int | None,
        full_page: bool,
        timeout: float,
    ) -> ScreenshotResult:
        output = output or self._temporary_output(format)
        payload: dict[str, Any] = {
            "session_id": session_id,
            "format": format,
            "full_page": full_page,
            "timeout": timeout,
            "wait": 0,
        }
        payload["output"] = str(output)
        if quality is not None:
            payload["quality"] = quality
        return _screenshot_result(
            self._request(
                "POST",
                "/screenshot",
                payload,
                timeout=_browser_timeout(timeout, 0),
            )
        )

    def _temporary_output(self, suffix: str) -> Path:
        with self._temporary_lock:
            self._temporary_sequence += 1
            sequence = self._temporary_sequence
        return Path(self._temporary.name) / f"capture-{sequence}.{suffix}"

    def _request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None,
        *,
        timeout: float,
    ) -> dict[str, Any]:
        body = None
        headers = {"Accept": "application/json", "Connection": "close"}
        if payload is not None:
            body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = Request(
            f"{self.base_url}{path}", data=body, headers=headers, method=method
        )
        http_status: int | None = None
        try:
            with urlopen(request, timeout=timeout) as response:
                http_status = response.status
                raw = response.read()
        except HTTPError as error:
            http_status = error.code
            try:
                raw = error.read()
            finally:
                error.close()
        except (URLError, TimeoutError, OSError) as error:
            raise RpcTransportError(
                f"Could not reach REL at {self.base_url}: {error}"
            ) from error

        try:
            envelope = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise RpcProtocolError(
                f"REL returned invalid JSON with HTTP status {http_status}"
            ) from error
        if not isinstance(envelope, dict):
            raise RpcProtocolError("REL RPC response must be a JSON object")
        request_id = envelope.get("request_id")
        if envelope.get("status") == "error":
            _raise_rpc_error(envelope, http_status)
        if envelope.get("status") != "ok":
            raise RpcProtocolError("REL RPC response has an invalid status")
        data = envelope.get("data")
        if not isinstance(request_id, str) or not isinstance(data, dict):
            raise RpcProtocolError("REL success response is missing request_id or data")
        return data


def _validate_base_url(base_url: str) -> str:
    value = base_url.rstrip("/")
    split = urlsplit(value)
    if split.scheme != "http" or split.hostname not in {
        "127.0.0.1",
        "localhost",
        "::1",
    }:
        raise ValueError("REL base URL must be an HTTP loopback URL")
    if split.username is not None or split.password is not None:
        raise ValueError("REL base URL must not include credentials")
    if split.path not in {"", "/v1"} or split.query or split.fragment:
        raise ValueError("REL base URL must end at the /v1 API root")
    return f"{split.scheme}://{split.netloc}/v1"


def _browser_timeout(timeout: float, wait: float) -> float:
    return max(timeout + wait + 15.0, 30.0)


def _raise_rpc_error(envelope: dict[str, Any], http_status: int | None) -> None:
    error = envelope.get("error")
    if not isinstance(error, dict):
        raise RpcProtocolError("REL error response is missing error")
    error_id = error.get("id")
    code = error.get("code")
    message = error.get("message")
    retryable = error.get("retryable")
    details = error.get("details")
    request_id = envelope.get("request_id")
    if (
        not isinstance(error_id, str)
        or not isinstance(code, int)
        or not isinstance(message, str)
        or not isinstance(retryable, bool)
        or (details is not None and not isinstance(details, dict))
    ):
        raise RpcProtocolError("REL returned a malformed RPC error")
    raise RpcError(
        error_id=error_id,
        code=code,
        message=message,
        retryable=retryable,
        details=details,
        request_id=request_id if isinstance(request_id, str) else None,
        http_status=http_status,
    )


def _page_result(data: dict[str, Any]) -> PageResult:
    page = data.get("page")
    capture = data.get("capture")
    if not isinstance(page, dict) or not isinstance(capture, dict):
        raise RpcProtocolError("REL page response is missing page or capture")
    page_id = page.get("id")
    session_id = page.get("session_id")
    url = page.get("url")
    output_path = capture.get("output_path")
    bytesize = capture.get("bytesize")
    status = capture.get("target_http_status")
    if (
        not isinstance(page_id, str)
        or not isinstance(session_id, str)
        or not isinstance(url, str)
        or not isinstance(output_path, str)
        or not isinstance(bytesize, int)
        or (status is not None and not isinstance(status, int))
    ):
        raise RpcProtocolError("REL returned a malformed page response")
    return PageResult(
        page_id=page_id,
        session_id=session_id,
        url=url,
        output_path=Path(output_path),
        bytesize=bytesize,
        status=status,
    )


def _screenshot_result(data: dict[str, Any]) -> ScreenshotResult:
    page = data.get("page")
    screenshot = data.get("screenshot")
    if not isinstance(page, dict) or not isinstance(screenshot, dict):
        raise RpcProtocolError("REL screenshot response is missing page or screenshot")
    page_id = page.get("id")
    session_id = page.get("session_id")
    url = page.get("url")
    output_path = screenshot.get("output_path")
    format = screenshot.get("format")
    mime_type = screenshot.get("mime_type")
    bytesize = screenshot.get("bytesize")
    width = screenshot.get("width")
    height = screenshot.get("height")
    if (
        not isinstance(page_id, str)
        or not isinstance(session_id, str)
        or not isinstance(url, str)
        or not isinstance(output_path, str)
        or not isinstance(format, str)
        or not isinstance(mime_type, str)
        or not isinstance(bytesize, int)
        or not isinstance(width, int)
        or not isinstance(height, int)
    ):
        raise RpcProtocolError("REL returned a malformed screenshot response")
    return ScreenshotResult(
        page_id=page_id,
        session_id=session_id,
        url=url,
        output_path=Path(output_path),
        bytesize=bytesize,
        format=format,
        mime_type=mime_type,
        width=width,
        height=height,
    )
