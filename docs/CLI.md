# Rel CLI

The recommended CLI is the `rel` binary bundled in `/Applications/Rel.app` and
linked from a writable directory already in `PATH`. Use **Rel → Settings… →
General → Install Command Line** to create the link. If no safe destination is
available, Rel leaves the filesystem unchanged so you can create the link
manually. The matching public client can also be installed from source:

```sh
cargo install --git https://github.com/gabriel/rel-tools \
  --tag v0.1.1 \
  --package rel-cli
```

The CLI is a thin client built on the typed [`rel-client`](SDK.md)
Rust crate. Ordinary user-facing commands map directly to an [RPC v1](RPC.md)
operation. The `mcp` command adapts a focused subset of those same operations
to stdio MCP; the CLI does not implement another browser or read application
data directly.

Related documents: [MCP](MCP.md), [SDK](SDK.md), and [RPC](RPC.md).

## Commands

```text
rel health
rel status
rel navigate URL [options]
rel perform ACTIONS [options]
rel capture [options]
rel URL [options]
rel capture URL [options]
rel page attach URL [options]
rel page action PAGE_ID --action JSON [options]
rel proxy list
rel proxy get ALIAS
rel proxy create --alias ALIAS --upstream-host HOST --upstream-port PORT [options]
rel proxy update ALIAS [options]
rel proxy delete ALIAS
rel proxy rotate ALIAS
rel session list
rel session get SESSION_ID
rel session create [options]
rel session update SESSION_ID [options]
rel session delete SESSION_ID
rel mcp
rel --help | -h
rel --version
```

`rel --agent` exists only in the proprietary binary bundled with Rel.app. It is
not part of the public `rel-cli` package.

`health` and `status` inspect the currently running agent without launching the
app. Every other command, including `mcp`, proxy reads, and session reads,
starts Rel.app in the background when its agent is unavailable. `rel mcp`
performs that startup check once, then serves its original stdio connection.
`REL_AGENT_PORT` overrides the default local port, `17319`.

When a browser command targets a tab while Rel is in the background, Rel selects
that tab by default without activating the app. Turn off **Rel → Settings… →
General → Follow browser commands** to keep the current tab selected instead.
Internal session synchronization and read-only resource commands do not change
the selection.

Capture with a URL remains the default URL-first command: `rel URL [options]`.
The explicit `rel capture URL [options]` form is equivalent. Argument-free
`rel capture` instead captures the current shorthand page selected by
`rel navigate`. The removed `ping`, `logs`, and
`--rotate-proxy-session` interfaces have no compatibility aliases; use
`status`, the app's Logs view, and `proxy rotate`, respectively.

## Quick examples

### Like curl, but from a real browser

Give Rel a URL to load it in embedded Chromium and write the rendered HTML to
standard output:

```sh
rel https://example.com
```

Redirect it just like curl when saving the page is more useful:

```sh
rel https://example.com > example.html
```

### Create a proxied session and keep using it

Create the named proxy first:

```sh
rel proxy create \
  --alias office \
  --upstream-host proxy.example.com \
  --upstream-port 8000 \
  --username account \
  --password secret
```

By default, `session create` returns the ordinary JSON response envelope. Add
`--id-only` to print just the opaque ID for shell command substitution, then
quote the variable when passing it to later commands:

```sh
session_id="$(rel session create --name Research --proxy=office --id-only)"

rel navigate https://example.com --session-id="$session_id"
rel capture --session-id="$session_id" > example.html
```

For a sequence of commands, export the ID under Rel's standard environment
variable and omit the repeated options:

```sh
export REL_SESSION_ID="$session_id"

rel navigate https://example.com
rel perform '[{"action":"wait-for","selector":"body"}]'
rel capture > example.html
```

An explicit `--session-id` always takes precedence over `REL_SESSION_ID`.

## Output and errors

Ordinary commands print the complete pretty-printed RPC v1 success envelope:

```json
{
  "status": "ok",
  "request_id": "req_...",
  "data": {}
}
```

Both capture forms write rendered HTML to standard output when `--output` is
omitted. URL capture writes validated NDJSON events to standard error, one
compact event envelope per physical line. For URL capture with `--output`,
stdout remains empty and events remain on stderr. Current-page capture with
`--output` prints its JSON response envelope. This keeps the primary artifact
pipeable without mixing it with progress metadata:

```sh
rel https://example.com > example.html 2> capture.ndjson
```

The CLI verifies response content types and request IDs through `rel-client`.
Pressing Ctrl-C terminates the foreground CLI and closes its RPC connection.
If a browser operation is still active, the resident agent cancels the matching
Chromium work; Rel.app, the agent, and the persistent browser session remain
available for later commands.

`rel mcp` is a protocol process rather than an ordinary one-shot command. Its
standard output contains only newline-delimited JSON-RPC 2.0 messages for the
MCP client, and diagnostics use standard error. See [MCP](MCP.md) for its
version negotiation, tool results, and errors.

An RPC failure is printed to standard error as the complete structured error
envelope. Argument, transport, and protocol failures are plain text on standard
error. Clients should branch on `error.id`, not `error.message`.

Exit status is:

| Status | Meaning |
| --- | --- |
| `0` | Help/version or a successful RPC operation. `status` also requires `overall_status:"ok"`. |
| `1` | Usage, transport, protocol, or RPC failure; unhealthy `status`; or the terminal exit code from an unsuccessful capture. |
| `130` | The shell terminated the CLI with Ctrl-C (`SIGINT`). |

For `rel mcp`, clean stdin closure exits successfully; startup or stdio failure
exits unsuccessfully.

## MCP server

```sh
/Applications/Rel.app/Contents/Resources/rel mcp
```

The command accepts no options. MCP clients normally launch it and own its
stdin/stdout pipes rather than running it in an interactive terminal. It
supports current `2026-07-28` discovery and legacy initialization through
`2025-11-25`, and exposes exactly six tools: `rel_status`, `rel_capture`,
`rel_page_attach`, `rel_page_action`, `rel_list_sessions`, and
`rel_list_proxies`.

Every tool forwards through `rel-client` and RPC v1. Capture aggregates its
validated NDJSON stream into `{request_id, exit_code, events}`. Every tool
execution result includes its complete JSON in both a text content block and
`structuredContent`. Captured files use absolute `file:///` URIs at the MCP
boundary and are also returned as standard MCP `resource_link` content blocks.

## Health and status

```sh
rel health
rel status
```

`health` calls `GET /v1/health` and reports agent worker readiness. `status`
calls `GET /v1/status` and reports the app, agent, Browser Proxy, and Chromium
bridge checks. Neither command synthesizes a local process report when the
agent is unavailable.

## Shorthand page workflow

For sequential automation that does not need to manage page or session IDs:

```sh
rel navigate https://example.com
rel perform '[{"action":"click","selector":"button.more"},{"action":"wait-for","selector":"#results"}]'
rel capture > final.html
```

`navigate` calls `POST /v1/navigate`, navigates the current shorthand page, and
prints the ordinary JSON response envelope. Its first call reuses the first
persisted session unless `--session-id` or `REL_SESSION_ID` supplies one,
creating a session only when none exists. Later calls without a session ID reuse
the current page and session. It accepts `--session-id`, `--proxy`, `--output`,
`--timeout`, and `--wait`.

If the main frame returns HTTP 4xx or 5xx, `navigate` exits unsuccessfully as
soon as Chromium commits that response instead of waiting for all background
loading to stop. The `UPSTREAM_UNAVAILABLE` error message and details include
the exact target status and final URL. The rendered page remains selected in
the session.

`perform` calls `POST /v1/perform` with one positional, non-empty JSON array of
canonical action objects. Actions run in array order. It accepts `--session-id`,
`--output`, `--timeout`, and `--wait`, then prints the JSON response envelope
containing the page and capture metadata. A single action must still be wrapped
in an array.

Argument-free `capture` calls `POST /v1/capture`. Without `--output`, it writes
the current rendered HTML to stdout. With `--output`, it prints the JSON response
envelope. It accepts `--output`, `--timeout`, and `--wait` but not URL-capture
options such as `--action`, `--proxy`, or `--retry`. It also accepts
`--session-id`.

For `navigate`, `perform`, and argument-free `capture`, `--session-id` defaults
to `REL_SESSION_ID` when set. The agent keeps a distinct current shorthand page
for each session, so embedded terminals can use these commands concurrently.
An explicit option always wins. The shorthand registry is process-local and a
session's entry disappears when the agent restarts or that session closes.
Concurrent work within the same session should use `page attach` and
`page action` with explicit page IDs.

## Capture

```text
rel URL [options]
rel capture URL [options]
```

Capture loads a page in an embedded Chromium session, performs optional
actions, and writes the rendered HTML to stdout or an explicit output file.

| Option | RPC field | Contract |
| --- | --- | --- |
| `--output PATH` | `output` | Write HTML to this path instead of stdout. |
| `--timeout SECONDS` | `timeout` | Positive finite Chromium-operation timeout; default `90`. |
| `--wait SECONDS` | `wait` | Nonnegative finite delay after main-frame readiness; default `1`. |
| `--action JSON` | `actions[]` | One canonical action object; repeat the option for multiple actions. |
| `--actions JSON` | `actions` | A JSON array of canonical action objects, executed in order. |
| `--session-id ID` | `session_id` | Reuse an existing immutable federated session ID. When omitted, use `REL_SESSION_ID` if set; otherwise create a persistent session. |
| `--proxy ALIAS` | `proxy` | Select a proxy by its unique alias for the created or reused session. |
| `--retry COUNT` | `retry` | Retry count from 0 through 100; default `1`. |
| `--retry-delay SECONDS` | `retry_delay` | Finite delay from 0 through 86400 seconds; default `3`. |

Exactly one URL is required before the options. Scheme-less localhost
addresses use HTTP; other scheme-less hosts use HTTPS. Only HTTP and HTTPS are
accepted.

When `--session-id` is omitted, the CLI uses `REL_SESSION_ID` if it is set. This
is exported automatically by each embedded session terminal. An explicit option
always wins. If neither is present, capture creates a persistent browser session.
Its default label is `Session<ID>` and its immutable identifier is:

```text
machine-<installation UUID>.Session<ID>
```

For a new session, `--proxy oxylabs` is shorthand for creating a persistent
session assigned to `oxylabs`, then capturing with it. Its opaque ID is
returned as `data.session_id` in the NDJSON capture events. Omitting `--proxy`
uses Rel’s configured Session defaults. If creating the session exceeds the
configured maximum open-tab count, Rel permanently closes the oldest sessions;
their opaque IDs are returned as `data.closed_session_ids`.
For an existing session, omission preserves its current assignment; an explicit
proxy updates the assignment.

```sh
rel https://example.com/ --proxy=oxylabs
```

Canonical actions are:

```json
{"action":"click","selector":"button.more"}
{"action":"wait-for","selector":"#loaded-content"}
{"action":"wait","seconds":0.5}
{"action":"click-link","link":"https://example.com/more","match":{"type":"fuzzy-link","threshold":0.9}}
```

`wait-for` polls the live DOM until its CSS selector is present. It uses the
capture or page action's overall `--timeout` deadline, then continues with the
next action.

Function-style action strings and legacy action object shapes are rejected.
`--action` and `--actions` may be combined; actions execute in command-line
order.

A normal capture emits `capture.started`, `capture.browser_requested`,
`capture.page_ready`, `capture.rendered`, `capture.writing`, optional
`capture.retrying`, `capture.traffic`, `capture.completed` or
`capture.failed`, and finally `capture.finished`. The final event contains the
CLI exit code. When stdout is the destination, `capture.writing` and
`capture.completed` report `output_path:"-"`; the CLI's private staging path is
never exposed. A target website response such as HTTP 404 or 429 is reported
as `target_http_status`; it is not a Rel RPC error.

Example:

```sh
rel https://example.com \
  --output /tmp/example.html \
  --session-id machine-7b6063e7-f43b-447f-b3ea-9958c754fcec.Session12 \
  --action '{"action":"wait","seconds":0.5}'
```

## Attached pages

Attach an ephemeral automation page:

```sh
rel page attach https://example.com \
  --session-id machine-7b6063e7-f43b-447f-b3ea-9958c754fcec.Session12 \
  --timeout 90 --wait 1
```

`page attach` accepts `--session-id`, `--proxy`, `--output`, `--timeout`, and
`--wait`. Its result contains a process-local `page.id`.

Perform one canonical action on that attachment:

```sh
rel page action page_... \
  --action '{"action":"click","selector":"button.more"}' \
  --output /tmp/after-click.html
```

`page action` requires exactly one `--action` and also accepts `--output`,
`--timeout`, and `--wait`. The page remains pinned to the session, URL, and
proxy selected by `page attach`. Page IDs disappear when the agent restarts.

## Proxies

Every proxy has a required, case-insensitively unique alias. The alias is its
only public identifier: use it for every CLI and RPC reference. Numeric database
IDs and UUIDs are not accepted or returned by the public API.

```sh
rel proxy list
rel proxy get office
rel proxy delete office
```

Create a proxy:

```sh
rel proxy create \
  --alias office \
  --upstream-host proxy.example.com \
  --upstream-port 8000 \
  --username account \
  --password secret
```

`--alias`, `--upstream-host`, and `--upstream-port` are required. The alias is
immutable. Optional write fields are `--username`, `--password`, `--oxylabs-enabled true|false`,
`--oxylabs-location-parameter cc|country|st`, and
`--oxylabs-location-value VALUE`.

Update only the named fields:

```sh
rel proxy update office --upstream-port 9000
rel proxy update office --clear-username --clear-password
rel proxy update office --clear-oxylabs-location
```

An update requires at least one mutable option. The clear options send explicit
JSON `null` values instead of omitting their fields.

Rotate the generated sticky session for an Oxylabs-enabled proxy:

```sh
rel proxy rotate office
```

## Sessions

Read and delete persistent browser sessions by their opaque session IDs:

```sh
rel session list
rel session get machine-7b6063e7-f43b-447f-b3ea-9958c754fcec.Session12
rel session delete machine-7b6063e7-f43b-447f-b3ea-9958c754fcec.Session12
```

Create a session:

```sh
rel session create \
  --name Research \
  --proxy office \
  --adblock-enabled true \
  --image-blocking-mode over_limit \
  --image-size-limit-kb 100
```

Every create option is optional. Omitted proxy and filtering options use the
Session defaults configured in Rel.app. Use `--direct` to force a direct
connection instead of the default proxy. `--image-blocking-mode` is `all` or
`over_limit`. `--id-only` changes successful output to the new opaque session ID
and a trailing newline instead of the JSON response envelope. Errors remain on
standard error with the ordinary nonzero exit status.

Rel keeps at most eight persistent sessions by default. The maximum is
configurable from 1 through 100 in **Settings → Session**. Creating beyond the
limit closes the oldest session and deletes its browsing data. The create
response includes those opaque IDs in `data.closed_session_ids`.

Partially update a session:

```sh
rel session update machine-7b6063e7-f43b-447f-b3ea-9958c754fcec.Session12 --name Research-2 --adblock-enabled false
rel session update machine-7b6063e7-f43b-447f-b3ea-9958c754fcec.Session12 --direct
```

`--direct` sends `proxy_alias:null`, selecting direct networking during creation
or clearing a proxy assignment during update. An update requires at least one
mutable option. Session name and filtering policy are mutable; the opaque
session ID is immutable.
