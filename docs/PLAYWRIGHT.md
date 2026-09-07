# Playwright-compatible Python scraping

`rel-playwright` provides a scraping-focused subset of Playwright's Python API
while using the Chromium browser and saved configuration in REL. Change the
import and keep the familiar browser/page/locator workflow. Launching without
options uses REL's configured Default Profile (`Private` when unset); select another saved Profile or
Session only when the scraper needs it.

```python
from rel_playwright.sync_api import sync_playwright

with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page()
    response = page.goto("https://example.com")

    print(response.status)
    print(page.title())
    print(page.locator("article h2").all_inner_texts())

    page.locator("input[name=q]").fill("REL")
    page.locator("input[name=q]").press("Enter")
    page.screenshot(path="results.webp", full_page=True)
    browser.close()
```

REL remains the only Chromium and network owner. The adapter uses the supported
loopback [RPC v1](RPC.md) routes; it does not launch Playwright's Chromium,
expose the Chrome DevTools Protocol, forward arbitrary JavaScript, read proxy
credentials, or reach into REL's storage directories. Each compatibility Page
is one visible, isolated REL Session and inherits its Profile's browser
identity, persistent storage seed, proxy, AdBlock, and image-filter settings.

Related documents: [browser actions](ACTIONS.md), [macOS app](APP.md), the
[Python crawler](CRAWLER.md), and [RPC v1](RPC.md).

## Install

Requirements:

- macOS with REL installed and healthy;
- Python 3.11 or newer; and
- an existing REL Profile when the scraper needs custom identity, storage,
  proxy, or filtering configuration.

Install from the public repository:

```sh
git clone https://github.com/rel-me/rel-tools.git
cd rel-tools/playwright
python3 -m venv .venv
.venv/bin/python -m pip install -e .
```

There is no `playwright install` step and no Playwright browser download.

## Verified Playwright examples

The [`rel.me/playwright`](https://rel.me/playwright) walkthrough ports three
examples from Playwright's official Python documentation and records the small
REL-specific changes. The executable versions live in
[`playwright/examples`](https://github.com/rel-me/rel-tools/tree/main/playwright/examples):

- `playwright_quickstart.py` navigates, reads the page title, and captures a
  full-page screenshot;
- `playwright_locators.py` extracts a list through a CSS locator; and
- `playwright_actions.py` fills and submits REL's deterministic browser-action
  fixture with Enter.

Each keeps the documented Playwright browser/page/locator shape. The import
changes to `rel_playwright`, and a zero-argument `chromium.launch()` uses REL's
configured Default Profile (`Private` when unset). Pass `profile="Research"` when another saved
Profile's browser identity, storage, proxy, and filtering should be used.

Release REL uses `http://127.0.0.1:17319/v1` by default. `REL_AGENT_PORT` or
the `rel_base_url` launch option can select another supported runtime;
RELDebug normally uses port `27319`.

## Port an existing scraper

Change the import. The unchanged zero-argument launch uses the configured Default Profile (`Private` when unset):

```diff
-from playwright.sync_api import sync_playwright
+from rel_playwright.sync_api import sync_playwright

 with sync_playwright() as playwright:
     browser = playwright.chromium.launch()
     page = browser.new_page()
     page.goto("https://example.com")
```

Use the same async shape when needed:

```python
import asyncio

from rel_playwright.async_api import async_playwright

async def main() -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://example.com")
        print(await page.locator("main a").all_inner_texts())
        await browser.close()

asyncio.run(main())
```

The async adapter performs blocking loopback and file operations off the event
loop. REL still serializes work within each Session and can operate different
Sessions concurrently.

## REL launch options

`playwright.chromium.launch()` accepts these REL-specific options:

| Option | Meaning |
| --- | --- |
| `profile="Private"` | Explicit existing REL Profile copied into every new page Session. Omitting it uses the configured Default Profile, or `Private` if unset. |
| `session_id="Session12"` | Use one existing persistent Session instead of creating one. |
| `group="crawler-run"` | Group assigned to Sessions created by this Browser. A unique group is generated when omitted. |
| `persist=False` | Leave adapter-created Sessions open after close when true. Existing Sessions are never deleted. |
| `rel_base_url=...` | Override the loopback RPC v1 root. |
| `wait=0` | Extra REL settling delay in seconds after navigation and actions. |
| `slow_mo=0` | Client-side delay in milliseconds after browser operations. |
| `headless=False` | Accepted for porting; REL is always visible. `headless=True` is rejected. |

Use a saved Profile instead of translating Playwright proxy, user-agent,
locale, cookie, service-worker, viewport, or browser-argument options. That is
the important ownership boundary: REL applies one tested browser configuration
before navigation and keeps secrets in Rust and macOS Keychain.

By default, `browser.close()`, `context.close()`, or `page.close()` deletes a
Session created by the adapter, matching Playwright's temporary launch
lifetime. Set `persist=True` when the visible REL Session and its site storage
should remain available after the Python process exits. A caller-supplied
`session_id` is never deleted.

## Supported scraping surface

The sync and async modules expose the same focused classes and methods:

- `sync_playwright()` / `async_playwright()` and `Playwright.stop()`;
- `chromium.launch()`, `Browser.new_context()`, `Browser.new_page()`,
  `Browser.contexts`, and `Browser.close()`;
- `BrowserContext.new_page()`, `pages`, and `close()`;
- `Page.goto()`, `reload()`, `go_back()`, `go_forward()`, `url`, `title()`,
  `content()`, `screenshot()`, `wait_for_selector()`,
  `wait_for_load_state()`, and `wait_for_timeout()`;
- `Page.locator()` plus the page shortcuts `click`, `fill`, `type`, `press`, and
  `select_option`;
- `Locator.count()`, `all()`, `first`, `last`, `nth()`, nested CSS locators,
  `text_content()`, `inner_text()`, `inner_html()`, `all_text_contents()`,
  `all_inner_texts()` and `get_attribute()`; and
- `Locator.click()`, `fill()`, `type()`, `press()`, `select_option()`, and
  `wait_for()` through REL's canonical native browser actions.

CSS extraction uses each fresh rendered HTML capture. Singular extraction and
interaction is strict: a locator matching zero or multiple elements raises
`Error`. Indexed locators support extraction. An indexed interaction must still
resolve to one unique element because REL's native action matcher deliberately
does not add Playwright pseudo-selectors or injected-script targeting; use a
unique CSS selector instead.

Screenshots share REL's bounded image contract: at most 16,384 pixels on either
axis and 16,000,000 pixels total after display scaling. An oversized
`full_page=True` capture fails before Chromium rendering with
`RelRpcError.id == "OBSERVATION_TOO_LARGE"` instead of waiting for an unbounded
bitmap.

`wait_for_selector()` and `Locator.wait_for()` use REL's native `wait-for`
action. Both `attached` and `visible` currently mean DOM presence; REL does not
claim Playwright's computed visibility check. Navigation completes after the
main HTTP(S) frame finishes and has nonempty rendered source. `load`,
`domcontentloaded`, and `commit` are accepted compatibility labels for that
single REL readiness contract. `networkidle` is not accepted because background
page activity does not define REL navigation readiness.

## Explicit limits

This package is API-shaped compatibility for scraping, not a Playwright
protocol or CDP implementation. It raises `UnsupportedError` for features with
no supported REL equivalent, including:

- `connect`, `connect_over_cdp`, Firefox, WebKit, and headless launch;
- `evaluate`, `evaluate_all`, init scripts, bindings, and other caller-supplied
  JavaScript;
- request/response interception, direct API requests, response bodies, headers,
  and HAR recording;
- frames, popups, downloads, uploads, dialogs, permissions, geolocation,
  tracing, video, and browser installation; and
- live DOM-property queries such as `input_value()`; use `get_attribute()` only
  when the serialized HTML attribute is the intended data; and
- launch/context settings that would bypass the selected REL Profile.

Unknown keyword options also fail instead of being ignored. `TimeoutError`
represents REL `TIMEOUT` and `ACTION_TIMEOUT` failures. Other structured REL
failures are available as `RelRpcError` with `id`, `code`, `retryable`,
`details`, and `request_id` fields.

Use [`rel-crawler`](CRAWLER.md) when the job needs durable checkpoints,
bounded retries, session replacement, metadata sidecars, external skip hooks,
or a complete click-and-history crawl policy. Use `rel-playwright` when an
existing scraper mainly needs the familiar Playwright page and locator shape.

## Test

The tests use a fake loopback REL agent and never visit an external site:

```sh
cd playwright
.venv/bin/python -m unittest discover -s tests -v
```
