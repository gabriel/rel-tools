"""Structural client contract used by the crawler engine."""

from __future__ import annotations

from pathlib import Path
from typing import Any, Protocol

from .api import LinkObservation, NavigationOperation, PageOperation


class CrawlerClient(Protocol):
    """REL client operations required by :class:`RelCrawler`."""

    def health(self) -> dict[str, Any]: ...

    def get_session(self, session_id: str) -> dict[str, Any] | None: ...

    def create_session(self, *, profile: str | None, group: str) -> str: ...

    def delete_session(self, session_id: str) -> None: ...

    def navigate(
        self,
        *,
        url: str,
        session_id: str,
        output: Path,
        timeout: float,
        wait: float,
    ) -> PageOperation: ...

    def perform(
        self,
        *,
        actions: list[dict[str, Any]],
        session_id: str,
        output: Path,
        timeout: float,
        wait: float,
    ) -> PageOperation: ...

    def back(
        self, *, session_id: str, timeout: float, wait: float
    ) -> NavigationOperation: ...

    def observe_links(
        self, *, session_id: str, timeout: float, wait: float
    ) -> LinkObservation: ...
