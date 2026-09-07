from __future__ import annotations

import json
import tempfile
import unittest
from datetime import datetime
from pathlib import Path
from typing import Any
from unittest.mock import call, patch

from rel_crawler import (
    CapturedPage,
    CrawlConfigurationError,
    CrawlDefinition,
    CrawlError,
    Link,
    LinkObservation,
    NavigationOperation,
    PageOperation,
    RelCrawler,
    RelRpcError,
    RelTransportError,
    RenderedLink,
    canonicalize_url,
    extract_links,
)

SOURCE_URL = "https://example.com/new/"
ALBUM_ONE = "https://example.com/release/album/one"
ALBUM_TWO = "https://example.com/release/album/two"
ALBUM_THREE = "https://example.com/release/album/three"
ALBUM_FOUR = "https://example.com/release/album/four"
SOURCE_HTML = f"""
<html><body>
  <a href="{ALBUM_ONE}">Album one</a>
  <a href="/artist/not-an-album">Artist</a>
  <a href="{ALBUM_TWO}">Album two</a>
</body></html>
"""


def session_not_found() -> RelRpcError:
    return RelRpcError(
        error_id="SESSION_NOT_FOUND",
        code=10100,
        message="session disappeared",
        retryable=False,
        details=None,
        request_id="req_session_missing",
        http_status=404,
    )


def action_target_not_found() -> RelRpcError:
    return RelRpcError(
        error_id="ACTION_TARGET_NOT_FOUND",
        code=10204,
        message="load-more control is gone",
        retryable=False,
        details=None,
        request_id="req_action_missing",
        http_status=404,
    )


class FakeRelClient:
    def __init__(self) -> None:
        self.session_id: str | None = None
        self.current_url: str | None = None
        self.behaviors: dict[str, BaseException | int | list[BaseException | int]] = {}
        self.navigate_failures: list[BaseException] = []
        self.create_failures: dict[int, BaseException] = {}
        self.performed: list[str] = []
        self.navigated: list[str] = []
        self.back_count = 0
        self.back_url = SOURCE_URL
        self.deleted: list[str] = []
        self.profile: str | None = None
        self.created_profiles: list[str | None] = []
        self.create_count = 0
        self.action_batches: list[list[dict[str, Any]]] = []
        self.waited_for: list[str] = []
        self.result_urls: dict[str, str | list[str]] = {}
        self.source_html = SOURCE_HTML
        self.observed_links: list[RenderedLink] | None = None
        self.observation_count = 0
        self.observation_failures: list[BaseException] = []
        self.observation_truncated = False
        self.load_more_pages: list[str] = []
        self.load_more_click_count = 0
        self.load_more_position = 0
        self.load_more_base_html: str | None = None

    def health(self) -> dict[str, Any]:
        return {"version": "test"}

    def get_session(self, session_id: str) -> dict[str, Any] | None:
        if self.session_id != session_id:
            return None
        return {
            "id": session_id,
            "profile": self.profile,
        }

    def create_session(self, *, profile: str | None, group: str) -> str:
        self.create_count += 1
        failure = self.create_failures.get(self.create_count)
        if failure is not None:
            raise failure
        self.session_id = f"Session{self.create_count + 6}"
        self.profile = profile
        self.created_profiles.append(profile)
        return self.session_id

    def delete_session(self, session_id: str) -> None:
        self.deleted.append(session_id)
        self.session_id = None
        self.profile = None

    def navigate(
        self,
        *,
        url: str,
        session_id: str,
        output: Path,
        timeout: float,
        wait: float,
    ) -> PageOperation:
        self.navigated.append(url)
        if self.navigate_failures:
            raise self.navigate_failures.pop(0)
        if self.load_more_pages:
            if self.load_more_base_html is None:
                self.load_more_base_html = self.source_html
            self.source_html = self.load_more_base_html
            self.load_more_position = 0
        self.current_url = SOURCE_URL
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(self.source_html, encoding="utf-8")
        return PageOperation(
            page_id="page_source",
            session_id=session_id,
            url=SOURCE_URL,
            output_path=output,
            bytesize=len(self.source_html),
            target_http_status=200,
        )

    def perform(
        self,
        *,
        actions: list[dict[str, Any]],
        session_id: str,
        output: Path,
        timeout: float,
        wait: float,
    ) -> PageOperation:
        self.action_batches.append(actions)
        if actions[0]["action"] == "wait-for":
            self.waited_for.append(actions[0]["selector"])
            self.current_url = self.current_url or SOURCE_URL
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(self.source_html, encoding="utf-8")
            return PageOperation(
                page_id="page_source",
                session_id=session_id,
                url=self.current_url,
                output_path=output,
                bytesize=output.stat().st_size,
                target_http_status=200,
            )
        if actions[0]["action"] == "click":
            if self.load_more_position >= len(self.load_more_pages):
                raise action_target_not_found()
            self.load_more_click_count += 1
            additions = self.load_more_pages[self.load_more_position]
            self.load_more_position += 1
            self.source_html = self.source_html.replace(
                "</body>", f"{additions}</body>"
            )
            self.current_url = SOURCE_URL
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(self.source_html, encoding="utf-8")
            return PageOperation(
                page_id="page_source",
                session_id=session_id,
                url=SOURCE_URL,
                output_path=output,
                bytesize=output.stat().st_size,
                target_http_status=200,
            )
        link = actions[0]["link"]
        self.performed.append(link)
        self.waited_for.extend(
            action["selector"]
            for action in actions[1:]
            if action["action"] == "wait-for"
        )
        behavior = self.behaviors.get(link, 200)
        if isinstance(behavior, list):
            behavior = behavior.pop(0) if behavior else 200
        if isinstance(behavior, BaseException):
            raise behavior
        result_url = self.result_urls.get(link, link)
        if isinstance(result_url, list):
            result_url = result_url.pop(0) if result_url else link
        self.current_url = result_url
        output.parent.mkdir(parents=True, exist_ok=True)
        if result_url == SOURCE_URL:
            output.write_text(self.source_html, encoding="utf-8")
        else:
            output.write_text(
                "<html><head>"
                f"<title>Captured {link.rsplit('/', 1)[-1]}</title>"
                f'<link rel="canonical" href="{link}">'
                '<meta name="description" content="Captured fixture">'
                '<meta property="article:published_time" content="2025-02-03T04:05:06Z">'
                f"</head><body>{link}</body></html>",
                encoding="utf-8",
            )
        return PageOperation(
            page_id="page_source",
            session_id=session_id,
            url=result_url,
            output_path=output,
            bytesize=output.stat().st_size,
            target_http_status=behavior,
        )

    def observe_links(
        self, *, session_id: str, timeout: float, wait: float
    ) -> LinkObservation:
        self.observation_count += 1
        if self.observation_failures:
            raise self.observation_failures.pop(0)
        rendered = self.observed_links
        if rendered is None:
            rendered = [
                RenderedLink(
                    index=link.index,
                    url=link.original_url or link.url,
                    text=link.text,
                    in_viewport=True,
                    x=0,
                    y=float(link.index * 20),
                    width=100,
                    height=20,
                )
                for link in extract_links(self.source_html, SOURCE_URL)
            ]
        return LinkObservation(
            page_id="page_source",
            session_id=session_id,
            url=self.current_url or SOURCE_URL,
            observation_id=f"observation-{self.observation_count}",
            links=tuple(rendered),
            element_count=len(rendered),
            truncated=self.observation_truncated,
            omitted_node_count=1 if self.observation_truncated else 0,
            visited_node_count=20,
        )

    def back(
        self, *, session_id: str, timeout: float, wait: float
    ) -> NavigationOperation:
        self.back_count += 1
        self.current_url = self.back_url
        return NavigationOperation("page_source", session_id, self.back_url)


class RelCrawlerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.state_path = self.root / "state.json"
        self.capture_dir = self.root / "captures"
        self.client = FakeRelClient()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def definition(self, processed: list[CapturedPage]) -> CrawlDefinition:
        return CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda item: self.capture_dir
            / f"{item.index:02d}-{item.link.url.rsplit('/', 1)[-1]}.html",
            process_capture=processed.append,
        )

    def crawler(
        self, definition: CrawlDefinition, **options: Any
    ) -> RelCrawler:
        options.setdefault("action_delay", 0)
        return RelCrawler(
            definition,
            state_path=self.state_path,
            capture_dir=self.capture_dir,
            client=self.client,
            **options,
        )

    def test_clicks_captures_and_goes_back_for_each_selected_link(self) -> None:
        processed: list[CapturedPage] = []

        summary = self.crawler(self.definition(processed)).run()

        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_TWO])
        self.assertEqual(self.client.back_count, 2)
        self.assertEqual([capture.url for capture in processed], [ALBUM_ONE, ALBUM_TWO])
        self.assertTrue(all(capture.output_path.exists() for capture in processed))
        self.assertTrue(all(capture.metadata_path.exists() for capture in processed))
        self.assertEqual(processed[0].metadata_path.name, "00-one.metadata.json")
        metadata = json.loads(processed[0].metadata_path.read_text(encoding="utf-8"))
        captured_at = datetime.fromisoformat(metadata["captured_at"])
        self.assertTrue(metadata["captured_at"].endswith("Z"))
        self.assertIsNotNone(captured_at.tzinfo)
        self.assertEqual(metadata["schema_version"], 4)
        self.assertEqual(metadata["crawl"]["session_generation"], 1)
        self.assertEqual(metadata["page"]["url"], ALBUM_ONE)
        self.assertEqual(metadata["page"]["target_http_status"], 200)
        self.assertEqual(metadata["page"]["canonical_url"], ALBUM_ONE)
        self.assertEqual(
            metadata["page"]["meta"],
            [
                {"name": "description", "content": "Captured fixture"},
                {
                    "property": "article:published_time",
                    "content": "2025-02-03T04:05:06Z",
                },
            ],
        )
        self.assertEqual(
            metadata["page"]["content_timestamps"]["published"],
            [
                {
                    "value": "2025-02-03T04:05:06Z",
                    "source": "meta",
                    "field": "article:published_time",
                    "attribute": "property",
                }
            ],
        )
        self.assertFalse(metadata["backfilled"])
        self.assertEqual((summary.discovered, summary.captured, summary.failed), (2, 2, 0))

    def test_default_discovery_uses_only_rel_rendered_links(self) -> None:
        hidden = "https://example.com/release/album/hidden"
        self.client.source_html = (
            f'<a href="{hidden}" style="display:none">Hidden</a>'
            f'<a href="{ALBUM_ONE}">Album one</a>'
        )
        self.client.observed_links = [
            RenderedLink(
                index=1,
                url=ALBUM_ONE,
                text="Album one",
                in_viewport=False,
                x=10,
                y=900,
                width=120,
                height=24,
            )
        ]

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(self.definition([])).run()

        self.assertEqual((summary.discovered, summary.captured), (1, 1))
        self.assertEqual(self.client.performed, [ALBUM_ONE])
        self.assertEqual(self.client.observation_count, 1)
        self.assertIn(
            "REL rendered-link observation found 1 clickable anchors "
            "(1 currently outside the viewport)",
            "\n".join(logs.output),
        )

    def test_pending_checkpoint_link_is_skipped_when_no_longer_rendered(self) -> None:
        definition = self.definition([])
        self.crawler(definition).run()
        checkpoint = json.loads(self.state_path.read_text(encoding="utf-8"))
        stale = checkpoint["entries"][1]
        stale["status"] = "pending"
        stale["attempts"] = 0
        stale["captured_url"] = None
        stale["target_http_status"] = None
        self.state_path.write_text(json.dumps(checkpoint), encoding="utf-8")
        self.client.performed.clear()
        self.client.observed_links = [
            RenderedLink(
                index=0,
                url=ALBUM_ONE,
                text="Album one",
                in_viewport=True,
                x=10,
                y=10,
                width=120,
                height=24,
            )
        ]

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(definition).run()

        self.assertEqual(self.client.performed, [])
        self.assertEqual((summary.pending, summary.failed), (0, 1))
        updated = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(updated["entries"][1]["status"], "failed")
        self.assertIn("CrawlLinkUnavailable", updated["entries"][1]["last_error"])
        self.assertIn(
            f"skipping unavailable checkpoint link {ALBUM_TWO}",
            "\n".join(logs.output),
        )

    def test_truncated_rendered_link_observation_is_rejected(self) -> None:
        self.client.observation_truncated = True

        with self.assertRaisesRegex(CrawlError, "refusing partial discovery"):
            self.crawler(self.definition([])).run()

        self.assertEqual(self.client.performed, [])

    def test_load_more_clicks_wait_for_and_crawl_each_new_link_batch(self) -> None:
        processed: list[CapturedPage] = []
        self.client.load_more_pages = [
            f'<a href="{ALBUM_THREE}">Album three</a>',
            f'<a href="{ALBUM_FOUR}">Album four</a>',
        ]
        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda item: self.capture_dir / f"{item.index}.html",
            process_capture=processed.append,
            load_more_selector="#view-more",
            load_more_clicks=2,
        )

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(definition).run()

        self.assertEqual((summary.discovered, summary.captured), (4, 4))
        self.assertEqual(
            self.client.performed,
            [ALBUM_ONE, ALBUM_TWO, ALBUM_THREE, ALBUM_FOUR],
        )
        self.assertEqual(self.client.back_count, 4)
        self.assertEqual(self.client.load_more_click_count, 2)
        load_more_actions = [
            batch[0]
            for batch in self.client.action_batches
            if batch[0]["action"] == "click"
        ]
        self.assertEqual(
            load_more_actions,
            [
                {"action": "click", "selector": "#view-more", "scroll": True},
                {"action": "click", "selector": "#view-more", "scroll": True},
            ],
        )
        activity = "\n".join(logs.output)
        self.assertIn(
            "loading source batch 1/2: scrolling to and clicking selector '#view-more'",
            activity,
        )
        self.assertIn("load-more click 2 added 1 rendered links", activity)

    def test_load_more_stops_when_control_disappears(self) -> None:
        self.client.load_more_pages = [
            f'<a href="{ALBUM_THREE}">Album three</a>',
        ]
        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            load_more_selector="#view-more",
            load_more_clicks=3,
        )

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(definition).run()

        self.assertEqual((summary.discovered, summary.captured), (3, 3))
        self.assertEqual(self.client.load_more_click_count, 1)
        self.assertIn(
            "pagination is complete",
            "\n".join(logs.output),
        )

    def test_session_replacement_replays_completed_load_more_clicks(self) -> None:
        self.client.load_more_pages = [
            f'<a href="{ALBUM_THREE}">Album three</a>',
            f'<a href="{ALBUM_FOUR}">Album four</a>',
        ]
        self.client.behaviors[ALBUM_FOUR] = [session_not_found(), 200]
        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            load_more_selector="#view-more",
            load_more_clicks=2,
        )

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(definition).run()

        self.assertEqual((summary.captured, summary.failed), (4, 0))
        self.assertEqual(self.client.performed.count(ALBUM_FOUR), 2)
        self.assertEqual(self.client.load_more_click_count, 4)
        self.assertEqual(summary.session_generation, 2)
        self.assertIn(
            "restoring 2 completed load-more clicks",
            "\n".join(logs.output),
        )

    def test_observation_session_failure_reloads_source_in_new_session(self) -> None:
        self.client.observation_failures.append(session_not_found())

        summary = self.crawler(self.definition([]), max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(self.client.observation_count, 2)
        self.assertEqual(self.client.navigated, [SOURCE_URL, SOURCE_URL])
        self.assertEqual(self.client.deleted, ["Session7"])
        self.assertEqual(summary.session_id, "Session8")

    def test_waits_for_source_and_capture_selectors_after_navigation(self) -> None:
        processed: list[CapturedPage] = []
        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda item: self.capture_dir / f"{item.index}.html",
            process_capture=processed.append,
            source_ready_selector="#source-ready",
            capture_ready_selector="#capture-ready",
        )

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(definition, max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(
            self.client.action_batches,
            [
                [{"action": "wait-for", "selector": "#source-ready"}],
                [
                    {
                        "action": "click-link",
                        "link": ALBUM_ONE,
                        "match": {"type": "fuzzy-link", "threshold": 1.0},
                        "scroll": True,
                    },
                    {"action": "wait-for", "selector": "#capture-ready"},
                ],
                [{"action": "wait-for", "selector": "#source-ready"}],
            ],
        )
        self.assertEqual(
            self.client.waited_for,
            ["#source-ready", "#capture-ready", "#source-ready"],
        )
        activity = "\n".join(logs.output)
        self.assertIn(f"navigating to source {SOURCE_URL}", activity)
        self.assertIn("waiting for selector '#source-ready'", activity)
        self.assertIn(f"clicking {ALBUM_ONE} (attempt 1/2)", activity)
        self.assertIn("click-link auto-scroll is enabled", activity)
        self.assertIn("click will wait for target selector '#capture-ready'", activity)
        self.assertIn(f"click/capture returned url={ALBUM_ONE}", activity)
        self.assertIn("navigating Back to source", activity)
        self.assertIn("source restored through browser history", activity)

    def test_unicode_link_uses_ascii_uri_and_keeps_original_url(self) -> None:
        processed: list[CapturedPage] = []
        original_url = (
            "https://example.com/release/album/trooper-salute/友達がいました/"
        )
        uri = canonicalize_url(original_url)
        self.client.source_html = f'<a href="{original_url}">Release</a>'

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(self.definition(processed)).run()

        self.assertEqual((summary.discovered, summary.captured), (1, 1))
        self.assertEqual(self.client.performed, [uri])
        self.assertEqual(processed[0].item.link.url, uri)
        self.assertEqual(processed[0].item.link.original_url, original_url)
        metadata = json.loads(
            processed[0].metadata_path.read_text(encoding="utf-8")
        )
        self.assertEqual(metadata["link"]["url"], uri)
        self.assertEqual(metadata["link"]["original_url"], original_url)
        activity = "\n".join(logs.output)
        self.assertIn(f"clicking {original_url} (URI {uri})", activity)

    def test_unicode_checkpoint_url_is_migrated_on_resume(self) -> None:
        processed: list[CapturedPage] = []
        original_url = (
            "https://example.com/release/album/trooper-salute/友達がいました/"
        )
        uri = canonicalize_url(original_url)
        self.client.source_html = f'<a href="{original_url}">Release</a>'
        self.crawler(self.definition(processed)).run()
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        state["entries"][0]["url"] = original_url
        state["entries"][0]["key"] = original_url
        state["entries"][0].pop("original_url")
        self.state_path.write_text(json.dumps(state), encoding="utf-8")
        self.client.performed.clear()

        summary = self.crawler(self.definition(processed)).run()

        self.assertEqual((summary.discovered, summary.captured), (1, 1))
        self.assertEqual(self.client.performed, [])
        migrated = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(migrated["entries"][0]["url"], uri)
        self.assertEqual(migrated["entries"][0]["key"], uri)
        self.assertEqual(migrated["entries"][0]["original_url"], original_url)

    def test_source_page_result_is_retried_instead_of_captured(self) -> None:
        processed: list[CapturedPage] = []
        self.client.result_urls[ALBUM_ONE] = [SOURCE_URL, ALBUM_ONE]

        summary = self.crawler(self.definition(processed), max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_ONE])
        self.assertEqual([capture.url for capture in processed], [ALBUM_ONE])
        metadata = json.loads(processed[0].metadata_path.read_text(encoding="utf-8"))
        self.assertEqual(metadata["page"]["url"], ALBUM_ONE)

    def test_old_source_page_capture_is_requeued_and_overwritten(self) -> None:
        processed: list[CapturedPage] = []
        self.crawler(self.definition(processed), max_links=1).run()
        capture = processed[0]
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        state["entries"][0]["attempts"] = 2
        state["entries"][0]["captured_url"] = SOURCE_URL
        self.state_path.write_text(json.dumps(state), encoding="utf-8")
        metadata = json.loads(capture.metadata_path.read_text(encoding="utf-8"))
        metadata["page"]["url"] = SOURCE_URL
        capture.metadata_path.write_text(json.dumps(metadata), encoding="utf-8")
        capture.output_path.write_text(SOURCE_HTML, encoding="utf-8")
        processed.clear()
        self.client.performed.clear()

        summary = self.crawler(self.definition(processed), max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(self.client.performed, [ALBUM_ONE])
        self.assertEqual([capture.url for capture in processed], [ALBUM_ONE])
        repaired = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(repaired["entries"][0]["captured_url"], ALBUM_ONE)

    def test_existing_capture_is_skipped_and_backfilled_by_default(self) -> None:
        processed: list[CapturedPage] = []
        output_path = self.capture_dir / "00-one.html"
        output_path.parent.mkdir(parents=True)
        output_path.write_text(
            f'<html><link rel="canonical" href="{ALBUM_ONE}"></html>',
            encoding="utf-8",
        )

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(self.definition(processed), max_links=1).run()

        self.assertEqual((summary.captured, summary.skipped_existing), (1, 1))
        self.assertEqual(self.client.performed, [])
        self.assertEqual(self.client.back_count, 0)
        self.assertEqual(processed, [])
        self.assertIn(
            f"INFO:rel_crawler.crawler:skipping existing capture "
            f"{ALBUM_ONE} -> {output_path.resolve()}",
            logs.output,
        )
        metadata = json.loads(
            output_path.with_suffix(".metadata.json").read_text(encoding="utf-8")
        )
        self.assertTrue(metadata["backfilled"])
        self.assertTrue(metadata["checkpoint"]["skipped_existing"])
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(state["entries"][0]["status"], "captured")
        self.assertEqual(state["entries"][0]["attempts"], 0)
        self.assertTrue(state["entries"][0]["skipped_existing"])

    def test_existing_capture_can_be_overwritten_explicitly(self) -> None:
        processed: list[CapturedPage] = []
        output_path = self.capture_dir / "00-one.html"
        output_path.parent.mkdir(parents=True)
        output_path.write_text("old capture", encoding="utf-8")

        summary = self.crawler(
            self.definition(processed), max_links=1, skip_existing=False
        ).run()

        self.assertEqual((summary.captured, summary.skipped_existing), (1, 0))
        self.assertEqual(self.client.performed, [ALBUM_ONE])
        self.assertEqual(len(processed), 1)
        self.assertIn("Captured one", output_path.read_text(encoding="utf-8"))

    def test_skip_link_callback_skips_external_existing_resource(self) -> None:
        processed: list[CapturedPage] = []
        checked: list[str] = []

        def skip_link(link: Link) -> bool:
            checked.append(link.url)
            return link.url == ALBUM_ONE

        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda item: self.capture_dir
            / f"{item.index:02d}-{item.link.url.rsplit('/', 1)[-1]}.html",
            process_capture=processed.append,
            skip_link=skip_link,
        )

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(definition).run()

        self.assertEqual(checked, [ALBUM_ONE, ALBUM_TWO])
        self.assertEqual((summary.captured, summary.skipped_existing), (2, 1))
        self.assertEqual(self.client.performed, [ALBUM_TWO])
        self.assertEqual([capture.url for capture in processed], [ALBUM_TWO])
        self.assertFalse((self.capture_dir / "00-one.html").exists())
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(state["entries"][0]["status"], "captured")
        self.assertTrue(state["entries"][0]["skipped_existing"])
        self.assertEqual(state["entries"][0]["captured_url"], ALBUM_ONE)
        self.assertIn(
            f"INFO:rel_crawler.crawler:skipping link matched by skip_link "
            f"callback {ALBUM_ONE}",
            logs.output,
        )

    def test_skip_link_callback_must_return_bool(self) -> None:
        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            skip_link=lambda _link: "yes",  # type: ignore[arg-type,return-value]
        )

        with self.assertRaisesRegex(
            CrawlConfigurationError,
            "skip_link must return bool",
        ):
            self.crawler(definition).run()

    def test_unattempted_pending_entry_refreshes_its_capture_path(self) -> None:
        old_output = (self.capture_dir / "old.html").resolve()
        new_output = (self.capture_dir / "new.html").resolve()
        old_definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda _item: old_output,
        )
        new_definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda _item: new_output,
        )
        self.crawler(old_definition, max_links=1).run()
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        state["entries"][0]["status"] = "pending"
        state["entries"][0]["attempts"] = 0
        state["entries"][0]["captured_url"] = None
        self.state_path.write_text(json.dumps(state), encoding="utf-8")

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            summary = self.crawler(new_definition, max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_ONE])
        self.assertTrue(old_output.exists())
        self.assertTrue(new_output.exists())
        refreshed = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(refreshed["entries"][0]["output_path"], str(new_output))
        self.assertIn(
            f"updated pending capture path for {ALBUM_ONE}: "
            f"{old_output} -> {new_output}",
            "\n".join(logs.output),
        )

    def test_default_action_delay_paces_browser_actions(self) -> None:
        processed: list[CapturedPage] = []

        with patch("rel_crawler.sessions.time.sleep") as sleep:
            RelCrawler(
                self.definition(processed),
                state_path=self.state_path,
                capture_dir=self.capture_dir,
                client=self.client,
                max_links=1,
            ).run()

        self.assertEqual(sleep.call_args_list, [call(2.0), call(2.0), call(2.0)])

    def test_zero_action_delay_does_not_sleep(self) -> None:
        processed: list[CapturedPage] = []

        with patch("rel_crawler.sessions.time.sleep") as sleep:
            self.crawler(self.definition(processed), max_links=1).run()

        sleep.assert_not_called()

    def test_failed_link_is_checkpointed_and_skipped_on_restart(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = RuntimeError("broken page")

        with self.assertLogs("rel_crawler.crawler", level="INFO") as logs:
            first = self.crawler(self.definition(processed)).run()
        performed_after_first_run = list(self.client.performed)
        second = self.crawler(self.definition(processed)).run()

        self.assertEqual(
            performed_after_first_run, [ALBUM_ONE, ALBUM_ONE, ALBUM_TWO]
        )
        self.assertEqual(self.client.performed, performed_after_first_run)
        self.assertEqual((first.captured, first.failed), (1, 1))
        self.assertEqual((second.captured, second.failed), (1, 1))
        self.assertIn(
            f"INFO:rel_crawler.crawler:skipping failed link {ALBUM_ONE} after "
            "2 attempts: RuntimeError: broken page",
            logs.output,
        )
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual([entry["status"] for entry in state["entries"]], ["failed", "captured"])

    def test_retry_failed_requeues_with_fresh_attempts_and_overwrites_artifact(
        self,
    ) -> None:
        processed: list[CapturedPage] = []
        definition = self.definition(processed)
        self.client.behaviors[ALBUM_ONE] = 500

        first = self.crawler(definition, max_links=1).run()
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        output_path = Path(state["entries"][0]["output_path"])
        self.assertTrue(output_path.is_file())
        self.assertEqual((first.captured, first.failed), (0, 1))

        self.client.behaviors[ALBUM_ONE] = 200
        second = self.crawler(
            definition,
            max_links=1,
            retry_failed=True,
        ).run()

        self.assertEqual((second.captured, second.failed), (1, 0))
        self.assertEqual(second.skipped_existing, 0)
        self.assertEqual(self.client.performed.count(ALBUM_ONE), 3)
        self.assertEqual([capture.url for capture in processed], [ALBUM_ONE])
        retried = json.loads(self.state_path.read_text(encoding="utf-8"))["entries"][0]
        self.assertEqual(retried["status"], "captured")
        self.assertEqual(retried["attempts"], 1)
        self.assertFalse(retried["retry_requested"])

    def test_session_failure_creates_a_new_session_and_retries_the_link(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = [session_not_found(), 200]

        summary = self.crawler(self.definition(processed)).run()

        self.assertEqual((summary.captured, summary.failed), (2, 0))
        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_ONE, ALBUM_TWO])
        self.assertEqual(self.client.created_profiles, [None, None])
        self.assertEqual(self.client.deleted, ["Session7"])
        self.assertEqual(summary.session_id, "Session8")
        self.assertEqual(summary.session_generation, 2)
        self.assertEqual(summary.session_restart_count, 1)
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(state["session_generation"], 2)
        self.assertEqual(state["session_restart_count"], 1)
        self.assertEqual(state["entries"][0]["session_restarts"], 1)
        self.assertEqual(state["entries"][0]["attempts"], 2)
        self.assertEqual(state["last_session_restart"]["session_id"], "Session8")

    def test_source_session_failure_is_replaced_before_discovery(self) -> None:
        processed: list[CapturedPage] = []
        self.client.navigate_failures.append(session_not_found())

        summary = self.crawler(self.definition(processed), max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(self.client.created_profiles, [None, None])
        self.assertEqual(self.client.deleted, ["Session7"])
        self.assertEqual(self.client.navigated, [SOURCE_URL, SOURCE_URL])
        self.assertEqual(summary.session_id, "Session8")

    def test_failed_session_replacement_resumes_on_the_next_run(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = [session_not_found(), 200]
        self.client.create_failures[2] = RelTransportError("REL is restarting")

        with self.assertRaises(RelTransportError):
            self.crawler(self.definition(processed)).run()

        interrupted = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertIsNone(interrupted["session_id"])
        self.assertEqual(interrupted["entries"][0]["status"], "pending")
        self.assertEqual(interrupted["entries"][0]["session_restarts"], 1)
        self.client.create_failures.clear()

        summary = self.crawler(self.definition(processed)).run()

        self.assertEqual((summary.captured, summary.failed), (2, 0))
        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_ONE, ALBUM_TWO])
        self.assertEqual(summary.session_id, "Session9")
        self.assertEqual(summary.session_generation, 2)
        self.assertEqual(summary.session_restart_count, 1)

    def test_session_restarts_are_bounded_and_bad_link_is_skipped(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = session_not_found()

        first = self.crawler(self.definition(processed)).run()
        performed_after_first_run = list(self.client.performed)
        second = self.crawler(self.definition(processed)).run()

        self.assertEqual((first.captured, first.failed), (1, 1))
        self.assertEqual((second.captured, second.failed), (1, 1))
        self.assertEqual(performed_after_first_run, [ALBUM_ONE, ALBUM_ONE, ALBUM_TWO])
        self.assertEqual(self.client.performed, performed_after_first_run)
        self.assertEqual(
            self.client.created_profiles, [None, None, None]
        )

    def test_terminal_upstream_error_rotates_session_before_continuing(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = 503

        summary = self.crawler(self.definition(processed)).run()

        self.assertEqual((summary.captured, summary.failed), (1, 1))
        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_ONE, ALBUM_TWO])
        self.assertEqual(self.client.created_profiles, [None, None])
        self.assertEqual(self.client.deleted, ["Session7"])
        self.assertEqual(summary.session_id, "Session8")
        self.assertEqual(summary.session_restart_count, 1)

    def test_interrupted_link_consumes_its_attempt_and_resume_continues(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = KeyboardInterrupt()

        with self.assertRaises(KeyboardInterrupt):
            self.crawler(self.definition(processed)).run()

        interrupted = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(interrupted["entries"][0]["status"], "in_progress")
        self.client.behaviors.pop(ALBUM_ONE)

        summary = self.crawler(self.definition(processed)).run()

        self.assertEqual(self.client.performed, [ALBUM_ONE, ALBUM_ONE, ALBUM_TWO])
        self.assertEqual((summary.captured, summary.failed), (2, 0))

    def test_http_error_is_failed_but_can_be_accepted(self) -> None:
        processed: list[CapturedPage] = []
        self.client.behaviors[ALBUM_ONE] = 404

        summary = self.crawler(self.definition(processed), max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (0, 1))
        failed_state = json.loads(self.state_path.read_text(encoding="utf-8"))
        failed_output = Path(failed_state["entries"][0]["output_path"])
        failed_metadata = failed_output.with_suffix(".metadata.json")
        self.assertEqual(
            json.loads(failed_metadata.read_text(encoding="utf-8"))["page"][
                "target_http_status"
            ],
            404,
        )
        self.state_path.unlink()
        self.client.session_id = None
        self.client.performed.clear()
        processed.clear()

        summary = self.crawler(
            self.definition(processed),
            max_links=1,
            accept_http_errors=True,
            skip_existing=False,
        ).run()
        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(processed[0].target_http_status, 404)

    def test_unexpected_back_result_uses_direct_navigation_recovery(self) -> None:
        processed: list[CapturedPage] = []
        self.client.back_url = "https://example.com/unexpected-history-entry"

        summary = self.crawler(self.definition(processed), max_links=1).run()

        self.assertEqual((summary.captured, summary.failed), (1, 0))
        self.assertEqual(self.client.back_count, 1)
        self.assertEqual(self.client.navigated, [SOURCE_URL, SOURCE_URL])

    def test_duplicate_capture_paths_are_rejected(self) -> None:
        definition = CrawlDefinition(
            start_url=SOURCE_URL,
            select_link=lambda link: "/release/album/" in link.url,
            capture_path=lambda _item: self.capture_dir / "same.html",
        )

        with self.assertRaises(CrawlConfigurationError):
            self.crawler(definition).run()

    def test_profile_change_replaces_the_crawler_owned_session(self) -> None:
        processed: list[CapturedPage] = []
        definition = self.definition(processed)

        first = self.crawler(definition, profile="Private").run()
        second = self.crawler(definition, profile="oxylabs").run()

        self.assertNotEqual(first.session_id, second.session_id)
        self.assertEqual(self.client.created_profiles, ["Private", "oxylabs"])

    def test_unspecified_profile_reuses_managed_session_with_named_profile(self) -> None:
        processed: list[CapturedPage] = []
        definition = self.definition(processed)

        first = self.crawler(definition).run()
        self.client.profile = "Research"
        second = self.crawler(definition).run()

        self.assertEqual(first.session_id, second.session_id)
        self.assertEqual(self.client.created_profiles, [None])
        self.assertEqual(self.client.deleted, [])

    def test_existing_checkpoint_gains_session_tracking_fields(self) -> None:
        processed: list[CapturedPage] = []
        self.crawler(self.definition(processed), max_links=1).run()
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        state.pop("session_generation")
        state.pop("session_restart_count")
        state.pop("last_session_restart")
        state.pop("pending_session_restart")
        state["entries"][0].pop("session_restarts")
        state["entries"][0].pop("skipped_existing")
        self.state_path.write_text(json.dumps(state), encoding="utf-8")

        summary = self.crawler(self.definition(processed), max_links=1).run()

        upgraded = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.assertEqual(summary.session_generation, 1)
        self.assertEqual(summary.session_restart_count, 0)
        self.assertEqual(upgraded["entries"][0]["session_restarts"], 0)
        self.assertFalse(upgraded["entries"][0]["skipped_existing"])

    def test_existing_capture_gets_a_metadata_sidecar_on_restart(self) -> None:
        processed: list[CapturedPage] = []
        self.crawler(self.definition(processed), max_links=1).run()
        metadata_path = processed[0].metadata_path
        metadata_path.unlink()

        self.crawler(self.definition(processed), max_links=1).run()

        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        self.assertTrue(metadata["backfilled"])
        self.assertEqual(metadata["checkpoint"]["status"], "captured")

    def test_old_metadata_is_upgraded_without_changing_capture_time(self) -> None:
        processed: list[CapturedPage] = []
        self.crawler(self.definition(processed), max_links=1).run()
        metadata_path = processed[0].metadata_path
        old_metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        captured_at = old_metadata["captured_at"]
        old_metadata["schema_version"] = 3
        old_metadata["page"].pop("content_timestamps")
        old_metadata["link"].pop("original_url")
        metadata_path.write_text(json.dumps(old_metadata), encoding="utf-8")

        self.crawler(self.definition(processed), max_links=1).run()

        upgraded = json.loads(metadata_path.read_text(encoding="utf-8"))
        self.assertEqual(upgraded["schema_version"], 4)
        self.assertEqual(upgraded["captured_at"], captured_at)
        self.assertEqual(upgraded["link"]["original_url"], ALBUM_ONE)
        self.assertEqual(
            upgraded["page"]["content_timestamps"]["published"][0]["value"],
            "2025-02-03T04:05:06Z",
        )

    def test_legacy_sidecar_name_is_migrated_with_its_capture_time(self) -> None:
        processed: list[CapturedPage] = []
        self.crawler(self.definition(processed), max_links=1).run()
        metadata_path = processed[0].metadata_path
        old_metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        captured_at = old_metadata["captured_at"]
        old_metadata["schema_version"] = 1
        legacy_path = Path(f"{processed[0].output_path}.metadata.json")
        metadata_path.unlink()
        legacy_path.write_text(json.dumps(old_metadata), encoding="utf-8")

        self.crawler(self.definition(processed), max_links=1).run()

        migrated = json.loads(metadata_path.read_text(encoding="utf-8"))
        self.assertEqual(migrated["schema_version"], 4)
        self.assertEqual(migrated["captured_at"], captured_at)
        self.assertTrue(legacy_path.exists())


if __name__ == "__main__":
    unittest.main()
