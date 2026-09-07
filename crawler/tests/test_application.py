from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from rel_crawler import CrawlApplication, CrawlDefinition


class CrawlApplicationTests(unittest.TestCase):
    def test_creates_independent_crawlers_with_runtime_overrides(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            app = CrawlApplication(
                definition=CrawlDefinition(start_url="https://example.com/"),
                state_path=Path(temporary) / "checkpoint.json",
                profile="Private",
                retry_failed=False,
            )

            crawler = app.create_crawler(profile="Oxylabs", retry_failed=True)

        self.assertEqual(crawler.profile, "Oxylabs")
        self.assertTrue(crawler.retry_failed)
        self.assertEqual(app.profile, "Private")
        self.assertFalse(app.retry_failed)

    def test_rejects_unknown_runtime_override(self) -> None:
        app = CrawlApplication(
            definition=CrawlDefinition(start_url="https://example.com/"),
            state_path="checkpoint.json",
        )

        with self.assertRaisesRegex(TypeError, "unknown crawler application override"):
            app.create_crawler(proxy="unsupported")


if __name__ == "__main__":
    unittest.main()
