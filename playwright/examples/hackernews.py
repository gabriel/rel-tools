"""Print the current Hacker News story titles through a REL Profile."""

from __future__ import annotations

import os

from rel_playwright.sync_api import sync_playwright


def main() -> None:
    profile = os.environ.get("REL_PROFILE")
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(profile=profile)
        page = browser.new_page()
        page.goto("https://news.ycombinator.com/")
        for title in page.locator("span.titleline > a").all_inner_texts():
            print(title)
        browser.close()


if __name__ == "__main__":
    main()
