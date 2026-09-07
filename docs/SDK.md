# REL Rust SDK

`rel-client` is the typed Rust client for every public REL RPC v1 operation.
The `rel` CLI uses this crate rather than maintaining a separate transport or
request model.

The SDK source is available under the MIT license in
[`rel-me/rel-tools`](https://github.com/rel-me/rel-tools). REL's application
source and internal runtime implementation are not publicly distributed.

Related documents: [CLI](CLI.md), [MCP](MCP.md), and [RPC](RPC.md).

## Connect

Until a crates.io release is announced, pin the public repository tag:

```toml
[dependencies]
rel-client = { git = "https://github.com/rel-me/rel-tools", tag = "v0.1.1" }
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

The SDK is transport-only: it never launches the REL app, reads REL's SQLite
database, or tails log files. The caller is responsible for ensuring that the
installed app and agent are running. The bundled CLI adds app-launch behavior
for Chromium and mutation commands around this same client.

SDK browser methods inherit the [RPC session-selection behavior](RPC.md#transport):
when REL is inactive, the target session is selected by default without activating
the app. Users can disable this with the General setting **Follow browser
commands**.

## API parity

Each transport method maps to one public RPC route. `read_page` and
`read_observation` are documented composite helpers: the former obtains a new
semantic observation, while the latter re-queries a retained public snapshot
without navigating.

| Rust method | RPC operation |
| --- | --- |
| `health()` | `GET /v1/health` |
| `status()` | `GET /v1/status` |
| `list_notifications()` | `GET /v1/notifications` |
| `navigate(&NavigateRequest)` | `POST /v1/navigate` |
| `navigate_and_observe(&NavigateObservationRequest)` | `POST /v1/navigate/observe` |
| `read_page(&PageReadRequest)` | semantic `POST /v1/navigate/observe` or `POST /v1/observe` |
| `perform(&PerformRequest)` | `POST /v1/perform` |
| `capture_current_page(&PageCaptureRequest)` | `POST /v1/capture` |
| `screenshot_current_page(&ScreenshotRequest)` | `POST /v1/screenshot` |
| `observe_current_page(&ObservationRequest)` | `POST /v1/observe` |
| `capture(&CaptureRequest)` | `POST /v1/captures` |
| `attach_page(&PageAttachRequest)` | `POST /v1/pages` |
| `perform_page_action(page_id, &PageActionRequest)` | `POST /v1/pages/{page_id}/actions` |
| `take_page_screenshot(page_id, &PageScreenshotRequest)` | `POST /v1/pages/{page_id}/screenshot` |
| `observe_page(page_id, &PageObservationRequest)` | `POST /v1/pages/{page_id}/observe` |
| `perform_observation_action(observation_id, &ObservationActionRequest)` | `POST /v1/observations/{observation_id}/actions` |
| `find_in_observation(observation_id, &ObservationFindRequest)` | `POST /v1/observations/{observation_id}/find` |
| `get_observation(observation_id)` | `GET /v1/observations/{observation_id}` |
| `read_observation(observation_id, &ObservationReadRequest)` | composite over `GET /v1/observations/{observation_id}` |
| `list_proxies()` | `GET /v1/proxies` |
| `get_proxy(alias)` | `GET /v1/proxies/{alias}` |
| `create_proxy(&ProxyCreateRequest)` | `POST /v1/proxies` |
| `update_proxy(alias, &ProxyUpdateRequest)` | `PATCH /v1/proxies/{alias}` |
| `delete_proxy(alias)` | `DELETE /v1/proxies/{alias}` |
| `rotate_proxy_session(alias)` | `POST /v1/proxies/{alias}/rotate-session` |
| `export_proxy_transfer(&ProxyTransferExportRequest)` | `POST /v1/proxy-transfers/export` |
| `import_proxy_transfer(&ProxyTransferImportRequest)` | `POST /v1/proxy-transfers/import` |
| `list_sessions()` | `GET /v1/sessions` |
| `get_session(id)` | `GET /v1/sessions/{id}` |
| `create_session(&SessionCreateRequest)` | `POST /v1/sessions` |
| `list_profiles()` | `GET /v1/profiles` |
| `create_profile(&ProfileCreateRequest)` | `POST /v1/profiles` |
| `update_profile_data(id, &ProfileDataUpdateRequest)` | `PATCH /v1/profiles/{id}` |
| `delete_profile(id)` | `DELETE /v1/profiles/{id}` |
| `export_profile_transfer(&ProfileTransferExportRequest)` | `POST /v1/profile-transfers/export` |
| `import_profile_transfer(&ProfileTransferImportRequest)` | `POST /v1/profile-transfers/import` |
| `update_session(id, &SessionUpdateRequest)` | `PATCH /v1/sessions/{id}` |
| `pause_session(id)` | `POST /v1/sessions/{id}/pause` |
| `play_session(id)` | `POST /v1/sessions/{id}/play` |
| `delete_session(id)` | `DELETE /v1/sessions/{id}` |
| `close_session_group(group)` | `POST /v1/sessions/close` |

Ordinary methods return `RpcResponse<T>`, preserving `status`, `request_id`,
and the typed `data` resource. Resources include `Health`, `StatusReport`,
`BrowserNotification`, `PageOperationData`, `Proxy`, and `Session`, with list/data wrapper types that
match RPC v1.

The `rel_client::transfer` module validates the size and SQLite header of
versioned `.relprofile` and `.relproxy` archives and provides their safe output
filenames. `TransferExportData::contents()` decodes the RPC's base64 field, and
the transfer import request `from_bytes` helpers perform the inverse encoding.
The agent owns full schema validation, version checks, and protected Proxy
credential encryption. Both archive types share one five-table SQLite schema;
legacy JSON transfer documents are not supported. The 12 MiB transfer limit is
available as `MAX_TRANSFER_FILE_BYTES`.

`pause_session` and `play_session` return `SessionNetworkStateData`, containing
the canonical `session_id` and resulting `network_paused` value. Both methods
are idempotent; play reloads when the pause interrupted or deferred navigation.
If pause cancels navigation before the new document commits, REL restores the
previous URL and live document; play then resumes without reloading that page.

`Health::build` and `StatusReport::build` expose an optional `BuildIdentity`
with the installed bundle's ID, configuration, worktree, branch, commit, and
dirty state. The field is `None` when the agent was not launched by a
metadata-bearing app bundle.

The bundled [MCP adapter](MCP.md) uses this same client for all fourteen tools. It
calls `status`, `list_notifications`, `capture`, `attach_page`,
`read_page`, `perform_page_action`, both screenshot methods, all observation methods,
`list_sessions`, `close_session_group`, and `list_proxies`; it does not maintain
alternate request types or bypass the RPC transport. For capture, it exhausts
and validates `CaptureStream` before returning one aggregated MCP result.

`BrowserNotification` title and body fields are untrusted website content.
Listing them never starts a model turn; agent clients must keep them in the same
untrusted-data boundary as page text and pixels.

## Shorthand page workflow

For retrieval without action refs or pixels, use `PageReadRequest`. The helper
ranks semantic content and links against `query`, caps the Markdown independently
from the renderer's semantic bound, and reports both truncation states. Reads
include a bounded page-wide heading outline. Unqueried reads sample content
across the document rather than returning only its first sections, and the
result reports available as well as selected content and link counts:

```rust
use rel_client::{PageReadRequest, RelClient};

let client = RelClient::local();
let read = client.read_page(&PageReadRequest {
    url: Some("https://example.com/docs".into()),
    query: Some("installation".into()),
    max_chars: Some(6_000),
    max_sections: Some(16),
    ..PageReadRequest::default()
})?;
println!("{}", read.data.markdown);
# Ok::<(), rel_client::ClientError>(())
```

Matched rating values retain their adjacent labels, semantic link categories
such as genres and labels remain available, and link ranking does not treat a
generic URL path segment as a label match. This helper still uses REL's
embedded Chromium and the public RPC observation routes. It does not fetch
through a second HTTP client or browser backend.

The singular page methods can share the agent's process-local current page. Set
the same `session_id` on each request to scope that page to one browser session:

```rust
use rel_client::{
    Action, NavigateRequest, PageCaptureRequest, PerformRequest, RelClient,
};

let client = RelClient::local();
let session_id = "Session1".to_string();
let mut navigate = NavigateRequest::new("https://example.com");
navigate.session_id = Some(session_id.clone());
client.navigate(&navigate)?;
let mut perform = PerformRequest::new(vec![
    Action::WaitFor {
        selector: "button.more".into(),
        timeout: None,
    },
    Action::Click {
        selector: "button.more".into(),
        mouse_move: None,
        scroll: None,
    },
    Action::Wait { seconds: 0.5 },
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

`capture_current_page` also refreshes the shorthand page binding from the
currently visible URL. Its returned `page.url` is authoritative after History
API, query, or fragment changes made by the page.

`navigate` becomes ready after the requested HTTP(S) main frame starts,
finishes, and has nonempty rendered source. Subframe and page-initiated
background loading does not hold the request open. Its `wait` value is a
bounded settling delay after final main-frame readiness. Use `Action::Wait`
when a workflow needs additional settling time.

Take a visual capture from the same current page with `ScreenshotRequest`, or
use `PageScreenshotRequest` with an explicit attached page ID:

```rust
use rel_client::{RelClient, ScreenshotFormat, ScreenshotRequest};

let client = RelClient::local();
let screenshot = client.screenshot_current_page(&ScreenshotRequest {
    session_id: Some("Session1".into()),
    format: Some(ScreenshotFormat::Webp),
    quality: Some(80),
    full_page: true,
    ..ScreenshotRequest::default()
})?;
println!("{}", screenshot.data.screenshot.output_path);
# Ok::<(), rel_client::ClientError>(())
```

Request compact rendered semantics and typed element refs with
`ObservationRequest`. Hybrid adds a current-viewport PNG resource; visual keeps
semantics minimal. Optional `context` paths on content and elements preserve
their nearest landmark, form, dialog, list, table, and row relationships:

```rust
use rel_client::{
    NavigateObservationRequest, ObservationAction, ObservationActionKind,
    ObservationActionRequest, ObservationFindRequest, ObservationMode,
    ObservationReadRequest, RelClient,
};

let client = RelClient::local();
let observed = client.navigate_and_observe(&NavigateObservationRequest {
    url: Some("https://example.com".into()),
    navigation: None,
    session_id: Some("Session1".into()),
    mode: Some(ObservationMode::Hybrid),
    profile: None,
    proxy: None,
    timeout: None,
    wait: None,
})?;
let first_ref = observed.data.observation.elements[0].element_ref.clone();
let mut hover = ObservationAction::new(first_ref.clone(), ObservationActionKind::Hover);
hover.scroll = Some(true);
let next = client.perform_observation_action(
    &observed.data.observation.id,
    &ObservationActionRequest {
        actions: vec![
            hover,
            ObservationAction::new(first_ref, ObservationActionKind::Click),
            ObservationAction::wait(0.25),
            ObservationAction::scroll(0, -600),
        ],
        mode: Some(ObservationMode::Semantic),
        timeout: None,
        wait: None,
    },
)?;
let found = client.find_in_observation(
    &next.data.observation.id,
    &ObservationFindRequest {
        query: Some("continue".into()),
        role: Some("button".into()),
        limit: Some(10),
    },
)?;
println!("{}", found.data.total_matches);
let recalled = client.read_observation(
    &observed.data.observation.id,
    &ObservationReadRequest {
        query: Some("important facts and ratings".into()),
        max_chars: Some(6_000),
        max_sections: Some(20),
    },
)?;
println!("{}", recalled.data.markdown);
# Ok::<(), rel_client::ClientError>(())
```

Refs are scoped to one observation and document sequence. The agent retains
private locators and returns `OBSERVATION_STALE` instead of retargeting when the
document or element signature has changed. Observation actions execute in order,
stop at the first failure, and return one post-batch observation. Find searches
only the stored public snapshot and does not issue another browser read.
Navigation invalidates and erases private locators but retains the bounded public
snapshot for reading until the 32-observation registry evicts it, the session
closes, or the agent exits. Retained snapshots are evidence, not actionable page
state.

`navigate` returns `ClientError::Rpc` with ID `UPSTREAM_UNAVAILABLE` when the
main frame commits an HTTP 4xx or 5xx response. By default, detected Cloudflare
Turnstile and managed challenge pages first receive up to 15 seconds to
continue; this can be disabled in REL's General settings. Error details include
the final `url` and exact `target_http_status`; the navigated session remains
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

## Browser actions

The public `Action` enum serializes directly to the RPC v1 object shapes. See
the [Actions reference](ACTIONS.md) for every variant, a Rust example, selector
constraints, defaults, and failure behavior. Browser sessions controlled while
not visible use the global **Background Browser Size** preset; it is not
duplicated as an SDK field.

## Partial updates

Non-nullable PATCH fields use `Option<T>`: `None` omits a field and `Some(value)`
sets it. Nullable fields use `Change<T>` so callers can also send an explicit
JSON `null`. Those fields are proxy username, password, and Oxylabs location,
plus the `proxy_alias` for a session.

```rust
use rel_client::{Change, RelClient, SessionUpdateRequest};

let request = SessionUpdateRequest {
    name: Some("Research".into()),
    proxy_alias: Change::Clear,
    ..SessionUpdateRequest::default()
};
RelClient::local().update_session("Session12", &request)?;
# Ok::<(), rel_client::ClientError>(())
```

- `Option::None` omits a non-nullable field; `Option::Some(value)` sets it.
- `Change::Unchanged` omits a nullable field.
- `Change::Set(value)` sets a nullable field.
- `Change::Clear` sends JSON `null` for a nullable field.

This prevents an accidental clear when a caller intended a true partial
update.

## Session profiles

`SessionCreateRequest::default()` serializes to `{}`, so the agent copies the
configured **Default Profile**, or **Private** when the preference is unset. Set `profile` to select **AdBlock**,
**BandwidthSaver**, or a case-insensitively unique custom name. Explicit proxy
and filtering fields override the selected profile; use
`Change::Set("alias".into())` for a proxy or `Change::Clear` for direct
networking:

```rust
use rel_client::{Change, RelClient, SessionCreateRequest};

let request = SessionCreateRequest {
    group: Some("pgm".into()),
    profile: Some("BandwidthSaver".into()),
    proxy_alias: Change::Clear,
    ..SessionCreateRequest::default()
};
RelClient::local().create_session(&request)?;
RelClient::local().close_session_group("pgm")?;
# Ok::<(), rel_client::ClientError>(())
```

`Profile` contains its public `id`, unique `name`, proxy and filtering policy,
browser-data inclusion flags, optional fingerprint identity template, built-in
status, and creation time.
`ProfileCreateRequest` creates a custom settings template; browser-data import
itself remains app-owned. `ProfileDataUpdateRequest` updates the two inclusion
flags after REL.app stages an import; no cookie or password values cross RPC.
Re-importing a selected category replaces that category in the template.
Built-ins cannot be modified or deleted. `ImageBlockingMode::None` allows every
image without disabling AdBlock. Existing sessions retain copied settings and
data after their source profile is changed or deleted. REL Free can create one
persistent Session and one custom Profile; REL Pro removes those limits.

`ProfileCreateRequest::fingerprint_profile` uses `Change::Unchanged` to select
the default compatibility template, `Change::Clear` for native Chromium, and
`Change::Set(profile)` for explicit identity settings. REL.app preserves those
settings but generates a fresh seed whenever it creates a session from the
profile. The three built-in profiles use the compatibility template.

`Session::profile` exposes the source profile name and
`Session::profile_data_id` identifies the custom browser-data template copied
at creation, when any. `CaptureRequest`, `NavigateRequest`, and
`PageAttachRequest` accept `profile` only when `session_id` is absent.
`CaptureRequest` and `PageAttachRequest` also accept `group`, so an implicitly
created session can join a group. Group matching is case-insensitive; closing
an empty group succeeds with an empty `deleted_ids` vector.

`ProxyCreateRequest` requires an immutable, unique `alias`. The typed proxy
methods and the capture/page `proxy` field accept only that alias; public proxy
resources never expose or accept numeric IDs or UUIDs. Proxy creation, update,
rotation, assignment, and use require REL Pro. On Free, those calls return a
non-retryable `PRO_REQUIRED` error with feature and plan details.

Sessions similarly expose their immutable canonical `id` (for example,
`Session12`) as their sole public identifier. The typed session
methods accept that string, and `Session` and session deletion responses do not
expose numeric database IDs.

## Errors and validation

SDK failures use one `ClientError` type:

| Variant | Meaning |
| --- | --- |
| `Transport` | The agent could not be reached or the HTTP exchange failed. |
| `Protocol` | Content type, request ID, envelope, or event shape violated RPC v1. |
| `Rpc(RpcFailure)` | REL returned the standard structured RPC error envelope. |
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
wire contract. SDK versions are distributed alongside compatible REL releases.

### Per-proxy CA certificates

`ProxyCreateRequest` and `ProxyUpdateRequest` expose `tls: Option<ProxyTls>`:

```rust
use rel_client::{ProxyTls, ProxyUpdateRequest};
let request = ProxyUpdateRequest {
    tls: Some(ProxyTls::Custom {
        certificate_pem: std::fs::read_to_string("company-root-ca.pem")?,
    }),
    ..Default::default()
};
```

`ProxyTls::System` clears added roots, and `ProxyTls::BrightData` uses REL's bundled root for `brd.superproxy.io:44445`. `None` preserves trust on update and selects system trust on create. Proxy responses include the selected `tls` configuration. All certificate validation and session scoping is performed by the agent and embedded browser.
