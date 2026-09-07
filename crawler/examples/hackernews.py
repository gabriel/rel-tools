"""Capture discussion pages linked from the Hacker News front page."""

from __future__ import annotations

import hashlib
import logging
import os
import re
import sys
from pathlib import Path
from urllib.parse import parse_qs, parse_qsl, urlsplit

from rel_crawler import (
    CapturedPage,
    CrawlApplication,
    CrawlDefinition,
    CrawlItem,
    Link,
)

ROOT = Path(__file__).resolve().parent / "hackernews-output"
START_URL = os.environ.get("HN_START_URL", "https://news.ycombinator.com/news")
PROFILE = os.environ.get("REL_PROFILE")
_ITEM_ID = re.compile(r"^[0-9]+$")


def positive_int_environment(name: str) -> int | None:
    value = os.environ.get(name)
    if value is None:
        return None
    parsed = int(value)
    if parsed < 1:
        raise ValueError(f"{name} must be a positive integer")
    return parsed


MAX_LINKS = positive_int_environment("HN_MAX_LINKS")


def item_id(url: str) -> str | None:
    parsed = urlsplit(url)
    values = parse_qs(parsed.query, keep_blank_values=True).get("id", [])
    if len(values) != 1 or _ITEM_ID.fullmatch(values[0]) is None:
        return None
    return values[0]


def is_post(link: Link) -> bool:
    """Select Hacker News item discussions and ignore new-window targets."""

    parsed = urlsplit(link.url)
    return (
        parsed.hostname == "news.ycombinator.com"
        and parsed.path == "/item"
        and item_id(link.url) is not None
        and link.target != "_blank"
    )


def post_path(item: CrawlItem) -> Path:
    """Map item IDs to directories and keep other query variants distinct."""

    parsed = urlsplit(item.link.url)
    identifier = item_id(item.link.url)
    if identifier is None:
        raise ValueError(f"expected a Hacker News item URL, got {item.link.url!r}")
    filename = "index.html"
    extra_query = [
        (key, value)
        for key, value in parse_qsl(parsed.query, keep_blank_values=True)
        if key != "id"
    ]
    if extra_query:
        digest = hashlib.sha256(parsed.query.encode("utf-8")).hexdigest()[:12]
        filename = f"index.query-{digest}.html"
    return ROOT / "item" / identifier / filename


def report(capture: CapturedPage) -> None:
    print(
        f"captured {capture.url} -> {capture.output_path} "
        f"(metadata: {capture.metadata_path})"
    )


definition = CrawlDefinition(
    start_url=START_URL,
    select_link=is_post,
    capture_path=post_path,
    process_capture=report,
    source_ready_selector="a.morelink",
    capture_ready_selector="table.comment-tree",
)

app = CrawlApplication(
    definition=definition,
    state_path=ROOT / "checkpoint.json",
    profile=PROFILE,
    action_delay=2.0,
    max_attempts=2,
    max_session_restarts=1,
    max_links=MAX_LINKS,
    skip_existing=True,
)
crawler = app.create_crawler()


def configure_logging() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stderr,
    )


if __name__ == "__main__":
    configure_logging()
    print(crawler.run())
