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

By default, a browser operation selects its target tab when REL is inactive, so
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
| `10300` | `UPSTREAM_UNAVAILABLE` | yes | Navigation received a target HTTP error or the browser/proxy received an invalid upstream result |
| `10301` | `BROWSER_UNAVAILABLE` | yes | Required Chromium service is unavailable |
| `10302` | `AGENT_UNHEALTHY` | yes | The serialized control worker missed its health deadline |
| `10303` | `TIMEOUT` | yes | REL's operation deadline expired |
| `10304` | `PROXY_CONFIGURATION_FAILED` | no | Chromium could not apply the session proxy configuration |
| `10305` | `BROWSER_CREATION_FAILED` | no | Chromium could not create the session browser |
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

## Routes

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/v1/health` | Readiness of the agent control worker |
| `GET` | `/v1/status` | App, agent, proxy, and Chromium diagnostic report |
| `POST` | `/v1/navigate` | Navigate and select the current shorthand page |
| `POST` | `/v1/perform` | Perform actions on the current shorthand page |
| `POST` | `/v1/capture` | Capture the current shorthand page |
| `POST` | `/v1/captures` | Capture rendered HTML as an NDJSON operation |
| `POST` | `/v1/pages` | Attach an ephemeral automation page |
| `POST` | `/v1/pages/{page_id}/actions` | Perform one action on an attached page |
| `GET` | `/v1/proxies` | List proxies |
| `POST` | `/v1/proxies` | Create a proxy |
| `GET` | `/v1/proxies/{alias}` | Read one proxy |
| `PATCH` | `/v1/proxies/{alias}` | Partially update a proxy |
| `DELETE` | `/v1/proxies/{alias}` | Delete and detach a proxy |
| `POST` | `/v1/proxies/{alias}/rotate-session` | Rotate an Oxylabs session |
| `GET` | `/v1/sessions` | List persistent browser sessions |
| `POST` | `/v1/sessions` | Create a browser session |
| `GET` | `/v1/session-defaults` | Read defaults for newly created sessions |
| `PATCH` | `/v1/session-defaults` | Update defaults for newly created sessions |
| `GET` | `/v1/sessions/{id}` | Read one browser session |
| `PATCH` | `/v1/sessions/{id}` | Partially update a browser session |
| `DELETE` | `/v1/sessions/{id}` | Delete a browser session |

There are deliberately no log read, clear, or ingestion routes.

The [`rel-client`](SDK.md) Rust crate exposes one typed method for every route
in this table. The bundled CLI is built on that crate and uses resource commands
such as `rel capture`, `rel page`, `rel proxy`, and `rel session`; it has no
direct database or log-file command path.

The bundled `rel mcp` adapter also calls this API only through `rel-client`. It
maps six MCP tools to status, capture, page attachment and action, and session
and proxy listing. MCP does not add an HTTP `/mcp` route or another response
shape to RPC v1. See [MCP](MCP.md) for its stdio lifecycle and result wrapping.

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
    "worker": { "state": "idle" }
  }
}
```

Worker state is `starting`, `idle`, or `busy`. A startup/operation deadline
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
persisted session, creating one only when none exists. Later requests without it
reuse the current page and session. An explicit session selects that session as
the new current page.

Perform one or more canonical actions with `POST /v1/perform`:

```json
{
  "actions": [
    { "action": "click", "selector": "button.more" },
    { "action": "wait-for", "selector": "#results" }
  ],
  "session_id": "Session12",
  "output": "/optional/after-click.html",
  "timeout": 90,
  "wait": 1
}
```

`actions` must be a non-empty array. REL runs the actions in array order.

Capture without another action with `POST /v1/capture`:

```json
{
  "session_id": "Session12",
  "output": "/optional/current.html",
  "timeout": 90,
  "wait": 1
}
```

All three return the same page-operation envelope documented under attached
pages. When `session_id` is supplied, `navigate` selects and updates that
session's current shorthand page; `perform` and singular `capture` target it.
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
| `wait` | Finite seconds at least zero; default 1. |
| `actions` | Optional array of canonical action objects. |
| `session_id` | Optional existing canonical `Session<number>` ID. Omission creates a persistent session and returns its ID in capture events. |
| `proxy` | Optional unique proxy alias string, assigned to the created session or applied to the existing session. |
| `retry` | Integer 0 through 100; default 1. |
| `retry_delay` | Finite seconds 0 through 86400; default 3. |

The RPC accepts only action objects:

```json
{ "action": "click", "selector": "button.more" }
{ "action": "wait-for", "selector": "#loaded-content" }
{ "action": "wait", "seconds": 0.5 }
{
  "action": "click-link",
  "link": "https://example.com/next",
  "match": { "type": "fuzzy-link", "threshold": 0.9 }
}
```

`wait-for` completes when the CSS selector is present in the live DOM and uses
the enclosing operation's `timeout` as its deadline.

The legacy `output_mode` field and function-like action strings are rejected.

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

Omitting `session_id` creates a session and navigates it to `url`. Providing an
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
  "action": { "action": "click", "selector": "button" },
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

## Sessions

A session resource is:

```json
{
  "id": "Session12",
  "name": "Session12",
  "proxy_alias": null,
  "adblock_enabled": true,
  "image_blocking_mode": "over_limit",
  "image_size_limit_kb": 100,
  "created_at": 1785860000
}
```

- `GET /v1/sessions` returns `data.sessions`.
- `GET /v1/sessions/{id}` returns `data.session`.
- `POST /v1/sessions` accepts optional `name`, `proxy_alias`, `adblock_enabled`,
  `image_blocking_mode`, and `image_size_limit_kb`; returns `data.session`.
- `PATCH /v1/sessions/{id}` is partial and returns `data.session`.
- `DELETE /v1/sessions/{id}` returns the canonical session ID as
  `data.deleted_id` and refuses to remove the last session.

`image_blocking_mode` is `all` or `over_limit`. The legacy `block_images` alias
is rejected. Size is 1 through 1,048,576 kB. The visible name is editable and
case-insensitively unique; the canonical `id` is immutable. Session routes accept
only that ID; numeric database IDs are neither accepted nor returned.

## Session defaults

A session-defaults resource controls values used for future sessions. Proxy and
filtering values are copied into new sessions and do not alter existing ones:

```json
{
  "proxy_alias": null,
  "adblock_enabled": true,
  "image_blocking_mode": "over_limit",
  "image_size_limit_kb": 100
}
```

- `GET /v1/session-defaults` returns `data.session_defaults`.
- `PATCH /v1/session-defaults` accepts any non-empty subset of the fields above
  and returns `data.session_defaults`. `proxy_alias:null` selects direct
  networking.

On `POST /v1/sessions`, every omitted session setting uses this resource. A
present `proxy_alias:null` is an explicit direct override; a present non-null value
must reference an existing proxy. Automatically created sessions for captures
and attached pages follow the same defaults, except an explicit request proxy
overrides the default proxy. Capture events and page responses include
the effective session ID.
