"""Shared implementation for the synchronous and asynchronous public surfaces."""

from __future__ import annotations

import re
import time
import uuid
from http import HTTPStatus
from math import isfinite
from pathlib import Path
from typing import Any, Literal, NoReturn

from selectolax.parser import HTMLParser, Node

from ._rpc import (
    PageResult,
    RelRpcClient,
    RpcError,
    RpcProtocolError,
    RpcTransportError,
)

DEFAULT_TIMEOUT_MS = 30_000.0
DEFAULT_PROFILE = None
SUPPORTED_WAIT_UNTIL = {"load", "domcontentloaded", "commit"}
SUPPORTED_SCREENSHOT_FORMATS = {"png", "jpeg", "webp"}


class Error(Exception):
    """Base exception for the REL Playwright compatibility surface."""


class TimeoutError(Error):
    """A REL operation or native selector wait exceeded its deadline."""


class UnsupportedError(Error):
    """The requested Playwright feature has no safe REL RPC equivalent."""


class RelRpcError(Error):
    """A structured REL RPC error that was not translated to another API error."""

    def __init__(self, error: RpcError) -> None:
        super().__init__(str(error))
        self.id = error.id
        self.code = error.code
        self.message = error.message
        self.retryable = error.retryable
        self.details = error.details
        self.request_id = error.request_id
        self.http_status = error.http_status


class Response:
    """Navigation response metadata available from REL's main-frame capture."""

    def __init__(self, url: str, status: int | None) -> None:
        self._url = url
        self._status = status

    @property
    def url(self) -> str:
        return self._url

    @property
    def status(self) -> int:
        return self._status if self._status is not None else 0

    @property
    def ok(self) -> bool:
        return self._status is not None and 200 <= self._status <= 299

    @property
    def status_text(self) -> str:
        if self._status is None:
            return ""
        try:
            return HTTPStatus(self._status).phrase
        except ValueError:
            return ""


class Locator:
    """A CSS locator evaluated against fresh REL-rendered HTML."""

    def __init__(self, page: Page, selector: str, index: int | None = None) -> None:
        selector = selector.strip()
        if not selector:
            raise Error("locator selector must not be empty")
        self._page = page
        self._selector = selector
        self._index = index

    @property
    def page(self) -> Page:
        return self._page

    @property
    def first(self) -> Locator:
        return Locator(self._page, self._selector, 0)

    @property
    def last(self) -> Locator:
        return Locator(self._page, self._selector, -1)

    def nth(self, index: int) -> Locator:
        if not isinstance(index, int):
            raise TypeError("locator.nth index must be an integer")
        return Locator(self._page, self._selector, index)

    def locator(self, selector: str) -> Locator:
        selector = selector.strip()
        if not selector:
            raise Error("locator selector must not be empty")
        if self._index is not None:
            raise UnsupportedError(
                "nested locators from first, last, or nth are not supported"
            )
        return Locator(self._page, f"{self._selector} {selector}")

    def count(self) -> int:
        return len(self._nodes())

    def all(self) -> list[Locator]:
        return [
            Locator(self._page, self._selector, index) for index in range(self.count())
        ]

    def all_text_contents(self) -> list[str]:
        return [node.text(separator="", strip=False) for node in self._nodes()]

    def all_inner_texts(self) -> list[str]:
        return [_inner_text(node) for node in self._nodes()]

    def text_content(self, *, timeout: float | None = None) -> str | None:
        del timeout
        return self._node().text(separator="", strip=False)

    def inner_text(self, *, timeout: float | None = None) -> str:
        del timeout
        return _inner_text(self._node())

    def inner_html(self, *, timeout: float | None = None) -> str:
        del timeout
        child = self._node().child
        fragments: list[str] = []
        while child is not None:
            fragments.append(child.html or "")
            child = child.next
        return "".join(fragments)

    def get_attribute(self, name: str, *, timeout: float | None = None) -> str | None:
        del timeout
        if not isinstance(name, str) or not name:
            raise Error("attribute name must not be empty")
        return self._node().attributes.get(name)

    def input_value(self, *, timeout: float | None = None) -> str:
        del timeout
        raise UnsupportedError(
            "REL RPC does not expose live form-control properties; "
            "get_attribute('value') reads the serialized HTML attribute"
        )

    def click(
        self,
        *,
        timeout: float | None = None,
        force: bool | None = None,
        no_wait_after: bool | None = None,
        **kwargs: Any,
    ) -> None:
        _reject_options("locator.click", kwargs)
        if force:
            raise UnsupportedError("locator.click(force=True) is not supported by REL")
        if no_wait_after:
            raise UnsupportedError(
                "locator.click(no_wait_after=True) is not supported by REL"
            )
        self._interaction_selector()
        self._page._perform(
            [{"action": "click", "selector": self._selector}], timeout_ms=timeout
        )

    def fill(
        self,
        value: str,
        *,
        timeout: float | None = None,
        force: bool | None = None,
        no_wait_after: bool | None = None,
        **kwargs: Any,
    ) -> None:
        _reject_options("locator.fill", kwargs)
        if force or no_wait_after:
            raise UnsupportedError(
                "force and no_wait_after are not supported by REL fill"
            )
        self._interaction_selector()
        self._page._perform(
            [
                {"action": "clear", "selector": self._selector},
                {"action": "type", "selector": self._selector, "text": value},
            ],
            timeout_ms=timeout,
        )

    def type(
        self,
        text: str,
        *,
        delay: float | None = None,
        timeout: float | None = None,
        no_wait_after: bool | None = None,
        **kwargs: Any,
    ) -> None:
        _reject_options("locator.type", kwargs)
        if delay not in {None, 0}:
            raise UnsupportedError("per-character type delay is not supported by REL")
        if no_wait_after:
            raise UnsupportedError("type(no_wait_after=True) is not supported by REL")
        self._interaction_selector()
        self._page._perform(
            [{"action": "type", "selector": self._selector, "text": text}],
            timeout_ms=timeout,
        )

    def press(
        self,
        key: str,
        *,
        delay: float | None = None,
        timeout: float | None = None,
        no_wait_after: bool | None = None,
        **kwargs: Any,
    ) -> None:
        _reject_options("locator.press", kwargs)
        if delay not in {None, 0}:
            raise UnsupportedError("keyboard delay is not supported by REL")
        if no_wait_after:
            raise UnsupportedError("press(no_wait_after=True) is not supported by REL")
        self._interaction_selector()
        self._page._perform(
            [{"action": "press", "selector": self._selector, "key": key}],
            timeout_ms=timeout,
        )

    def select_option(
        self,
        value: str | None = None,
        *,
        timeout: float | None = None,
        no_wait_after: bool | None = None,
        **kwargs: Any,
    ) -> list[str]:
        _reject_options("locator.select_option", kwargs)
        if value is None or not isinstance(value, str):
            raise UnsupportedError("REL select_option requires one exact string value")
        if no_wait_after:
            raise UnsupportedError(
                "select_option(no_wait_after=True) is not supported by REL"
            )
        self._interaction_selector()
        self._page._perform(
            [{"action": "select", "selector": self._selector, "value": value}],
            timeout_ms=timeout,
        )
        return [value]

    def wait_for(
        self,
        *,
        state: Literal["attached", "visible"] = "visible",
        timeout: float | None = None,
    ) -> None:
        if state not in {"attached", "visible"}:
            raise UnsupportedError(
                "REL locator.wait_for supports only attached or visible; both map to "
                "native DOM presence"
            )
        self._page._wait_for_selector(self._selector, timeout_ms=timeout)

    def evaluate(self, *_args: Any, **_kwargs: Any) -> Any:
        raise UnsupportedError("caller-supplied JavaScript is not supported by REL")

    def evaluate_all(self, *_args: Any, **_kwargs: Any) -> Any:
        raise UnsupportedError("caller-supplied JavaScript is not supported by REL")

    def _nodes(self) -> list[Node]:
        document = self._page._document()
        try:
            return list(document.css(self._selector))
        except ValueError as error:
            raise Error(
                f"invalid or unsupported CSS selector {self._selector!r}: {error}"
            ) from error

    def _node(self) -> Node:
        nodes = self._nodes()
        if self._index is None:
            if len(nodes) != 1:
                raise Error(
                    f"strict mode violation: locator({self._selector!r}) resolved to "
                    f"{len(nodes)} elements"
                )
            return nodes[0]
        try:
            return nodes[self._index]
        except IndexError as error:
            raise Error(
                f"locator({self._selector!r}).nth({self._index}) resolved to no element"
            ) from error

    def _interaction_selector(self) -> str:
        if self._index is None:
            self._node()
            return self._selector
        nodes = self._nodes()
        try:
            nodes[self._index]
        except IndexError as error:
            raise Error(
                f"locator({self._selector!r}).nth({self._index}) resolved to no element"
            ) from error
        if len(nodes) != 1:
            raise UnsupportedError(
                "REL cannot safely translate first, last, or nth interaction into its "
                "strict native CSS action subset; use a unique selector"
            )
        return self._selector


class Page:
    """One visible REL session exposed with a scraping-focused Page API."""

    def __init__(self, context: BrowserContext, session_id: str, owned: bool) -> None:
        self._context = context
        self._client = context._browser._client
        self._session_id = session_id
        self._owned = owned
        self._closed = False
        self._page_id: str | None = None
        self._url = "about:blank"
        self._html: str | None = None
        self._default_timeout_ms = DEFAULT_TIMEOUT_MS
        self._default_navigation_timeout_ms = DEFAULT_TIMEOUT_MS

    @property
    def context(self) -> BrowserContext:
        return self._context

    @property
    def url(self) -> str:
        return self._url

    @property
    def session_id(self) -> str:
        """The immutable REL session backing this page."""

        return self._session_id

    def is_closed(self) -> bool:
        return self._closed

    def set_default_timeout(self, timeout: float) -> None:
        self._default_timeout_ms = _validate_timeout_ms(timeout)

    def set_default_navigation_timeout(self, timeout: float) -> None:
        self._default_navigation_timeout_ms = _validate_timeout_ms(timeout)

    def goto(
        self,
        url: str,
        *,
        timeout: float | None = None,
        wait_until: Literal["load", "domcontentloaded", "commit"] | None = None,
        referer: str | None = None,
        **kwargs: Any,
    ) -> Response:
        _reject_options("page.goto", kwargs)
        self._ensure_open()
        if referer is not None:
            raise UnsupportedError("page.goto(referer=...) is not supported by REL")
        if wait_until is not None and wait_until not in SUPPORTED_WAIT_UNTIL:
            raise UnsupportedError(
                "REL supports load, domcontentloaded, or commit readiness; "
                "networkidle is not supported"
            )
        timeout_ms = self._default_navigation_timeout_ms if timeout is None else timeout
        timeout_seconds = _seconds(timeout_ms)
        try:
            result = self._client.navigate(
                url=url,
                session_id=self._session_id,
                timeout=timeout_seconds,
                wait=self._context._browser._wait,
            )
        except RpcError as error:
            if error.id != "UPSTREAM_UNAVAILABLE" or not isinstance(
                error.details, dict
            ):
                _raise_public_error(error)
            status = error.details.get("target_http_status")
            final_url = error.details.get("url")
            if not isinstance(status, int) or not isinstance(final_url, str):
                _raise_public_error(error)
            result = self._capture(timeout_ms=timeout_ms)
            self._url = final_url
            self._slow_down()
            return Response(final_url, status)
        except (RpcTransportError, RpcProtocolError) as error:
            raise Error(str(error)) from error
        self._update(result)
        self._slow_down()
        return Response(result.url, result.status)

    def content(self) -> str:
        self._ensure_open()
        if self._page_id is None:
            return "<html><head></head><body></body></html>"
        self._capture()
        assert self._html is not None
        return self._html

    def title(self) -> str:
        title = self._document().css_first("title")
        return "" if title is None else title.text(separator="", strip=True)

    def locator(self, selector: str, **kwargs: Any) -> Locator:
        _reject_options("page.locator", kwargs)
        return Locator(self, selector)

    def click(self, selector: str, **kwargs: Any) -> None:
        self.locator(selector).click(**kwargs)

    def fill(self, selector: str, value: str, **kwargs: Any) -> None:
        self.locator(selector).fill(value, **kwargs)

    def type(self, selector: str, text: str, **kwargs: Any) -> None:
        self.locator(selector).type(text, **kwargs)

    def press(self, selector: str, key: str, **kwargs: Any) -> None:
        self.locator(selector).press(key, **kwargs)

    def select_option(
        self, selector: str, value: str | None = None, **kwargs: Any
    ) -> list[str]:
        return self.locator(selector).select_option(value, **kwargs)

    def wait_for_selector(
        self,
        selector: str,
        *,
        state: Literal["attached", "visible"] = "visible",
        timeout: float | None = None,
        **kwargs: Any,
    ) -> Locator:
        _reject_options("page.wait_for_selector", kwargs)
        locator = self.locator(selector)
        locator.wait_for(state=state, timeout=timeout)
        return locator

    def wait_for_timeout(self, timeout: float) -> None:
        if (
            isinstance(timeout, bool)
            or not isinstance(timeout, (int, float))
            or not isfinite(timeout)
            or timeout < 0
        ):
            raise Error("page.wait_for_timeout requires non-negative milliseconds")
        time.sleep(timeout / 1000.0)

    def wait_for_load_state(
        self,
        state: Literal["load", "domcontentloaded", "commit"] = "load",
        *,
        timeout: float | None = None,
    ) -> None:
        del timeout
        if state not in SUPPORTED_WAIT_UNTIL:
            raise UnsupportedError("REL does not expose Playwright networkidle state")
        self._ensure_open()

    def screenshot(
        self,
        *,
        path: str | Path | None = None,
        full_page: bool = False,
        type: Literal["png", "jpeg", "webp"] | None = None,
        quality: int | None = None,
        timeout: float | None = None,
        **kwargs: Any,
    ) -> bytes:
        _reject_options("page.screenshot", kwargs)
        self._ensure_open()
        output = Path(path).expanduser().resolve() if path is not None else None
        format = type or _screenshot_format(output)
        if format not in SUPPORTED_SCREENSHOT_FORMATS:
            raise UnsupportedError(f"unsupported screenshot type {format!r}")
        if quality is not None and (
            isinstance(quality, bool)
            or not isinstance(quality, int)
            or not 0 <= quality <= 100
        ):
            raise Error("screenshot quality must be an integer from 0 through 100")
        if format == "png" and quality is not None:
            raise Error("screenshot quality is not applicable to PNG")
        timeout_seconds = _seconds(
            self._default_timeout_ms if timeout is None else timeout
        )
        try:
            result = self._client.screenshot(
                session_id=self._session_id,
                output=output,
                format=format,
                quality=quality,
                full_page=full_page,
                timeout=timeout_seconds,
            )
        except RpcError as error:
            _raise_public_error(error)
        except (RpcTransportError, RpcProtocolError) as error:
            raise Error(str(error)) from error
        self._page_id = result.page_id
        self._url = result.url
        self._slow_down()
        try:
            return result.output_path.read_bytes()
        except OSError as error:
            raise Error(
                f"could not read REL screenshot {result.output_path}: {error}"
            ) from error

    def go_back(
        self, *, timeout: float | None = None, wait_until: str | None = None
    ) -> Response | None:
        return self._history("back", timeout=timeout, wait_until=wait_until)

    def go_forward(
        self, *, timeout: float | None = None, wait_until: str | None = None
    ) -> Response | None:
        return self._history("forward", timeout=timeout, wait_until=wait_until)

    def reload(
        self, *, timeout: float | None = None, wait_until: str | None = None
    ) -> Response:
        response = self._history("reload", timeout=timeout, wait_until=wait_until)
        assert response is not None
        return response

    def close(self, *, run_before_unload: bool | None = None, **kwargs: Any) -> None:
        _reject_options("page.close", kwargs)
        if run_before_unload:
            raise UnsupportedError(
                "page.close(run_before_unload=True) is not supported"
            )
        self._context._close_page(self)

    def evaluate(self, *_args: Any, **_kwargs: Any) -> Any:
        raise UnsupportedError("caller-supplied JavaScript is not supported by REL")

    def _history(
        self, navigation: str, *, timeout: float | None, wait_until: str | None
    ) -> Response | None:
        self._ensure_open()
        if wait_until is not None and wait_until not in SUPPORTED_WAIT_UNTIL:
            raise UnsupportedError("REL does not expose Playwright networkidle state")
        timeout_ms = self._default_navigation_timeout_ms if timeout is None else timeout
        try:
            result = self._client.history(
                navigation=navigation,
                session_id=self._session_id,
                timeout=_seconds(timeout_ms),
                wait=self._context._browser._wait,
            )
        except RpcError as error:
            if navigation == "back" and error.id == "ACTIVE_PAGE_NOT_FOUND":
                return None
            _raise_public_error(error)
        except (RpcTransportError, RpcProtocolError) as error:
            raise Error(str(error)) from error
        self._update(result)
        self._slow_down()
        return Response(result.url, result.status)

    def _capture(self, *, timeout_ms: float | None = None) -> PageResult:
        self._ensure_open()
        try:
            result = self._client.capture(
                session_id=self._session_id,
                timeout=_seconds(
                    self._default_timeout_ms if timeout_ms is None else timeout_ms
                ),
                wait=0,
            )
        except RpcError as error:
            _raise_public_error(error)
        except (RpcTransportError, RpcProtocolError) as error:
            raise Error(str(error)) from error
        self._update(result)
        return result

    def _perform(
        self, actions: list[dict[str, Any]], *, timeout_ms: float | None
    ) -> None:
        self._ensure_open()
        try:
            result = self._client.perform(
                actions=actions,
                session_id=self._session_id,
                timeout=_seconds(
                    self._default_timeout_ms if timeout_ms is None else timeout_ms
                ),
                wait=self._context._browser._wait,
            )
        except RpcError as error:
            _raise_public_error(error)
        except (RpcTransportError, RpcProtocolError) as error:
            raise Error(str(error)) from error
        self._update(result)
        self._slow_down()

    def _wait_for_selector(self, selector: str, *, timeout_ms: float | None) -> None:
        timeout_value = self._default_timeout_ms if timeout_ms is None else timeout_ms
        self._perform(
            [
                {
                    "action": "wait-for",
                    "selector": selector,
                    "timeout": _seconds(timeout_value),
                }
            ],
            timeout_ms=timeout_value,
        )

    def _document(self) -> HTMLParser:
        return HTMLParser(self.content())

    def _update(self, result: PageResult) -> None:
        try:
            html = result.output_path.read_text(encoding="utf-8")
        except OSError as error:
            raise Error(
                f"could not read REL capture {result.output_path}: {error}"
            ) from error
        self._page_id = result.page_id
        self._url = result.url
        self._html = html

    def _slow_down(self) -> None:
        delay = self._context._browser._slow_mo_ms
        if delay:
            time.sleep(delay / 1000.0)

    def _ensure_open(self) -> None:
        if self._closed:
            raise Error("page is closed")
        if not self._context._browser.is_connected():
            raise Error("browser is closed")


class BrowserContext:
    """A REL launch configuration that creates one isolated session per page."""

    def __init__(
        self,
        browser: Browser,
        *,
        profile: str | None,
        session_id: str | None,
        group: str,
        persist: bool,
    ) -> None:
        self._browser = browser
        self._profile = profile
        self._session_id = session_id
        self._group = group
        self._persist = persist
        self._existing_session_claimed = False
        self._closed = False
        self._pages: list[Page] = []
        self._default_timeout_ms = DEFAULT_TIMEOUT_MS
        self._default_navigation_timeout_ms = DEFAULT_TIMEOUT_MS

    @property
    def browser(self) -> Browser:
        return self._browser

    @property
    def pages(self) -> list[Page]:
        return [page for page in self._pages if not page.is_closed()]

    def new_page(self) -> Page:
        if self._closed:
            raise Error("browser context is closed")
        if self._session_id is not None:
            if self._existing_session_claimed:
                raise UnsupportedError(
                    "one existing REL session can back only one compatibility Page"
                )
            session_id = self._session_id
            owned = False
            self._existing_session_claimed = True
        else:
            try:
                session_id = self._browser._client.create_session(
                    profile=self._profile, group=self._group
                )
            except RpcError as error:
                _raise_public_error(error)
            except (RpcTransportError, RpcProtocolError) as error:
                raise Error(str(error)) from error
            owned = True
        page = Page(self, session_id, owned)
        page.set_default_timeout(self._default_timeout_ms)
        page.set_default_navigation_timeout(self._default_navigation_timeout_ms)
        self._pages.append(page)
        return page

    def set_default_timeout(self, timeout: float) -> None:
        value = _validate_timeout_ms(timeout)
        self._default_timeout_ms = value
        for page in self._pages:
            page.set_default_timeout(value)

    def set_default_navigation_timeout(self, timeout: float) -> None:
        value = _validate_timeout_ms(timeout)
        self._default_navigation_timeout_ms = value
        for page in self._pages:
            page.set_default_navigation_timeout(value)

    def close(self, *, reason: str | None = None) -> None:
        del reason
        if self._closed:
            return
        for page in list(self._pages):
            self._close_page(page)
        self._closed = True
        if self in self._browser._contexts:
            self._browser._contexts.remove(self)

    def _close_page(self, page: Page) -> None:
        if page._closed:
            return
        if page._owned and not self._persist:
            try:
                self._browser._client.delete_session(page.session_id)
            except RpcError as error:
                if error.id != "SESSION_NOT_FOUND":
                    _raise_public_error(error)
            except (RpcTransportError, RpcProtocolError) as error:
                raise Error(str(error)) from error
        page._closed = True


class Browser:
    """A connected REL compatibility client."""

    def __init__(
        self,
        client: RelRpcClient,
        *,
        profile: str | None,
        session_id: str | None,
        group: str,
        persist: bool,
        slow_mo_ms: float,
        wait: float,
    ) -> None:
        self._client = client
        self._profile = profile
        self._session_id = session_id
        self._group = group
        self._persist = persist
        self._slow_mo_ms = slow_mo_ms
        self._wait = wait
        self._connected = True
        self._contexts: list[BrowserContext] = []

    @property
    def contexts(self) -> list[BrowserContext]:
        return list(self._contexts)

    def is_connected(self) -> bool:
        return self._connected

    def new_context(self, **kwargs: Any) -> BrowserContext:
        self._ensure_connected()
        profile = kwargs.pop("profile", self._profile)
        session_id = kwargs.pop("session_id", self._session_id)
        group = kwargs.pop("group", self._group)
        persist = kwargs.pop("persist", self._persist)
        _reject_options("browser.new_context", kwargs)
        _validate_rel_options(profile, session_id, group, persist)
        if self._session_id is not None and self._contexts:
            raise UnsupportedError(
                "a Browser launched with session_id can create only one context"
            )
        context = BrowserContext(
            self,
            profile=profile,
            session_id=session_id,
            group=group,
            persist=persist,
        )
        self._contexts.append(context)
        return context

    def new_page(self, **kwargs: Any) -> Page:
        return self.new_context(**kwargs).new_page()

    def close(self, *, reason: str | None = None) -> None:
        del reason
        if not self._connected:
            return
        try:
            for context in list(self._contexts):
                context.close()
        finally:
            self._client.close()
            self._connected = False

    def _ensure_connected(self) -> None:
        if not self._connected:
            raise Error("browser is closed")


class BrowserType:
    """REL-backed Chromium launcher with explicit REL configuration options."""

    def __init__(self, name: str, browser_registry: list[Browser]) -> None:
        self.name = name
        self._browser_registry = browser_registry

    def launch(
        self,
        *,
        headless: bool | None = None,
        slow_mo: float | None = None,
        timeout: float | None = None,
        profile: str | None = DEFAULT_PROFILE,
        session_id: str | None = None,
        group: str | None = None,
        persist: bool = False,
        rel_base_url: str | None = None,
        wait: float = 0,
        **kwargs: Any,
    ) -> Browser:
        if self.name != "chromium":
            raise UnsupportedError(
                "REL embeds Chromium; Firefox and WebKit are unavailable"
            )
        if headless is True:
            raise UnsupportedError(
                "REL owns a visible browser and cannot launch headless"
            )
        _reject_options("browser_type.launch", kwargs)
        group = group or f"rel-playwright-{uuid.uuid4().hex}"
        slow_mo = 0.0 if slow_mo is None else slow_mo
        _validate_rel_options(profile, session_id, group, persist)
        if (
            isinstance(slow_mo, bool)
            or not isinstance(slow_mo, (int, float))
            or not isfinite(slow_mo)
            or slow_mo < 0
        ):
            raise Error("slow_mo must be non-negative milliseconds")
        if (
            isinstance(wait, bool)
            or not isinstance(wait, (int, float))
            or not isfinite(wait)
            or wait < 0
        ):
            raise Error("wait must be non-negative seconds")
        client = RelRpcClient(rel_base_url)
        try:
            client.health(timeout=_seconds(5_000.0 if timeout is None else timeout))
            if session_id is not None:
                client.get_session(session_id)
        except RpcError as error:
            _raise_public_error(error)
        except (RpcTransportError, RpcProtocolError) as error:
            raise Error(str(error)) from error
        browser = Browser(
            client,
            profile=profile,
            session_id=session_id,
            group=group,
            persist=persist,
            slow_mo_ms=float(slow_mo),
            wait=float(wait),
        )
        self._browser_registry.append(browser)
        return browser

    def connect(self, *_args: Any, **_kwargs: Any) -> Browser:
        raise UnsupportedError(
            "REL does not expose a Playwright protocol socket; use launch(session_id=...)"
        )

    def connect_over_cdp(self, *_args: Any, **_kwargs: Any) -> Browser:
        raise UnsupportedError(
            "REL does not expose CDP; use launch(profile=...) or launch(session_id=...)"
        )


class Playwright:
    """Top-level compatibility object returned by sync_playwright()."""

    def __init__(self) -> None:
        self._browsers: list[Browser] = []
        self.chromium = BrowserType("chromium", self._browsers)
        self.firefox = BrowserType("firefox", self._browsers)
        self.webkit = BrowserType("webkit", self._browsers)

    def stop(self) -> None:
        for browser in list(self._browsers):
            browser.close()


def _raise_public_error(error: RpcError) -> NoReturn:
    if error.id in {"TIMEOUT", "ACTION_TIMEOUT"}:
        raise TimeoutError(error.message) from error
    raise RelRpcError(error) from error


def _validate_rel_options(
    profile: Any, session_id: Any, group: Any, persist: Any
) -> None:
    if profile is not None and (not isinstance(profile, str) or not profile.strip()):
        raise Error("profile must be a non-empty REL Profile name")
    if session_id is not None and (
        not isinstance(session_id, str)
        or re.fullmatch(r"Session[1-9][0-9]*", session_id, re.IGNORECASE) is None
    ):
        raise Error("session_id must use the Session<number> format")
    if not isinstance(group, str) or not group.strip():
        raise Error("group must be a non-empty REL session group")
    if not isinstance(persist, bool):
        raise Error("persist must be a boolean")


def _reject_options(operation: str, options: dict[str, Any]) -> None:
    if options:
        names = ", ".join(sorted(options))
        raise UnsupportedError(f"{operation} does not support option(s): {names}")


def _validate_timeout_ms(value: float) -> float:
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not isfinite(value)
        or value <= 0
    ):
        raise UnsupportedError(
            "REL requires a positive bounded timeout; Playwright timeout=0 is unsupported"
        )
    return float(value)


def _seconds(milliseconds: float) -> float:
    return _validate_timeout_ms(milliseconds) / 1000.0


def _screenshot_format(path: Path | None) -> str:
    if path is None:
        return "png"
    suffix = path.suffix.lower()
    if suffix in {".jpg", ".jpeg"}:
        return "jpeg"
    if suffix == ".webp":
        return "webp"
    return "png"


def _inner_text(node: Node) -> str:
    return " ".join(node.text(separator=" ", strip=True).split())
