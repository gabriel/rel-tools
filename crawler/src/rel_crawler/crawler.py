"""Restartable source-page crawler implemented on REL RPC v1."""

from __future__ import annotations

import logging
import math
import tempfile
from pathlib import Path
from typing import Any

from .api import RelClient
from .captures import CaptureMixin
from .checkpoint import CheckpointMixin
from .errors import CrawlConfigurationError
from .html import canonicalize_url
from .links import LinkQueueMixin
from .models import CrawlDefinition, CrawlSummary
from .pagination import PaginationMixin
from .protocols import CrawlerClient
from .runner import LinkRunnerMixin
from .sessions import SessionMixin
from .source import SourceMixin

_LOGGER = logging.getLogger(__name__)
_LOGGER.addHandler(logging.NullHandler())


class RelCrawler(
    SourceMixin,
    PaginationMixin,
    LinkQueueMixin,
    LinkRunnerMixin,
    CaptureMixin,
    SessionMixin,
    CheckpointMixin,
):
    """Crawl selected links from one page using click, capture, and history back."""

    def __init__(
        self,
        definition: CrawlDefinition,
        *,
        state_path: str | Path,
        capture_dir: str | Path = "captures",
        client: CrawlerClient | None = None,
        rel_base_url: str | None = None,
        session_id: str | None = None,
        profile: str | None = None,
        group: str | None = None,
        timeout: float = 90.0,
        wait: float = 1.0,
        action_delay: float = 2.0,
        max_attempts: int = 2,
        max_session_restarts: int = 1,
        max_links: int | None = None,
        skip_existing: bool = True,
        retry_failed: bool = False,
        accept_http_errors: bool = False,
        close_owned_session_on_finish: bool = False,
    ) -> None:
        if client is not None and rel_base_url is not None:
            raise CrawlConfigurationError(
                "client and rel_base_url cannot be supplied together"
            )
        try:
            canonicalize_url(definition.start_url)
        except (ValueError, UnicodeError) as error:
            raise CrawlConfigurationError(str(error)) from error
        if profile is not None and (not isinstance(profile, str) or not profile.strip()):
            raise CrawlConfigurationError("profile must not be empty")
        for name, selector in (
            ("source_ready_selector", definition.source_ready_selector),
            ("capture_ready_selector", definition.capture_ready_selector),
            ("load_more_selector", definition.load_more_selector),
        ):
            if selector is not None and (
                not isinstance(selector, str) or not selector.strip()
            ):
                raise CrawlConfigurationError(f"{name} must be a non-empty string")
        if (
            type(definition.load_more_clicks) is not int
            or definition.load_more_clicks < 0
        ):
            raise CrawlConfigurationError(
                "load_more_clicks must be a non-negative integer"
            )
        if (definition.load_more_selector is None) != (
            definition.load_more_clicks == 0
        ):
            raise CrawlConfigurationError(
                "load_more_selector and a positive load_more_clicks must be "
                "set together"
            )
        if definition.load_more_clicks and definition.extract_links is not None:
            raise CrawlConfigurationError(
                "load-more crawling requires REL rendered-link discovery"
            )
        if (
            not math.isfinite(timeout)
            or timeout <= 0
            or not math.isfinite(wait)
            or wait < 0
        ):
            raise CrawlConfigurationError(
                "timeout must be positive and wait non-negative"
            )
        if not math.isfinite(action_delay) or action_delay < 0:
            raise CrawlConfigurationError(
                "action_delay must be finite and non-negative"
            )
        if max_attempts < 1:
            raise CrawlConfigurationError("max_attempts must be at least 1")
        if max_session_restarts < 0:
            raise CrawlConfigurationError("max_session_restarts must not be negative")
        if max_links is not None and max_links < 1:
            raise CrawlConfigurationError("max_links must be at least 1")

        self.definition = definition
        self.state_path = Path(state_path).expanduser().resolve()
        self.capture_dir = Path(capture_dir).expanduser().resolve()
        self.client = client if client is not None else RelClient(rel_base_url)
        self.requested_session_id = session_id
        self.profile = profile.strip() if profile is not None else None
        self.group = group.strip() if group is not None else self._default_group()
        if not self.group:
            raise CrawlConfigurationError("group must not be empty")
        self.timeout = float(timeout)
        self.wait = float(wait)
        self.action_delay = float(action_delay)
        self.max_attempts = max_attempts
        self.max_session_restarts = max_session_restarts
        self.max_links = max_links
        self.skip_existing = skip_existing
        self.retry_failed = retry_failed
        self.accept_http_errors = accept_http_errors
        self.close_owned_session_on_finish = close_owned_session_on_finish
        self._state: dict[str, Any] = {}
        self._source_url = canonicalize_url(definition.start_url)
        self.source_ready_selector = (
            definition.source_ready_selector.strip()
            if definition.source_ready_selector is not None
            else None
        )
        self.capture_ready_selector = (
            definition.capture_ready_selector.strip()
            if definition.capture_ready_selector is not None
            else None
        )
        self.load_more_selector = (
            definition.load_more_selector.strip()
            if definition.load_more_selector is not None
            else None
        )
        self.load_more_clicks = definition.load_more_clicks
        self._has_browser_action = False
        self._load_more_depth = 0

    def run(self) -> CrawlSummary:
        """Run or resume the crawl while holding an exclusive checkpoint lock."""

        _LOGGER.info(
            "starting crawl source=%s checkpoint=%s captures=%s",
            self.definition.start_url,
            self.state_path,
            self.capture_dir,
        )
        with self._checkpoint_lock():
            _LOGGER.info("acquired checkpoint lock %s", self.state_path)
            self._has_browser_action = False
            self._load_more_depth = 0
            self._state = self._load_state()
            _LOGGER.info(
                "loaded checkpoint with %d entries and session %s",
                len(self._state["entries"]),
                self._state.get("session_id") or "none",
            )
            self._recover_interrupted_entries()
            self._recover_misdirected_captures()
            self._retry_failed_entries()
            _LOGGER.info("checking REL health")
            health = self.client.health()
            _LOGGER.info("REL is healthy: %s", health.get("version", "unknown version"))
            session_id = self._ensure_session()
            _LOGGER.info(
                "using REL session %s (managed=%s, generation=%s)",
                session_id,
                self._state["managed_session"],
                self._state["session_generation"],
            )

            self.capture_dir.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(prefix="rel-crawler-") as temporary:
                temporary_dir = Path(temporary)
                (
                    session_id,
                    _source,
                    discovered,
                ) = self._load_source_with_session_recovery(
                    session_id,
                    temporary_dir / "source.html",
                )
                discoverable_keys, active_keys = self._merge_links(
                    discovered,
                    reconcile_unavailable=False,
                )
                self._skip_existing_captures(allowed_keys=active_keys)
                self._skip_callback_links(allowed_keys=active_keys)
                self._backfill_metadata()
                session_id = self._crawl_pending(
                    session_id,
                    temporary_dir / "recovery.html",
                    allowed_keys=active_keys,
                )
                for click_number in range(1, self.load_more_clicks + 1):
                    session_id, expanded = self._expand_source_with_session_recovery(
                        session_id,
                        temporary_dir / "load-more.html",
                        click_number=click_number,
                    )
                    if expanded is None:
                        break
                    self._load_more_depth = click_number
                    discovered = expanded
                    discoverable_keys, active_keys = self._merge_links(
                        discovered,
                        reconcile_unavailable=False,
                    )
                    self._skip_existing_captures(allowed_keys=active_keys)
                    self._skip_callback_links(allowed_keys=active_keys)
                    self._backfill_metadata()
                    session_id = self._crawl_pending(
                        session_id,
                        temporary_dir / "recovery.html",
                        allowed_keys=active_keys,
                    )
                self._reconcile_unavailable(discoverable_keys)

            summary = self._summary(session_id)
            if (
                self.close_owned_session_on_finish
                and self._state["managed_session"]
                and summary.pending == 0
            ):
                _LOGGER.info("closing completed crawler-owned session %s", session_id)
                self.client.delete_session(session_id)
                self._state["session_id"] = None
                self._save_state()
                _LOGGER.info("closed completed crawler-owned session %s", session_id)
            _LOGGER.info(
                "crawl complete: discovered=%d captured=%d failed=%d pending=%d "
                "skipped_existing=%d session=%s",
                summary.discovered,
                summary.captured,
                summary.failed,
                summary.pending,
                summary.skipped_existing,
                summary.session_id,
            )
            return summary
