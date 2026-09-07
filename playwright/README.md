# REL Playwright compatibility

`rel-playwright` lets Python scrapers keep the familiar Playwright control
shape while REL owns the visible Chromium session, storage, browser identity,
filtering, and proxy configuration.

```python
from rel_playwright.sync_api import sync_playwright

with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page()
    response = page.goto("https://example.com")
    print(response.status, page.title())
    print(page.locator("main a").all_inner_texts())
    browser.close()
```

The import changes from `playwright.sync_api` to `rel_playwright.sync_api`.
There is also an `async_api` module. No Playwright browser download is needed.
`chromium.launch()` uses REL's configured Default Profile (`Custom` when unset) by default; pass
`profile="Research"` to select another saved Profile.

This is a focused scraping compatibility layer, not the Playwright wire
protocol. It supports Chromium launch, REL Profile or Session selection,
contexts, pages, navigation and history, CSS locators, common native actions,
HTML/text/attribute extraction, waits, and screenshots. Arbitrary JavaScript,
CDP, route interception, downloads, frames, dialogs, tracing, browser install,
and Firefox or WebKit intentionally fail with `UnsupportedError`.

See the [public guide](../docs/PLAYWRIGHT.md) for installation, launch options,
session lifetime, supported methods, and migration details.
