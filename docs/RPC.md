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
| `POST` | `/v1/proxies/{alias}/rotate-session` | Rotate an Oxylabs session |
| `POST` | `/v1/proxy-transfers/export` | Export a versioned proxy transfer |
| `POST` | `/v1/proxy-transfers/import` | Import a versioned proxy transfer |
| `GET` | `/v1/sessions` | List persistent browser sessions |
| `POST` | `/v1/sessions` | Create a browser session |
| `POST` | `/v1/sessions/close` | Close every browser session in a group |
| `GET` | `/v1/profiles` | List built-in and custom session profiles |
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
    "worker": { "state": "idle" }
  }
}
```

`build` identifies the installed worktree build and is `null` for agents that
were not launched from a metadata-bearing app bundle. Worker state is
`starting`, `idle`, or `busy`. A startup/operation deadline
violation or failed worker returns `AGENT_UNHEALTHY`, with the worker
snapshot in `error.details.worker`. Health deadlines diagnose stalls; they do not
cancel the active request.

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
persisted session, creating one from **Default** only when none exists. Later
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
| `profile` | Optional built-in or custom profile name for the newly created session. It cannot be combined with `session_id`; omission uses **Default**. |
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
**Default** when it is absent, and navigates it to `url`. `profile` and `group`
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
    "session_id": null,
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
  `oxylabs_location_parameter`, and `oxylabs_location_value`.
- `PATCH /v1/proxies/{alias}` is a true partial update. Missing fields are retained.
  `username:null` or `password:null` clears that value.
- `DELETE /v1/proxies/{alias}` detaches it from all sessions, then returns
  `data.deleted_alias`.
- `POST /v1/proxies/{alias}/rotate-session` requires an Oxylabs-enabled proxy and
  returns `data.proxy`.

Aliases are case-insensitively unique, immutable, and must start with a letter;
they may contain only letters, numbers, hyphens, and underscores (maximum 64
characters), and cannot be a UUID. An alias is the sole public proxy identifier: numeric database IDs
and UUIDs are neither accepted nor returned by proxy APIs.
Oxylabs location requires both parameter and value; parameter is `cc`, `country`,
or `st`. `oxylabs.session_id` is generated by REL and is read-only; rotate it
with the dedicated rotate-session operation.

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

Profiles are named templates copied into future sessions. The three generated
built-ins are **Direct** (direct connection, filters off), **AdBlock**
(AdBlock on), and **BandwidthSaver** (AdBlock on and images larger than 10 kB
blocked). A profile resource is:

```json
{
  "id": "builtin-bandwidth-saver",
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
  "is_builtin": true,
  "created_at": 0
}
```

- `GET /v1/profiles` returns built-ins first, then custom profiles, in
  `data.profiles`.
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

Profile names contain 1–128 non-control characters after trimming and are the
selector used during session creation. On `POST /v1/sessions`, omission selects
**Direct**. Explicit session settings override the selected profile. A present
`proxy_alias:null` is a direct override; a non-null value must reference an
existing proxy. Automatically created sessions for capture, navigation, and
attached pages follow the same rule. Capture events and page responses include
the effective session ID. Browser-data payloads remain app-owned and never
cross RPC in plaintext; the inclusion flags describe what the app has attached to a custom
profile. Importing a selected category again replaces that category in the
app-owned template without changing sessions already created from it.

The fingerprint object is an identity template. When REL.app creates a session
from a named profile, it preserves the template settings and generates a fresh
seed before the session's Chromium context is used. The built-in profiles use
the compatibility template by default.

Fingerprint profiles also accept an optional `overrides` array. Omit the field
to enable all supported overrides, or provide an explicit list to leave
all unlisted controls native. An empty array applies no overrides. All existing
profile value fields remain required and validated; inactive values are stored
for later editing, but are not sent as browser overrides.

| Override | Applied behavior |
| --- | --- |
| `locale` | Language preferences and locale |
| `timezone` | IANA timezone |
| `hardware_concurrency` | Page CPU thread count; workers retain native values in this engine |
| `network` | Network information profile |
| `device_surfaces` | Linked memory, touch, screen/pixel ratio, PDF plugin fallback, and storage quota controls |
| `graphics` | Linked Canvas/WebGL readbacks, WebGL identity/extensions, and unavailable WebGPU adapters |
| `audio` | Audio readbacks using the profile's audio mode |

Browser identity overrides have been removed. Chromium supplies the native
User-Agent, navigator platform, and client hints for every session, including
legacy full profiles. The retired `identity` selection is accepted only to read
old profiles and is removed during normalization. Stored browser identity fields
remain required for the existing schema but do not override browser identity.

For example, add `"overrides": ["timezone"]` to a valid profile with
`"timezone": "Asia/Tokyo"` to change only timezone. Unknown override names and
non-array values are rejected. Saving the profile preserves this list through
export/import. Missing lists in older profiles enable the remaining supported overrides.

In the macOS app, new untemplated sessions and new editor drafts start Native.
Choose **Native + overrides** in **Session Identity**, enable only the desired
controls, and review **Changed from native** before saving. Resetting every
control to native saves a null session fingerprint. Saving recreates only the
session's Chromium context. Named profile templates retain their explicit
settings. Device surfaces and graphics are linked groups in the pinned engine;
individual fields within those groups cannot independently remain native.

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
