"""Reusable crawler application configuration."""

from __future__ import annotations

from dataclasses import dataclass, fields
from pathlib import Path
from typing import Any

from .crawler import RelCrawler
from .models import CrawlDefinition, CrawlSummary
from .protocols import CrawlerClient


@dataclass(frozen=True, slots=True)
class CrawlApplication:
    """A crawl definition plus runtime defaults consumable by Python or the CLI."""

    definition: CrawlDefinition
    state_path: str | Path
    capture_dir: str | Path = "captures"
    client: CrawlerClient | None = None
    rel_base_url: str | None = None
    session_id: str | None = None
    profile: str | None = None
    group: str | None = None
    timeout: float = 90.0
    wait: float = 1.0
    action_delay: float = 2.0
    max_attempts: int = 2
    max_session_restarts: int = 1
    max_links: int | None = None
    skip_existing: bool = True
    retry_failed: bool = False
    accept_http_errors: bool = False
    close_owned_session_on_finish: bool = False

    def create_crawler(self, **overrides: Any) -> RelCrawler:
        """Create an independent crawler, applying explicit runtime overrides."""

        values = {
            field.name: getattr(self, field.name)
            for field in fields(self)
            if field.name != "definition"
        }
        unknown = set(overrides) - set(values)
        if unknown:
            names = ", ".join(sorted(unknown))
            raise TypeError(f"unknown crawler application override: {names}")
        values.update(overrides)
        return RelCrawler(self.definition, **values)

    def run(self, **overrides: Any) -> CrawlSummary:
        """Create and run a crawler with optional runtime overrides."""

        return self.create_crawler(**overrides).run()
