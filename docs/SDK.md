# Rel Rust SDK

`rel-client` is the typed Rust client for every public Rel RPC v1 operation.
The `rel` CLI uses this crate rather than maintaining a separate transport or
request model.

The SDK source is available under the MIT license in
[`gabriel/rel-tools`](https://github.com/gabriel/rel-tools). Rel's application
source and internal runtime implementation are not publicly distributed.

Related documents: [CLI](CLI.md), [MCP](MCP.md), and [RPC](RPC.md).

## Connect

Until a crates.io release is announced, pin the public repository tag:

```toml
[dependencies]
rel-client = { git = "https://github.com/gabriel/rel-tools", tag = "v0.1.1" }
```

```rust
use rel_client::RelClient;

let client = RelClient::local();
let status = client.status()?;
println!("{}", status.data.overall_status);
# Ok::<(), rel_client::ClientError>(())
```

`RelClient::local()` connects to `http://127.0.0.1:17319/v1` and honors
`REL_AGENT_PORT`. `RelClient::new(base_url)` accepts an explicit RPC v1 base
URL. `with_request_timeout(Duration)` changes the ten-second timeout used by
ordinary requests. Capture and page methods derive longer deadlines from their
operation timeout, wait, retry count, and retry delay.

The SDK is transport-only: it never launches Rel.app, reads Rel's SQLite
database, or tails log files. The caller is responsible for ensuring that the
installed app and agent are running. The bundled CLI adds app-launch behavior
for Chromium and mutation commands around this same client.

SDK browser methods inherit the [RPC tab-selection behavior](RPC.md#transport):
when Rel is inactive, the target tab is selected by default without activating
the app. Users can disable this with the General setting **Follow browser
commands**.

## API parity

Each method maps to one public RPC route:

| Rust method | RPC operation |
| --- | --- |
| `health()` | `GET /v1/health` |
| `status()` | `GET /v1/status` |
| `navigate(&NavigateRequest)` | `POST /v1/navigate` |
| `perform(&PerformRequest)` | `POST /v1/perform` |
| `capture_current_page(&PageCaptureRequest)` | `POST /v1/capture` |
| `capture(&CaptureRequest)` | `POST /v1/captures` |
| `attach_page(&PageAttachRequest)` | `POST /v1/pages` |
| `perform_page_action(page_id, &PageActionRequest)` | `POST /v1/pages/{page_id}/actions` |
| `list_proxies()` | `GET /v1/proxies` |
| `get_proxy(alias)` | `GET /v1/proxies/{alias}` |
| `create_proxy(&ProxyCreateRequest)` | `POST /v1/proxies` |
| `update_proxy(alias, &ProxyUpdateRequest)` | `PATCH /v1/proxies/{alias}` |
| `delete_proxy(alias)` | `DELETE /v1/proxies/{alias}` |
| `rotate_proxy_session(alias)` | `POST /v1/proxies/{alias}/rotate-session` |
| `list_sessions()` | `GET /v1/sessions` |
| `get_session(id)` | `GET /v1/sessions/{id}` |
| `create_session(&SessionCreateRequest)` | `POST /v1/sessions` |
| `session_defaults()` | `GET /v1/session-defaults` |
| `update_session_defaults(&SessionDefaultsUpdateRequest)` | `PATCH /v1/session-defaults` |
| `update_session(id, &SessionUpdateRequest)` | `PATCH /v1/sessions/{id}` |
| `delete_session(id)` | `DELETE /v1/sessions/{id}` |

Ordinary methods return `RpcResponse<T>`, preserving `status`, `request_id`,
and the typed `data` resource. Resources include `Health`, `StatusReport`,
`PageOperationData`, `Proxy`, and `Session`, with list/data wrapper types that
match RPC v1.

The bundled [MCP adapter](MCP.md) uses this same client for all six tools. It
calls `status`, `capture`, `attach_page`, `perform_page_action`, `list_sessions`,
and `list_proxies`; it does not maintain alternate request types or bypass the
RPC transport. For capture, it exhausts and validates `CaptureStream` before
returning one aggregated MCP result.

## Shorthand page workflow

The singular page methods can share the agent's process-local current page. Set
the same `session_id` on each request to scope that page to one browser session:

```rust
use rel_client::{
    Action, NavigateRequest, PageCaptureRequest, PerformRequest, RelClient,
};

let client = RelClient::local();
let session_id = "machine-a.Session1".to_string();
let mut navigate = NavigateRequest::new("https://example.com");
navigate.session_id = Some(session_id.clone());
client.navigate(&navigate)?;
let mut perform = PerformRequest::new(vec![
    Action::Click {
        selector: "button.more".into(),
    },
    Action::WaitFor {
        selector: "#results".into(),
    },
]);
perform.session_id = Some(session_id.clone());
client.perform(&perform)?;
let capture = client.capture_current_page(&PageCaptureRequest {
    session_id: Some(session_id),
    output: Some("/tmp/final.html".into()),
    ..PageCaptureRequest::default()
})?;
println!("{}", capture.data.capture.output_path);
# Ok::<(), rel_client::ClientError>(())
```

`navigate` returns `ClientError::Rpc` with ID `UPSTREAM_UNAVAILABLE` as soon as
the main frame commits an HTTP 4xx or 5xx response. Its details include the
final `url` and exact `target_http_status`; the navigated session remains
selected.

The first navigation without a session ID reuses the first persisted session,
creating one only when none exists; later unscoped requests use the most recent
shorthand page. Session-scoped shorthand pages let clients operate concurrently
across sessions. The state is cleared when the agent restarts or the session
closes. Use explicit page methods for concurrent work within one session.

## Capture streaming

`capture` returns a lazy `CaptureStream`, an iterator of validated
`Result<CaptureEvent, ClientError>` values:

```rust
use rel_client::{Action, CaptureRequest, RelClient};

let client = RelClient::local();
let mut request = CaptureRequest::new("https://example.com");
request.output = Some("/tmp/example.html".into());
request.actions.push(Action::Wait { seconds: 0.5 });

let mut stream = client.capture(&request)?;
for event in stream.by_ref() {
    let event = event?;
    println!("{}", event.event);
}

if !stream.is_finished() {
    return Err("capture stream ended before capture.finished".into());
}
println!("exit code: {}", stream.exit_code().unwrap_or(1));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`request_id()` exposes the response request ID. `is_finished()` becomes true
only after a valid `capture.finished` event has been read; `exit_code()` is then
available. The iterator rejects malformed JSON, invalid event envelopes,
request-ID mismatches, and a terminal event without an integer exit code.
Capture and attached-page responses always expose an absolute filesystem path
in `output_path`, even when a request supplied a relative `output` path. The MCP
adapter maps these paths to `file:///` URIs; the Rust SDK preserves the native
RPC path contract.

Dropping `CaptureStream` before `capture.finished` closes its HTTP connection
and cancels the matching agent and Chromium operation. The persistent session
and resident agent remain available.

## Canonical actions

The public `Action` enum serializes directly to the RPC v1 object shapes:

```rust
use rel_client::{Action, FuzzyLinkMatch};

let click = Action::Click {
    selector: "button.more".into(),
};
let wait_for = Action::WaitFor {
    selector: "#loaded-content".into(),
};
let wait = Action::Wait { seconds: 0.5 };
let link = Action::ClickLink {
    link: "https://example.com/more".into(),
    match_rule: FuzzyLinkMatch::new(0.9),
};
```

There are no function-style action strings or compatibility action shapes in
the SDK.

## Partial updates

Non-nullable PATCH fields use `Option<T>`: `None` omits a field and `Some(value)`
sets it. Nullable fields use `Change<T>` so callers can also send an explicit
JSON `null`. Those fields are proxy username, password, and Oxylabs location,
plus the `proxy_alias` for a session or Session defaults.

```rust
use rel_client::{Change, RelClient, SessionUpdateRequest};

let request = SessionUpdateRequest {
    name: Some("Research".into()),
    proxy_alias: Change::Clear,
    ..SessionUpdateRequest::default()
};
RelClient::local().update_session("machine-<uuid>.Session12", &request)?;
# Ok::<(), rel_client::ClientError>(())
```

- `Option::None` omits a non-nullable field; `Option::Some(value)` sets it.
- `Change::Unchanged` omits a nullable field.
- `Change::Set(value)` sets a nullable field.
- `Change::Clear` sends JSON `null` for a nullable field.

This prevents an accidental clear when a caller intended a true partial
update.

## Session creation defaults

`SessionCreateRequest::default()` serializes to `{}`, so the agent copies the
Session defaults configured in Rel.app. Use `Change::Set("alias".into())` to override the
default proxy or `Change::Clear` to create a direct session:

```rust
use rel_client::{Change, RelClient, SessionCreateRequest};

let request = SessionCreateRequest {
    proxy_alias: Change::Clear,
    ..SessionCreateRequest::default()
};
RelClient::local().create_session(&request)?;
# Ok::<(), rel_client::ClientError>(())
```

The `SessionDefaults` resource contains `proxy_alias`, `adblock_enabled`,
`image_blocking_mode`, `image_size_limit_kb`, and `max_open_tabs`. Proxy and
filter updates affect only subsequently created sessions. `max_open_tabs`
defaults to 8, accepts 1 through 100, and closes the oldest excess sessions when
lowered. `SessionData` and `SessionDefaultsData` expose those removed opaque IDs
as `closed_session_ids`.

`ProxyCreateRequest` requires an immutable, unique `alias`. The typed proxy
methods and the capture/page `proxy` field accept only that alias; public proxy
resources never expose or accept numeric IDs or UUIDs.

Sessions similarly expose their immutable opaque `id` (for example,
`machine-<uuid>.Session12`) as their sole public identifier. The typed session
methods accept that string, and `Session` and session deletion responses do not
expose numeric database IDs.

## Errors and validation

SDK failures use one `ClientError` type:

| Variant | Meaning |
| --- | --- |
| `Transport` | The agent could not be reached or the HTTP exchange failed. |
| `Protocol` | Content type, request ID, envelope, or event shape violated RPC v1. |
| `Rpc(RpcFailure)` | Rel returned the standard structured RPC error envelope. |
| `Io` | Reading a response or capture stream failed. |
| `Json` | JSON serialization or deserialization failed. |

Use `ClientError::rpc_failure()` to inspect an optional `RpcFailure`, then
branch on `failure.error.code` or `failure.error.id`. `RpcError` preserves the
numeric `code`, string `id`, `message`, `retryable`, and optional object-valued
`details`. The `rpc_error_codes` module exports constants for every standard
code; all application codes are 10,000 or greater and are unrelated to HTTP
statuses.

The client validates `Content-Type`, requires `X-Request-Id`, and checks it
against the envelope or every NDJSON event.

## Stability

The SDK targets RPC v1 only. Removing legacy CLI syntax does not change this
wire contract. SDK versions are distributed alongside compatible Rel releases.
