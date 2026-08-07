# Rel RPC v1

Rel exposes one local, versioned JSON API. This document is the supported wire
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
session, Rel.app, and the resident agent remain running for other clients.

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
    "http_code": 404,
    "message": "Session machine-<uuid>.Session42 was not found.",
    "retryable": false,
    "details": {
      "id": "machine-<uuid>.Session42"
    }
  }
}
```

`id`, `http_code`, `message`, and `retryable` are required. `details` is an
optional JSON object. Clients must branch on `id`, never parse `message`.
`http_code` always equals the actual HTTP status for ordinary responses; it is
also retained so the identical error object works in NDJSON streams.

`retryable:true` means retrying the same idempotent operation may succeed without
user correction. It does not mean every mutation is automatically safe to
repeat.

### Standard error IDs

| ID | HTTP | Retryable | Meaning |
| --- | ---: | --- | --- |
| `INVALID_REQUEST` | 400 | no | Malformed HTTP or JSON |
| `ROUTE_NOT_FOUND` | 404 | no | No v1 route matches |
| `METHOD_NOT_ALLOWED` | 405 | no | Resource exists but method is unsupported |
| `PAYLOAD_TOO_LARGE` | 413 | no | Request body exceeds 16 MiB |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | no | JSON endpoint received unsupported content |
| `VALIDATION_FAILED` | 422 | no | Parsed request violates field constraints |
| `SESSION_NOT_FOUND` | 404 | no | Session ID does not exist |
| `PAGE_NOT_FOUND` | 404 | no | Ephemeral attached page does not exist |
| `PAGE_MISMATCH` | 409 | no | Attached page state no longer matches the request |
| `PROXY_NOT_FOUND` | 404 | no | Proxy does not exist |
| `CONFLICT` | 409 | no | Name/state/last-session conflict |
| `BROWSER_BUSY` | 409 | yes | Chromium is servicing incompatible work |
| `NETWORK_PAUSED` | 409 | no | Session networking is paused |
| `ACTION_TARGET_NOT_FOUND` | 422 | no | Click target could not be found |
| `REQUEST_CANCELLED` | 409 | yes | Browser work was cancelled |
| `RATE_LIMITED` | 429 | yes | Rel itself is rate limiting the caller |
| `UPSTREAM_UNAVAILABLE` | 502 | yes | Browser/proxy received an invalid upstream result |
| `BROWSER_UNAVAILABLE` | 503 | yes | Required Chromium service is unavailable |
| `AGENT_UNHEALTHY` | 503 | yes | The serialized control worker missed its health deadline |
| `TIMEOUT` | 504 | yes | Rel's operation deadline expired |
| `INTERNAL_ERROR` | 500 | no | Unexpected internal failure |

A target website returning 404 or 429 is not a Rel RPC error. Its status is
reported as `target_http_status` in capture data.

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
violation or failed worker returns HTTP 503 `AGENT_UNHEALTHY`, with the worker
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
  "session_id": "machine-....Session12",
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
  "session_id": "machine-....Session12",
  "output": "/optional/after-click.html",
  "timeout": 90,
  "wait": 1
}
```

`actions` must be a non-empty array. Rel runs the actions in array order.

Capture without another action with `POST /v1/capture`:

```json
{
  "session_id": "machine-....Session12",
  "output": "/optional/current.html",
  "timeout": 90,
  "wait": 1
}
```

All three return the same page-operation envelope documented under attached
pages. When `session_id` is supplied, `navigate` selects and updates that
session's current shorthand page; `perform` and singular `capture` target it.
Without `session_id`, they use the most recently navigated shorthand page for
compatibility. `perform` and singular `capture` return `ACTIVE_PAGE_NOT_FOUND`
with HTTP 409 until a matching page has been selected by navigation. This
registry is process-local and is cleared by an agent restart or when its session
closes. Concurrent work within one session should use explicit page IDs.

### `POST /v1/captures`

```json
{
  "url": "https://example.com",
  "output": "/optional/page.html",
  "timeout": 90,
  "wait": 1,
  "actions": [],
  "session_id": "machine-....Session12",
  "proxy": "office",
  "retry": 1,
  "retry_delay": 3
}
```

| Field | Contract |
| --- | --- |
| `url` | Required HTTP(S) URL; scheme-less input is normalized by the agent. |
| `output` | Optional nonempty path or null; generated when absent. |
| `timeout` | Finite seconds greater than zero; default 90. |
| `wait` | Finite seconds at least zero; default 1. |
| `actions` | Optional array of canonical action objects. |
| `session_id` | Optional existing federated ID. Omission creates a persistent session and returns its ID in capture events. |
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

Preflight failures use the ordinary error response. Once accepted, Rel returns
HTTP 200 `application/x-ndjson`. Each physical line is one complete object; there
is no encoded stdout/stderr layer:

```json
{
  "status": "ok",
  "request_id": "req_...",
  "event": "capture.started",
  "data": {
    "url": "https://example.com/",
    "session_id": "machine-....Session12"
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
contains output path, bytes, final URL, optional `target_http_status`, session ID,
capture ID, and proxy traffic. A target status at least 400 is a completed
capture with `outcome:"target_error"` and CLI exit code 1; it is not an API
error.

## Attached pages

### `POST /v1/pages`

```json
{
  "url": "https://example.com",
  "session_id": "machine-....Session12",
  "proxy": "office",
  "output": "/optional/page.html",
  "timeout": 90,
  "wait": 1
}
```

Omitting session creates one. The final normalized browser URL must equal the
requested URL. Success data:

```json
{
  "page": {
    "id": "page_...",
    "session_id": "machine-....Session12",
    "url": "https://example.com/"
  },
  "capture": {
    "output_path": "tmp/captures/...html",
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
or `st`. `oxylabs.session_id` is generated by Rel and is read-only; rotate it
with the dedicated rotate-session operation.

## Sessions

A session resource is:

```json
{
  "id": "machine-<uuid>.Session12",
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
  `image_blocking_mode`, and `image_size_limit_kb`; returns `data.session` and
  `data.closed_session_ids`.
- `PATCH /v1/sessions/{id}` is partial and returns `data.session`.
- `DELETE /v1/sessions/{id}` returns the opaque session ID as `data.deleted_id`
  and refuses to remove the last session.

`image_blocking_mode` is `all` or `over_limit`. The legacy `block_images` alias
is rejected. Size is 1 through 1,048,576 kB. The visible name is editable and
case-insensitively unique; the opaque `id` is immutable. Session routes accept
only that ID; numeric database IDs are neither accepted nor returned.

## Session defaults

A session-defaults resource controls values used for future sessions plus the
global persistent-session limit. Proxy and filtering values are copied into new
sessions and do not alter existing ones:

```json
{
  "proxy_alias": null,
  "adblock_enabled": true,
  "image_blocking_mode": "over_limit",
  "image_size_limit_kb": 100,
  "max_open_tabs": 8
}
```

- `GET /v1/session-defaults` returns `data.session_defaults`.
- `PATCH /v1/session-defaults` accepts any non-empty subset of the fields above
  and returns `data.session_defaults` plus `data.closed_session_ids`.
  `proxy_alias:null` selects direct networking.

`max_open_tabs` accepts integers from 1 through 100 and defaults to 8. Creating
a session beyond the limit atomically closes the oldest session. Lowering the
setting immediately closes the oldest excess sessions. Closed session IDs are
invalidated, their browsing data is deleted, and their opaque IDs are returned
in `closed_session_ids`.

On `POST /v1/sessions`, every omitted session setting uses this resource. A
present `proxy_alias:null` is an explicit direct override; a present non-null value
must reference an existing proxy. Automatically created sessions for captures
and attached pages follow the same defaults, except an explicit request proxy
overrides the default proxy. Capture events and page responses include
`closed_session_ids` when implicit creation removes older sessions.
