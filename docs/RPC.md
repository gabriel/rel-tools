# REL RPC v1

REL exposes one local, versioned JSON API. This document is the supported wire
contract; unversioned routes and legacy response shapes are not supported.

Related documents: [CLI](CLI.md), [MCP](MCP.md), and [Rust SDK](SDK.md).

## Transport

- Base URL: `http://127.0.0.1:17319/v1`
- `REL_AGENT_PORT` overrides port `17319`.
- HTTP/1.1, one request per connection, `Connection: close`.
- JSON request limit: 16 MiB.
- Ordinary responses use `application/json`.
- Capture streams use `application/x-ndjson` and terminate at connection close.
- The agent is loopback-only but currently has no client authentication.

Every parsed request receives an opaque ID. Ordinary responses include it in the
`X-Request-Id` header and body. Every capture-stream line includes the same ID.

Closing a browser operation's HTTP connection before its response finishes
cancels that operation. The agent also sends cancellation through its private
Chromium bridge, so navigation, waits, and actions stop instead of continuing
in the background. Cancellation is request-scoped: the persistent browser
session, the REL app, and the resident agent remain running for other clients.

By default, a browser operation selects its target session when REL is inactive, so
the affected page is visible the next time the app is viewed. REL is not
activated or brought forward. Turn off **REL → Settings… → General → Follow
browser commands** to preserve the current selection. This presentation setting
does not change RPC results or session behavior.

## Response envelope

Every successful ordinary response is:

```json
{
  "status": "ok",
  "request_id": "req_01J...",
  "data": {}
}
```

Every failure is:

```json
{
  "status": "error",
  "request_id": "req_01J...",
  "error": {
    "id": "SESSION_NOT_FOUND",
    "code": 10100,
    "message": "Browser session Session42 was not found.",
    "retryable": false,
    "details": {
      "id": "Session42"
    }
  }
}
```

`id`, `code`, `message`, and `retryable` are required. `details` is an optional
JSON object. Numeric RPC codes begin at 10,000 and are independent of HTTP
transport statuses. Clients can branch on `code` or `id`, but must never parse
`message` or infer application meaning from the HTTP status. The same error
object is used in ordinary responses and NDJSON streams.

`retryable:true` means retrying the same idempotent operation may succeed without
user correction. It does not mean every mutation is automatically safe to
repeat.

### Standard error codes

| Code | ID | Retryable | Meaning |
| ---: | --- | --- | --- |
| `10000` | `INVALID_REQUEST` | no | Malformed HTTP or JSON |
| `10001` | `ROUTE_NOT_FOUND` | no | No v1 route matches |
| `10002` | `METHOD_NOT_ALLOWED` | no | Resource exists but method is unsupported |
| `10003` | `PAYLOAD_TOO_LARGE` | no | Request body exceeds 16 MiB |
| `10004` | `UNSUPPORTED_MEDIA_TYPE` | no | JSON endpoint received unsupported content |
| `10005` | `VALIDATION_FAILED` | no | Parsed request violates field constraints |
| `10006` | `UNSUPPORTED_MODALITY` | no | The selected adapter cannot carry the requested observation modality |
| `10007` | `OBSERVATION_TOO_LARGE` | no | A semantic snapshot or image exceeds its independent bound |
| `10100` | `SESSION_NOT_FOUND` | no | Session ID does not exist |
| `10101` | `PAGE_NOT_FOUND` | no | Ephemeral attached page does not exist |
| `10102` | `PAGE_MISMATCH` | no | Attached page state no longer matches the request |
| `10103` | `PROXY_NOT_FOUND` | no | Proxy does not exist |
| `10104` | `ACTIVE_PAGE_NOT_FOUND` | no | The shorthand workflow has no current page |
| `10200` | `CONFLICT` | no | Name/state/last-session conflict |
| `10201` | `BROWSER_BUSY` | yes | Chromium is servicing incompatible work |
| `10202` | `NETWORK_PAUSED` | no | Session networking is paused |
| `10203` | `ACTION_TARGET_NOT_FOUND` | no | Click target could not be found |
| `10204` | `REQUEST_CANCELLED` | yes | Browser work was cancelled |
| `10205` | `RATE_LIMITED` | yes | REL itself is rate limiting the caller |
| `10206` | `ACTION_TIMEOUT` | yes | A browser action's local timeout expired |
| `10207` | `OBSERVATION_STALE` | no | An observation or element reference no longer matches the live document |
| `10208` | `PRO_REQUIRED` | no | The Free plan does not include the requested resource or capability |
| `10300` | `UPSTREAM_UNAVAILABLE` | yes | Navigation received a target HTTP error or the browser/proxy received an invalid upstream result |
| `10301` | `BROWSER_UNAVAILABLE` | yes | Required Chromium service is unavailable |
| `10302` | `AGENT_UNHEALTHY` | yes | The serialized control worker missed its health deadline |
| `10303` | `TIMEOUT` | yes | REL's operation deadline expired |
| `10304` | `PROXY_CONFIGURATION_FAILED` | no | Chromium could not apply the session proxy configuration |
| `10305` | `BROWSER_CREATION_FAILED` | no | Chromium could not create the session browser |
| `10306` | `SEMANTIC_EXTRACTION_FAILED` | no | The renderer could not produce a valid bounded semantic snapshot |
| `10999` | `INTERNAL_ERROR` | no | Unexpected internal failure |

A target website returning 404 or 429 is not a REL RPC error for capture
operations. Its status is reported as `target_http_status` in capture data.
`POST /v1/navigate` instead returns `UPSTREAM_UNAVAILABLE` when its main frame
commits an HTTP 4xx or 5xx response. With the default **REL → Settings… →
General → Wait for Cloudflare Turnstile** setting, detected Turnstile and
managed Cloudflare challenge pages receive up to 15 seconds to continue before
that error is returned. This also applies to browser capture and page-creation
navigation. The error details contain the final `url` and exact
`target_http_status`; the navigated session remains selected.

### Free and Pro access

The running app selects the agent's access plan; RPC callers cannot override it.
REL Free permits one persistent Session and one custom Profile. Creating
another returns `PRO_REQUIRED`. Proxy creation, update, rotation, assignment,
and use also return `PRO_REQUIRED`; listing, reading, deleting, and detaching
previously stored resources remain available.

`PRO_REQUIRED` uses HTTP 403 and includes stable `details.feature` and
`details.plan` strings. The currently returned feature values are `proxies`,
`additional_sessions`, and `additional_custom_profiles`. Register REL Pro in
**REL → Settings… → Plan** to remove these limits.

## Routes

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/v1/health` | Readiness of the agent control worker |
| `GET` | `/v1/status` | App, agent, proxy, and Chromium diagnostic report |
| `GET` | `/v1/notifications` | List opt-in website notifications as untrusted content |
| `POST` | `/v1/navigate` | Navigate and select the current shorthand page |
| `POST` | `/v1/navigate/observe` | Navigate and return a synchronized page observation |
| `POST` | `/v1/perform` | Perform actions on the current shorthand page |
| `POST` | `/v1/capture` | Capture the current shorthand page |
| `POST` | `/v1/screenshot` | Capture an image of the current shorthand page |
| `POST` | `/v1/observe` | Observe the current shorthand page |
| `POST` | `/v1/captures` | Capture rendered HTML as an NDJSON operation |
| `POST` | `/v1/pages` | Attach an ephemeral automation page |
| `POST` | `/v1/pages/{page_id}/actions` | Perform one action on an attached page |
| `POST` | `/v1/pages/{page_id}/screenshot` | Capture an image of an attached page |
| `POST` | `/v1/pages/{page_id}/observe` | Observe an attached page |
| `POST` | `/v1/observations/{observation_id}/actions` | Perform ordered observation-scoped actions |
| `POST` | `/v1/observations/{observation_id}/find` | Search stored public observation semantics |
| `GET` | `/v1/observations/{observation_id}` | Read one retained public semantic snapshot |
| `GET` | `/v1/proxies` | List proxies |
| `POST` | `/v1/proxies` | Create a proxy |
| `GET` | `/v1/proxies/{alias}` | Read one proxy |
| `PATCH` | `/v1/proxies/{alias}` | Partially update a proxy |
| `DELETE` | `/v1/proxies/{alias}` | Delete and detach a proxy |
| `POST` | `/v1/proxies/{alias}/rotate-session` | Rotate managed proxy sessions |
| `POST` | `/v1/proxy-transfers/export` | Export a versioned proxy transfer |
| `POST` | `/v1/proxy-transfers/import` | Import a versioned proxy transfer |
| `GET` | `/v1/sessions` | List persistent browser sessions |
| `POST` | `/v1/sessions` | Create a browser session |
| `POST` | `/v1/sessions/close` | Close every browser session in a group |
| `GET` | `/v1/profiles` | List saved session configurations |
| `POST` | `/v1/profiles` | Create a custom session profile |
| `PATCH` | `/v1/profiles/{id}` | Update custom-profile browser-data availability |
| `DELETE` | `/v1/profiles/{id}` | Delete a custom session profile |
| `POST` | `/v1/profile-transfers/export` | Export a versioned profile transfer |
| `POST` | `/v1/profile-transfers/import` | Import a versioned profile transfer |
| `GET` | `/v1/sessions/{id}` | Read one browser session |
| `PATCH` | `/v1/sessions/{id}` | Partially update a browser session |
| `POST` | `/v1/sessions/{id}/pause` | Pause session network activity |
| `POST` | `/v1/sessions/{id}/play` | Resume session network activity |
| `DELETE` | `/v1/sessions/{id}` | Delete a browser session |

There are deliberately no log read, clear, or ingestion routes.

The [`rel-client`](SDK.md) Rust crate exposes one typed method for every route
in this table. The bundled CLI is built on that crate and uses resource commands
such as `rel capture`, `rel page`, `rel proxy`, and `rel session`; it has no
direct database or log-file command path.

The bundled `rel-mcp` adapter also calls this API only through `rel-client`. It
maps fourteen MCP tools to status, opt-in website notifications, bounded
semantic reading, HTML and image
capture, page attachment and actions, observations, session-group closing, and
session and proxy listing.
MCP does not add an HTTP `/mcp` route or another response shape to RPC v1. See
[MCP](MCP.md) for its stdio lifecycle and result wrapping.
`rel_read` is a `rel-client` composition over `POST /v1/navigate/observe` and
`POST /v1/observe`. The SDK's `read_observation` helper applies the same bounded
selection to `GET /v1/observations/{observation_id}`; neither helper adds an
alternate browser transport.

## Health

### `GET /v1/health`

HTTP 200 while the worker is ready or operating within its deadline:

```json
{
  "status": "ok",
  "request_id": "req_...",
  "data": {
    "version": "0.1.8",
    "pid": 123,
    "browser_proxy_port": 17400,
    "build": {
      "id": "ba49-deadbeef-a1b2c3d4",
      "configuration": "Debug",
      "worktree": "ba49",
      "branch": "codex/example",
      "commit": "deadbeef",
      "dirty": true
    },
    "worker": { "state": "idle" },
    "database_recovery": null
  }
}
```

`build` identifies the installed worktree build and is `null` for agents that
were not launched from a metadata-bearing app bundle. Worker state is
`starting`, `idle`, or `busy`. A startup/operation deadline
violation or failed worker returns `AGENT_UNHEALTHY`, with the worker
snapshot in `error.details.worker`. Health deadlines diagnose stalls; they do not
cancel the active request.

`database_recovery` is `null` when no committed database upgrade/recovery report
exists. Otherwise it contains `schema_version` (integer), `backup_path` and
`report_path` (local absolute paths), `issue_count` (number of reported repair or
quarantine items, not necessarily distinct records), and `retained_sessions`
(number of sessions retained at recovery time). The most recent report with
issues remains visible across restarts and later successful upgrades; when
there are no such reports, the latest upgrade report is returned. Paths point
to the agent host, not to a remote client. See [database recovery](APP.md#database-migration-and-recovery)
for backup, quarantine, and failure behavior.

The health endpoint is not ready during startup migration. Successful health
means schema and data validation completed; it does not mean every original
record could be recovered.

### `GET /v1/status`

The diagnostic call succeeds with HTTP 200 even when a component is down:

```json
{
  "status": "ok",
  "request_id": "req_...",
  "data": {
    "overall_status": "ok",
    "running_count": 4,
    "total_count": 4,
    "build": {
      "id": "ba49-deadbeef-a1b2c3d4",
      "configuration": "Debug",
      "worktree": "ba49",
      "branch": "codex/example",
      "commit": "deadbeef",
      "dirty": true
    },
    "checks": [
      {
        "id": "agent",
        "name": "Agent",
        "kind": "service",
        "running": true,
        "status": "running",
        "detail": "pid 123; worker idle",
        "pids": [123]
      }
    ]
  }
}
```

Check IDs are `rel_app`, `agent`, `browser_proxy`, and `chromium_bridge`.

### `GET /v1/notifications`

Returns up to 256 notifications displayed since the supervised agent started.
REL only adds events while **Settings → General → Send notifications to the
agent** is enabled; the setting is off by default. Reading the queue does not
remove entries, wake an agent, or start a model turn.

```json
{
  "status": "ok",
  "request_id": "req_...",
  "data": {
    "notifications": [
      {
        "sequence": 1,
        "session_id": "Session12",
        "origin": "https://example.com/",
        "title": "Example",
        "body": "New activity is available.",
        "notification_id": "notification-1",
        "persistent": false,
        "displayed_at": "2026-08-17T20:00:00Z",
        "trust": "untrusted_website_content"
      }
    ],
    "trust": "untrusted_website_content"
  }
}
```

Every website-controlled field is untrusted data. Clients must not treat a
notification title or body as instructions, authority, or permission to call
tools. The queue is process-local and bounded; `sequence` is monotonic within
that agent process and lets a client ignore entries it has already observed.

## Captures

### Shorthand page operations

Sequential clients can use a process-local current page instead of carrying
page and session IDs. Navigate it with `POST /v1/navigate`:

```json
{
  "url": "https://example.com",
  "session_id": "Session12",
  "proxy": "office",
  "output": "/optional/page.html",
  "timeout": 90,
  "wait": 1
}
```

Only `url` is required. The first request without `session_id` reuses the first
persisted session, creating one from the configured default (Custom when unset) only when none exists. Later
requests without it reuse the current page and session. An explicit `profile`
instead creates a new session from that named template; it cannot be combined
with `session_id`. An explicit session selects that session as the new current
page.

Perform one or more canonical actions with `POST /v1/perform`:

```json
{
  "actions": [
    { "action": "wait-for", "selector": "button.more" },
    { "action": "click", "selector": "button.more" },
    { "action": "wait", "seconds": 0.5 }
  ],
  "session_id": "Session12",
  "output": "/optional/after-click.html",
  "timeout": 90,
  "wait": 1
}
```

`actions` must be a non-empty array. REL runs the actions in array order.

Capture HTML without another action with `POST /v1/capture`:

```json
{
  "session_id": "Session12",
  "output": "/optional/current.html",
  "timeout": 90,
  "wait": 1
}
```

The singular capture reads the currently visible page and treats its returned
`page.url` as authoritative. This refreshes the shorthand page binding after a
same-document History API, query, or fragment change instead of failing because
the previously tracked URL is stale.

All three return the same page-operation envelope documented under attached
pages. When `session_id` is supplied, `navigate` selects and updates that
session's current shorthand page; `perform` and singular `capture` target it.
Navigation becomes ready after the requested HTTP(S) main frame starts,
finishes, and has nonempty rendered source. Subframe and page-initiated
background loading does not delay completion. The `wait` delay begins after
main-frame readiness and restarts if another main-frame navigation begins. Use
a timed `wait` action when a workflow needs additional settling time.
If navigation commits an HTTP 4xx or 5xx main-frame response, it returns
`UPSTREAM_UNAVAILABLE` instead of waiting for unrelated background loading to
become idle. A detected Cloudflare Turnstile or managed challenge receives the
default-on 15-second continuation window described above. The error includes
the exact `target_http_status`, and the page remains the session's current
shorthand page.
Without `session_id`, they use the most recently navigated shorthand page for
compatibility. `perform` and singular `capture` return `ACTIVE_PAGE_NOT_FOUND`
with `ACTIVE_PAGE_NOT_FOUND` until a matching page has been selected by
navigation. This registry is process-local and is cleared by an agent restart
or when its session closes. Concurrent work within one session should use
explicit page IDs.

### Screenshots

Capture the visible viewport of the current shorthand page with
`POST /v1/screenshot`, or use `POST /v1/pages/{page_id}/screenshot` for an
explicit attached page:

```json
{
  "session_id": "Session12",
  "output": "/optional/page.webp",
  "format": "webp",
  "quality": 80,
  "full_page": true,
  "timeout": 90,
  "wait": 0
}
```

`session_id` is accepted only by the current-page route. `format` is `png`
(default), `jpeg`, or `webp`. `quality` is an integer from 0 through 100 and is
ignored for PNG. `full_page` defaults to false; true captures content beyond
the visible viewport. `output` follows the same absolute-response-path contract
as HTML capture. When omitted, REL writes under its temporary `screenshots`
directory. Every encoded image is limited to 16,384 pixels on either axis and
16,000,000 pixels total. REL checks full-page document dimensions before asking
Chromium to render the image and returns `OBSERVATION_TOO_LARGE` immediately
when the scaled page exceeds either bound.

Success uses the ordinary RPC envelope:

```json
{
  "page": {
    "id": "page_...",
    "session_id": "Session12",
    "url": "https://example.com/"
  },
  "screenshot": {
    "output_path": "/private/tmp/rel/screenshots/example.webp",
    "bytesize": 48231,
    "format": "webp",
    "mime_type": "image/webp",
    "width": 1280,
    "height": 2400
  }
}
```

The image bytes remain in the file rather than the JSON response. The MCP
adapter reads that validated file and emits standard image content when its
caller did not request a specific output URI.

### Page observations, semantic find, and reference actions

`POST /v1/navigate/observe` combines navigation and the first observation:

```json
{"url":"https://example.com","session_id":"Session12","mode":"hybrid","timeout":90,"wait":0}
```

It accepts the navigation fields `url`, `session_id`, `profile`, and `proxy`
plus the observation fields `mode`, `timeout`, and `wait`. It reuses the same
active-page and persistent-session rules as `POST /v1/navigate`, but returns an
observation instead of creating an HTML capture artifact.

Set `navigation` to `back`, `forward`, or `reload` to operate on the active
page's history and omit `url`, `profile`, and `proxy`. `navigation` defaults to
`url`, where `url` is required. An optional `session_id` scopes history
navigation to that session's active page.

`POST /v1/observe` observes the current shorthand page. The attached-page form
is `POST /v1/pages/{page_id}/observe`:

```json
{"session_id":"Session12","mode":"hybrid","timeout":90,"wait":0}
```

`session_id` is accepted only by the current-page route. `mode` is `semantic`
(the default), `hybrid`, or `visual`; `auto` is not an RPC mode. Semantic mode
returns compact rendered text and typed interactive elements. Hybrid also
returns a synchronized current-viewport PNG. Visual returns minimal semantics
plus that PNG. Screenshot bytes are kept in a typed temporary file resource,
with its dimensions and exact CSS-to-image scales in the response.

Each observation contains an ID, document sequence, capture time, title,
truncation counts, viewport/document geometry, semantic `content`, and typed
`elements`. Content and elements may include a bounded `context` path such as
`main > form: Checkout > table: Items > tr: Product A`; this preserves useful
landmark, form, dialog, list, table, and row relationships without exposing a
selector or durable DOM identity. `omitted_node_count` reports entries dropped by traversal or output
bounds, while `clipped_text_count` reports individual text fields shortened to
their field limit; `truncated` is true when either occurred. Tables preserve DOM
order with `table`, `table_row`, `table_caption`, and `table_cell` content kinds,
including repeated cell values. Elements hidden by rendered CSS visibility are
excluded along with `hidden` and `aria-hidden` subtrees. Element refs such as
`e17` are valid only for that page, document sequence, and observation. Private
locators never cross RPC.

Semantic observations visit at most 50,000 DOM nodes, retain at most 5,000
candidates and returned entries, limit individual text fields to 2,048 bytes,
and limit total returned semantics to 512 KiB. REL reports rather than silently
hiding any truncation caused by these independent bounds.

Act through a ref with
`POST /v1/observations/{observation_id}/actions`:

```json
{
  "actions": [
    {"ref":"e17","action":"hover","scroll":true},
    {"ref":"e18","action":"click","mouse_move":true,"scroll":true},
    {"action":"wait","seconds":0.25},
    {"action":"scroll","delta_x":0,"delta_y":-600}
  ],
  "mode":"hybrid"
}
```

`actions` contains 1–32 ordered items. Element actions are `click`, `type`
(requires `text`), `clear`, `press` (requires `key`), `select` (requires
`value`), and `hover`; each requires a ref. Page actions are `scroll`, with
integer `delta_x`/`delta_y` from -10000 through 10000 and at least one non-zero
delta. These are native wheel deltas: negative `delta_y` scrolls toward the
page bottom, positive `delta_y` scrolls toward the top, negative `delta_x`
scrolls right, and positive `delta_x` scrolls left. `wait` takes `seconds` from
0 through 60. REL stops at the first
failure and returns only one new post-action observation after the whole batch.
It revalidates the document sequence and every target signature before input. A
mismatch returns `OBSERVATION_STALE`; no selector or nearby-target fallback is
attempted.

Search the stored public snapshot without another browser read using
`POST /v1/observations/{observation_id}/find`:

```json
{"query":"continue","role":"button","limit":20}
```

At least `query` or `role` is required. Query matching is case-insensitive over
content text and public element role, name, value, and destination. Role is an
exact case-insensitive element filter. `limit` defaults to 20 and may be 1–100.
Results distinguish `content` and `element` matches, preserve actionable refs,
and report `total_matches` plus `truncated`. Private locators are never stored in
or returned from the searchable public snapshot.

Read the complete retained public snapshot using
`GET /v1/observations/{observation_id}`. It returns the ordinary page and
observation envelope. After the page navigates, REL erases the observation's
private locators and rejects actions with `OBSERVATION_STALE`, but its public
semantic content remains readable. The process-local registry retains at most
32 observations and removes them when their session closes or the agent exits.

### `POST /v1/captures`

```json
{
  "url": "https://example.com",
  "output": "/optional/page.html",
  "timeout": 90,
  "wait": 1,
  "actions": [],
  "session_id": "Session12",
  "proxy": "office",
  "retry": 1,
  "retry_delay": 3
}
```

| Field | Contract |
| --- | --- |
| `url` | Required HTTP(S) URL; scheme-less input is normalized by the agent. |
| `output` | Optional nonempty filesystem path or null; generated when absent. Relative input is resolved against the agent process directory. Responses always contain an absolute `output_path`. |
| `timeout` | Finite seconds greater than zero; default 90. |
| `wait` | Finite settling seconds after final main-frame readiness; default 1. Background loading does not restart it. |
| `actions` | Optional array of canonical [action objects](ACTIONS.md). |
| `session_id` | Optional existing canonical `Session<number>` ID. Omission creates a persistent session and returns its ID in capture events. |
| `profile` | Optional saved profile name for the newly created session. It cannot be combined with `session_id`; omission uses the configured default or Custom. |
| `group` | Optional 1–128 character group for the newly created session. It cannot be combined with `session_id`; matching and bulk close are case-insensitive. |
| `proxy` | Optional unique proxy alias string, assigned to the created session or applied to the existing session. |
| `retry` | Integer 0 through 100; default 1. |
| `retry_delay` | Finite seconds 0 through 86400; default 3. |

The RPC accepts the same closed JSON objects as the CLI and MCP server. The
[Actions reference](ACTIONS.md) defines every action, selector constraint,
default, and failure behavior. Browser sessions controlled while not visible
use the global **Background Browser Size** preset; RPC has no per-request
viewport override.

Preflight failures use the ordinary error response. Once accepted, REL returns
HTTP 200 `application/x-ndjson`. Each physical line is one complete object; there
is no encoded stdout/stderr layer:

```json
{
  "status": "ok",
  "request_id": "req_...",
  "event": "capture.started",
  "data": {
    "url": "https://example.com/",
    "session_id": "Session12"
  }
}
```

Events, in normal order:

1. `capture.started`
2. `capture.browser_requested`
3. `capture.page_ready`
4. `capture.rendered`
5. `capture.writing`
6. `capture.retrying` when applicable
7. `capture.traffic`
8. `capture.completed` or `capture.failed`
9. `capture.finished`, containing `exit_code`

`capture.failed` uses the standard nested error object. `capture.completed`
contains an absolute output path, bytes, final URL, optional
`target_http_status`, session ID, capture ID, and proxy traffic. A target status
at least 400 is a completed capture with `outcome:"target_error"` and CLI exit
code 1; it is not an API error.

## Attached pages

### `POST /v1/pages`

```json
{
  "url": "https://example.com",
  "session_id": "Session12",
  "proxy": "office",
  "output": "/optional/page.html",
  "timeout": 90,
  "wait": 1
}
```

Omitting `session_id` creates a session from the named `profile`, or from
the configured default (Custom when unset) when it is absent, and navigates it to `url`. `profile` and `group`
cannot be combined with `session_id`. Providing an
existing session attaches its current page without navigating; its final
normalized browser URL must equal the requested URL. Success data:

```json
{
  "page": {
    "id": "page_...",
    "session_id": "Session12",
    "url": "https://example.com/"
  },
  "capture": {
    "output_path": "/private/tmp/rel/captures/...html",
    "bytesize": 1234,
    "target_http_status": 200
  }
}
```

Page IDs are process-local and disappear when the agent restarts.

### `POST /v1/pages/{page_id}/actions`

```json
{
  "action": { "action": "click", "selector": "button.more" },
  "output": "/optional/page.html",
  "timeout": 90,
  "wait": 1
}
```

The response uses the same page/capture data. URL, proxy, and session come from
the attached page and cannot be overridden.

## Proxies

A proxy resource is:

```json
{
  "alias": "office",
  "upstream_host": "proxy.example.com",
  "upstream_port": 8000,
  "username": "optional",
  "password_set": true,
  "oxylabs": {
    "enabled": false,
    "location_parameter": null,
    "location_value": null
  }
}
```
If no Oxylabs configuration exists for a proxy, `oxylabs` is omitted.

Passwords are accepted on writes but never returned.

- `GET /v1/proxies` returns `data.proxies`, ordered by creation order.
- `GET /v1/proxies/{alias}` returns `data.proxy`.
- `POST /v1/proxies` requires `alias`, `upstream_host`, and `upstream_port`. Optional
  write fields are `username`, `password`, `oxylabs_enabled`,
  `oxylabs_location_parameter`, `oxylabs_location_value`, `bright_data`, and `locale`.
- `PATCH /v1/proxies/{alias}` is a true partial update. Missing fields are retained.
  `username:null`, `password:null`, or `locale:null` clears that value.
- `DELETE /v1/proxies/{alias}` detaches it from all sessions, then returns
  `data.deleted_alias`.
- `POST /v1/proxies/{alias}/rotate-session` requires an Oxylabs- or Bright Data-enabled proxy and
  returns `data.proxy`.

`locale` is an optional BCP-47 language/locale associated with the proxy, such as
`fr-CA`. Whitespace is trimmed and underscores normalize to hyphens. Null or an
empty string clears the preference; omission on update preserves it. Invalid
values are rejected before mutation. Countries and provider location selectors
never infer this value. Proxy responses include `locale`, and assigned session
responses include `proxy_locale`. Proxy/Profile transfers preserve this setting
in transfer version 5; older transfers have no proxy locale.

Aliases are case-insensitively unique, immutable, and must start with a letter;
they may contain only letters, numbers, hyphens, and underscores (maximum 64
characters), and cannot be a UUID. An alias is the sole public proxy identifier: numeric database IDs
and UUIDs are neither accepted nor returned by proxy APIs.
Oxylabs location requires both parameter and value; parameter is `cc`, `country`,
or `st`. REL generates a distinct persistent sticky ID for each browser session.
The dedicated rotate-session operation replaces the IDs for all sessions assigned
to that proxy. IDs are not part of the public proxy resource.

The app's New Proxy form suggests the first available alias for the selected
provider, such as `bright-data-1` or `oxylabs-residential-1`. Custom proxies use
`proxy-1`. Existing aliases are checked without regard to case. Changing the
provider or pasting a curl command updates the suggestion unless you have edited
the alias yourself. Existing proxy aliases remain immutable.

### Paste proxy settings from curl

In the app's proxy editor, choose **Paste curl command** to fill the proxy fields
from the clipboard. The importer accepts `--proxy` / `-x` and `--proxy-user` /
`-U`, including quoted values, `--option=value`, attached short options, and
credentials embedded in an HTTP proxy URL. The endpoint must include a port.

REL recognizes provider presets and extracts Bright Data or Oxylabs location
suffixes into their controls. Pasted session suffixes are replaced by REL's
per-browser-session IDs. Bright Data's `[replace with password]` placeholder is
left blank; enter the real password before saving. When importing into an existing
proxy, enter the imported proxy's password rather than reusing its saved password.
The alias remains unchanged. Review the form and save to apply the settings.

The command is parsed as text and is never executed. The destination URL and
other curl options are ignored, including `-k`; REL's certificate validation
settings still apply.

### Bright Data location and sessions

Select Bright Data in the app's proxy editor to configure a country and optional
state, city, or ASN. REL shows the resulting masked connection string and gives
each browser session its own persistent `-session-<id>` suffix.

The proxy create and update routes accept a `bright_data` object:

```json
{
  "alias": "bright",
  "upstream_host": "brd.superproxy.io",
  "upstream_port": 44445,
  "username": "brd-customer-CUSTOMER-zone-ZONE",
  "password": "ZONE_PASSWORD",
  "tls": {"mode": "bright_data"},
  "bright_data": {
    "enabled": true,
    "country": "us",
    "state": "ny",
    "city": "newyork",
    "asn": "56386"
  }
}
```

Use the base username without location or session suffixes. REL appends country,
state, city, ASN, then its session ID. Country uses a two-letter code. State and
city require a country; city names have no spaces. State and city accept only
ASCII letters and digits, and ASN accepts digits. Values are trimmed and
lowercased. Targeting availability depends on the Bright Data zone and network;
see [Bright Data's configuration guide](https://docs.brightdata.com/proxy-networks/config-options).

Omitting `bright_data` on update preserves its configuration. Supplying the object
replaces it; omitted targeting fields clear those selections. Set `enabled:false`
to disable managed suffixes. Bright Data and Oxylabs cannot both be enabled.
Responses include `bright_data`; passwords and generated sticky IDs are excluded.

IDs survive reopening a session and editing its proxy location. Assigning a
different proxy, re-enabling managed sessions, or calling `rotate-session`
generates new IDs. A sticky ID requests the same peer; Bright Data may replace an
unavailable peer. Location changes can also change the peer.

Version 4 proxy and profile transfer archives preserve Bright Data configuration.
Older versions remain importable with managed Bright Data settings disabled.

`POST /v1/proxy-transfers/export` accepts `alias`, `include_credentials`, and
an optional `passphrase`. It returns `data.filename` and `data.contents_base64`,
where `contents_base64` decodes to the complete versioned `.relproxy` SQLite
archive. Ordinary clients must set `include_credentials:false`. Including
app-protected credentials requires a nonempty passphrase and the private
authorization header held by the owning REL app.

`POST /v1/proxy-transfers/import` accepts `contents_base64`, an optional `alias`
override, and the passphrase when credentials are included. It validates and
decrypts the transfer in Rust and returns the newly created `data.proxy`.
Transfers are limited to 12 MiB and never overwrite an existing alias.

## Sessions

A session resource is:

```json
{
  "id": "Session12",
  "name": "Session12",
  "profile": "BandwidthSaver",
  "profile_data_id": null,
  "group": "pgm",
  "proxy_alias": null,
  "adblock_enabled": true,
  "image_blocking_mode": "over_limit",
  "image_size_limit_kb": 100,
  "created_at": 1785860000
}
```

- `GET /v1/sessions` returns `data.sessions`.
- `GET /v1/sessions/{id}` returns `data.session`.
- `POST /v1/sessions` accepts optional `name`, `group`, `profile`, `proxy_alias`,
  `adblock_enabled`, `image_blocking_mode`, and `image_size_limit_kb`; returns
  `data.session`.
- `PATCH /v1/sessions/{id}` is partial and returns `data.session`.
- `POST /v1/sessions/{id}/pause` and `/play` take no body and idempotently
  return `data.session_id` and `data.network_paused`. Pause cancels active
  requests and blocks new network work. Play resumes network activity and
  reloads the current page when the pause interrupted or deferred navigation.
  If pause cancels navigation before the new document commits, REL restores the
  previous URL and live document; play then resumes without reloading that page.
- `DELETE /v1/sessions/{id}` returns the canonical session ID as
  `data.deleted_id`.
- `POST /v1/sessions/close` accepts `{"group":"pgm"}` and returns the trimmed
  group plus every closed canonical ID as `data.deleted_ids`. Group matching is
  case-insensitive, and an empty group is an idempotent success.

`image_blocking_mode` is `none`, `all`, or `over_limit`. `none` allows every
image while leaving `adblock_enabled` independent. The legacy `block_images`
alias is rejected. Size is 1 through 1,048,576 kB. The visible name is editable and
case-insensitively unique; the canonical `id` is immutable. Session routes accept
only that ID; numeric database IDs are neither accepted nor returned. A group
is immutable, contains 1–128 non-control characters after trimming, and may be
shared by any number of sessions.

## Profiles

Profiles are user-created session configurations. There are no generated
built-ins, and the list is empty until a Profile is saved. A profile resource is:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "BandwidthSaver",
  "proxy_alias": null,
  "adblock_enabled": true,
  "image_blocking_mode": "over_limit",
  "image_size_limit_kb": 10,
  "includes_cookies": false,
  "includes_passwords": false,
  "fingerprint_profile": {
    "schema_version": 1,
    "seed": "12345",
    "platform": "macos",
    "browser_brand": "chromium",
    "browser_version": "151.0.7922.76",
    "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.7922.76 Safari/537.36",
    "locale": "en-US",
    "timezone": "America/Los_Angeles",
    "network_profile": "desktop",
    "hardware_concurrency": 8,
    "device_memory_gib": 8,
    "max_touch_points": 0,
    "screen": {
      "width": 1920,
      "height": 1080,
      "available_height": 985,
      "device_scale_factor": 2
    },
    "graphics_profile": "apple-m2",
    "storage_quota_bytes": 107374182400,
    "canvas_noise_mode": "deterministic",
    "audio_noise_mode": "deterministic"
  },
  "is_builtin": false,
  "created_at": 0
}
```

- `GET /v1/profiles` returns saved profiles sorted by name in `data.profiles`.
- `POST /v1/profiles` requires a case-insensitively unique `name`; it accepts
  the proxy, filtering, browser-data inclusion, and `fingerprint_profile`
  fields above and returns `data.profile`. Omitting `fingerprint_profile` uses
  the compatibility template. Set it to `null` for native Chromium identity.
- `PATCH /v1/profiles/{id}` accepts any editable profile setting and returns
  the updated custom profile in `data.profile`. REL.app uses the browser-data
  flags only after it has safely staged imported browser data; cookie and
  password values never cross RPC.
- `DELETE /v1/profiles/{id}` deletes a custom profile and returns
  `data.deleted_id`. Built-in IDs are not stored and cannot be deleted.

`POST /v1/profile-transfers/export` accepts `name`, `include_cookies`,
`include_passwords`, `include_proxy_credentials`, and an optional `passphrase`.
It returns `data.filename` plus `data.contents_base64`, which decodes to the
versioned `.relprofile` SQLite archive. A passphrase and the owning app's
private authorization header are required when any secret category is
selected. The Rust response contains the schema and Profile/Proxy records; the
app adds encrypted browser records before presenting the file to the user.

`POST /v1/profile-transfers/import` accepts `contents_base64`, optional `name`
and `passphrase` fields, and app-only `browser_data_ready`. It validates the
archive in Rust, decrypts embedded Proxy credentials, and returns the newly
created `data.profile`. The owning app sets `browser_data_ready:true` only
after it has authenticated and decrypted the browser records for restoration.
Transfer files are limited to 12 MiB and never overwrite an existing name.

Both transfer types use SQLite `application_id` `RELT`, schema version 1, and
the same five tables: `metadata`, `proxies`, `profiles`, `cookies`, and
`passwords`. The `proxies` table is identical for standalone Proxy archives
and Proxies embedded in Profile archives. Protected credential and browser
payloads are encrypted BLOBs. Version 1 accepts no legacy JSON representation.

Existing Sessions retain their stored settings and browser data after built-in
Profiles are removed. The old built-in names are no longer reserved: they can
be used for new saved configurations. Historical schema migrations retain their
original data-preserving rename behavior.

Profile names contain 1–128 non-control characters after trimming and are the
selector used during session creation. On `POST /v1/sessions`, omission selects
the **Default Profile** configured in **Settings → General**, or direct **Custom** settings
when no preference is set (AdBlock on, all images allowed, Private). This applies to clients that omit the field. Create Session in the app
starts with Custom and sends explicit settings. `profile:null` explicitly selects Custom without inheriting any saved profile
or browser data. An explicit saved profile always takes precedence. The preference
uses the stable profile ID, so renaming a custom profile preserves its selection.
If that profile is deleted, creation without an explicit profile reports an error
until another default is selected. Explicit session settings override the selected profile. A present
`proxy_alias:null` is a direct override; a non-null value must reference an
existing proxy. Automatically created sessions for capture, navigation, and
attached pages follow the same rule. Capture events and page responses include
the effective session ID. Browser-data payloads remain app-owned and never
cross RPC in plaintext; the inclusion flags describe what the app has attached to a custom
profile. Importing a selected category again replaces that category in the
app-owned template without changing sessions already created from it.

The fingerprint object is an identity template. Every session creation path
copies the selected profile through the agent. It preserves the template settings and generates a fresh
seed before the session's Chromium context is used. New profiles and Custom
sessions use Private when `fingerprint_profile` is omitted; explicit null
keeps native values. `POST /v1/sessions` also accepts `fingerprint_profile` as a
direct override of the selected configuration.


Fingerprint profiles also accept an optional `overrides` array. Omit the field
to enable all supported overrides, or provide an explicit list to leave
all unlisted controls native. An empty array applies no overrides. All existing
profile value fields remain required and validated; inactive values are stored
for later editing, but are not sent as browser overrides.

| Override | Applied behavior |
| --- | --- |
| `locale` | Language preferences and locale, applied only when resolved values differ from native |
| `timezone` | IANA timezone |
| `hardware_concurrency` | Page CPU thread count; workers retain native values in this engine |
| `network` | Network information profile |
| `device_surfaces` | Linked memory, touch, screen/pixel ratio, PDF plugin fallback, and storage quota controls |
| `graphics` | Linked Canvas/WebGL readbacks, WebGL identity/extensions, and unavailable WebGPU adapters |
| `audio` | Audio readbacks using the profile's audio mode |

`locale_mode` accepts `automatic` or `custom`. Private defaults to
`automatic`. Resolution uses an explicit Custom `locale` first, then the
assigned proxy's configured `locale`, then the macOS user's preferred/default
locale. In Automatic mode the required legacy `locale` field is stored but does
not pin the effective value. A proxy country alone is never a language hint.
Matching native values skip both language and locale emulation; the stored
control selection stays enabled. A disabled `locale` control always stays native.

For older fingerprints without `locale_mode`, omitted `overrides` means
Automatic, while an explicit override list preserves Custom locale semantics.
Set `locale_mode` explicitly when writing new fingerprints. Resolution is shared
by app, CLI, SDK, MCP, and restored sessions when their Chromium contexts are
prepared. Proxy routing, data, and the remaining privacy controls are unchanged.

Browser identity overrides have been removed. Chromium generates the User-Agent
with a reduced product version and supplies native platform and client hints for every session, including
legacy full profiles. The retired `identity` selection is accepted only to read
old profiles and is removed during normalization. Stored browser identity fields
remain required for the existing schema but do not override browser identity.

For example, add `"overrides": ["timezone"]` to a valid profile with
`"timezone": "Asia/Tokyo"` to change only timezone. Unknown override names and
non-array values are rejected. Saving the profile preserves this list through
export/import. Missing lists in older profiles enable the remaining supported overrides.

New profile drafts and Custom session defaults use **Private**,
with all seven supported controls enabled. Its read-only details are hidden
by default in profiles, identity editors, and session information. Choose
**Show** to the left of the mode value to open a popover without expanding the parent
layout. The **Browser Identity** row belongs to the main Profile section.
Choose **Custom Privacy** to edit individual values and toggles in compact rows.
Profile forms open those settings in a separate editor; **New Browser Identity…**
starts from the current identity and **Use Identity** applies the custom settings
to the Profile draft.
The shared device preset appears once, info buttons explain linked settings,
and the readback seed is in an expandable section. **Native** turns off all
controls. Chromium generates the User-Agent with the product version reduced to
`MAJOR.0.0.0` in every mode. Native branding and client hints remain engine-owned
and are not editable. High-entropy client hints can still expose the full engine
version when requested by a site.

Custom Privacy saves an explicit `overrides` list, including when all seven
controls are enabled or all are disabled. It therefore reopens as Custom.
Private uses the omitted-list representation; selecting it resets custom
values to the standard full preset while retaining the session seed. Native
saves a null fingerprint. Existing explicit choices and saved session-creation
preferences are preserved. Saving recreates only the session's Chromium context.
Device surfaces and graphics remain linked groups in the pinned engine.
Individual fields within those groups cannot independently remain native.

On the Chromium 152 upgrade, REL updates stored Chromium 151 fingerprint
versions and their matching user agents in sessions and named profiles before
loading them. Seeds and other fingerprint settings are preserved; changed
session fingerprints advance their generation. This happens transactionally
in the current app database and does not reset browser data. API submissions
must still use a Chromium 152 four-component version and matching user agent.

### Proxy TLS configuration

Proxy create/update requests accept a `tls` object, also returned on proxy resources:

```json
{"tls":{"mode":"system"}}
{"tls":{"mode":"bright_data"}}
{"tls":{"mode":"custom","certificate_pem":"-----BEGIN CERTIFICATE-----\n…\n-----END CERTIFICATE-----\n"}}
```

Use `system` to clear additional roots. Omission on create selects system trust; omission on update preserves the existing setting. `bright_data` requires `brd.superproxy.io:44445`. `custom` requires a PEM bundle with 1–16 CA certificates and a maximum size of 64 KiB. Unknown modes/fields, malformed certificates, private keys, leaf certificates and inconsistent provider endpoints are rejected before updating the proxy. Normal hostname, validity and chain verification stays enabled.

Session resources additionally contain `proxy_ca_certificates`, a derived array of base64 DER CA certificates from the assigned proxy. Direct sessions return an empty array. This is read-only session metadata; configure trust on the proxy. Updating proxy certificates synchronizes affected open sessions and recreates their browser views. Proxy/profile transfer format version 3 stores the TLS configuration; versions 1 and 2 remain readable and default to system trust.

## Webhooks

Webhook calls use the same loopback RPC v1 API. Browser session cookies,
fingerprints, and proxy settings are not used. Configure up to 32 named webhooks;
REL stores their URLs, credentials, and routing configuration in the current app
variant's macOS Keychain. List and create responses contain metadata only.
Webhook request bodies are limited to 32 KiB.

| Method | Path | Result |
| --- | --- | --- |
| `GET` | `/v1/webhooks` | `{webhooks:[...]}` metadata |
| `POST` | `/v1/webhooks` | Create configuration; `{webhook:{...}}` |
| `DELETE` | `/v1/webhooks/{id}` | Remove configuration and its pending events; `{deleted:true}` |
| `POST` | `/v1/webhooks/{id}/send` | Send once; `{webhook_id,http_status,accepted:true}` |
| `GET` | `/v1/webhooks/events` | `{events:[...]}` pending inbound events |
| `DELETE` | `/v1/webhooks/events/{event_id}` | Acknowledge/remove an event; `{acknowledged:true}` |
| `POST` | `/v1/webhooks/{id}/receive` | Authenticated provider callback |
| `GET` | `/v1/webhooks/{id}/receive` | WhatsApp callback verification only |

There are no webhook CLI commands, MCP tools, or typed SDK methods yet. Scripts
can use these HTTP endpoints directly. Settings provides creation, deletion,
callback copying, and an explicit **Send Test** action.

### Configuration

Create a webhook with a unique `name`, `kind` (`json`, `discord`, or `whatsapp`),
and at least one direction:

```json
{
  "name": "Team updates",
  "kind": "discord",
  "url": "https://discord.com/api/webhooks/WEBHOOK_ID/WEBHOOK_TOKEN"
}
```

For outgoing calls, `url` is an absolute HTTPS URL. HTTP is allowed only for
`localhost`, `127.0.0.1`, and `::1` fixtures. URL userinfo and fragments are
rejected. Optional `bearer_token` sets the Authorization bearer token. WhatsApp
sending additionally requires `recipient` and `bearer_token`; set `url` to the
versioned Graph API `/{phone-number-id}/messages` endpoint. REL does not choose
or upgrade the Graph API version.

For incoming events, add:

```json
{
  "receive": {
    "schedule_id": "UUID-OF-A-SAVED-PROMPT",
    "secret": "PROVIDER-SIGNING-SECRET-OR-DISCORD-PUBLIC-KEY",
    "verify_token": "WHATSAPP-VERIFICATION-TOKEN"
  }
}
```

`receive.schedule_id` is the UUID of a prompt saved in REL. Settings offers a
picker. `secret` must be 32 to 4096 bytes. For Discord it is the application's
64-digit hexadecimal Ed25519 public key. For WhatsApp it is the Meta app secret;
`verify_token` must contain at least 16 characters. JSON webhooks use an HMAC
signing secret and do not need `verify_token`.

Metadata is `{id,name,kind,can_send,schedule_id,receive_path}`. The last two
fields are null for destinations without an incoming trigger. Credentials and
outgoing URLs are never returned. To rotate or change a destination, create a
replacement, update prompt completion selections, and delete the old webhook.
Deletion does not silently redirect prompts to another destination.

### Sending

Send either `{"text":"Hello"}` or `{"payload":{...}}`, never both. The JSON
object `payload` is sent unchanged for service-specific messages such as
WhatsApp templates or Discord embeds. `text` maps to:

- JSON: `{"text":"Hello"}`.
- Discord: `{"content":"Hello","allowed_mentions":{"parse":[]}}`.
- WhatsApp: `{"messaging_product":"whatsapp","to":"RECIPIENT","type":"text","text":{"body":"Hello"}}`.

Text limits are 2,000 characters for Discord and 4,096 for WhatsApp. Longer
messages fail with `INVALID_REQUEST`; they are never silently truncated or
split. Shorten the saved prompt's expected output or supply a service-specific
payload. Discord calls set `wait=true`, preserving other query parameters.

Delivery has a 15-second timeout, disables redirects, and makes no automatic
retry. HTTP 2xx means the provider accepted the request, not that a person read
it. HTTP 429 returns `RATE_LIMITED`; other non-2xx statuses, including redirects,
return `UPSTREAM_UNAVAILABLE`, with `details.http_status`. Transport failures
also return `UPSTREAM_UNAVAILABLE`. These errors have `retryable:false`: after
a timeout or ambiguous failure, inspect the destination before sending again.
Provider response bodies and transport URLs are omitted from errors.

### Receiving and consuming events

The local agent stays bound to loopback. An external provider requires a public
HTTPS relay or reverse proxy forwarding **only** the selected
`/v1/webhooks/{id}/receive` path, preserving the raw JSON body, query string, and
signature headers. Buffer and size-limit the request at the relay. Never expose
management, send, inbox, browser, or other RPC routes to the public internet.
Relaying, DNS, TLS certificates, and provider subscriptions are configured
outside REL. REL must remain open and the Mac reachable.

- **JSON**: `X-REL-Webhook-Signature: sha256=<hex>` contains HMAC-SHA256 of the
  exact raw body using the configured secret. Include a unique event identifier
  in the body when otherwise identical events should run separately.
- **WhatsApp**: `X-Hub-Signature-256: sha256=<hex>` contains HMAC-SHA256 of the
  raw body with the Meta app secret. Verification accepts
  `hub.mode=subscribe`, the configured `hub.verify_token`, and a numeric
  `hub.challenge`; it returns the challenge as plain text. Signed message
  notifications for `object:whatsapp_business_account` enter the inbox. Status
  receipts are acknowledged without triggering a prompt, avoiding reply loops.
- **Discord Webhook Events**: verify `X-Signature-Ed25519` against the timestamp
  concatenated with the raw body. `X-Signature-Timestamp` must be within five
  minutes of the Mac's clock. A signed `type:0` PING receives an empty HTTP 204;
  signed `type:1` events enter the inbox and receive HTTP 204. This endpoint is
  for Discord Webhook Events, not Gateway events or interaction callbacks.

Missing, invalid, or stale signatures return HTTP 401. Other callback errors
use RPC error envelopes. JSON and WhatsApp accepted event responses use the
ordinary success envelope with `{accepted:true}`; exact duplicate bodies return
`{accepted:false}`. WhatsApp status-only receipts return plain text HTTP 200.
Discord callback acknowledgements and WhatsApp verification/status responses
use the provider's wire format without the ordinary RPC envelope/request ID.

Events have `{id,webhook_id,schedule_id,payload,trust}`; `trust` is always
`untrusted_webhook_content`. The inbox holds at most 64 pending events and
returns `RATE_LIMITED` (429) instead of dropping events when full. The most
recent 1,024 accepted body hashes, scoped to their webhook, suppress duplicates
even after acknowledgment. The inbox and duplicate history are in memory and
reset when the agent exits. This is not a durable queue or an exactly-once
processing guarantee.

The app polls every two seconds, waits for the selected prompt to be enabled
and idle, acknowledges the event, and runs that saved prompt with the body
explicitly labeled as untrusted data. The original saved prompt is unchanged.
Incoming data cannot select another prompt or completion destination. Missing,
disabled, and busy prompts leave events pending; remove their webhook or use
the event DELETE endpoint to clear them. A model failure is recorded as a failed
prompt run, without automatically replaying the event. A crash after
acknowledgment can interrupt an event's run. Separate consumers should not
acknowledge events intended for the app.
