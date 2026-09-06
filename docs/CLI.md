# REL CLI

The recommended CLI is the `rel` binary bundled in `/Applications/REL.app` and
linked from a writable directory already in `PATH`. Use **REL → Settings… →
General → Install Command Line** to create the link. If no safe destination is
available, REL leaves the filesystem unchanged so you can create the link
manually. The matching public client can also be installed from source:

```sh
cargo install --git https://github.com/rel-me/rel-tools \
  --tag v0.1.1 \
  --package rel-cli
```

The CLI is a thin client built on the typed [`rel-client`](SDK.md)
Rust crate. Ordinary user-facing commands map to [RPC v1](RPC.md) operations;
`rel read` and the profile/proxy transfer commands are bounded client-side
compositions of those operations. The standalone `rel-mcp` binary adapts a
focused subset of the same operations to stdio MCP; neither client implements
another browser or reads application data directly.

Related documents: [Actions](ACTIONS.md), [MCP](MCP.md), [SDK](SDK.md), and
[RPC](RPC.md).

## Commands

```text
rel health
rel status
rel navigate [URL] [options]
rel read [URL] [--query TEXT] [options]
rel perform ACTIONS [options]
rel capture [options]
rel URL [options]
rel capture URL [options]
rel page attach [URL] [options]
rel page action PAGE_ID --action JSON [options]
rel observe [--page-id ID] [--mode semantic|hybrid|visual] [options]
rel observation action OBSERVATION_ID --request JSON
rel proxy list
rel proxy get ALIAS
rel proxy create --alias ALIAS --upstream-host HOST --upstream-port PORT [options]
rel proxy update ALIAS [options]
rel proxy delete ALIAS
rel proxy rotate ALIAS
rel proxy export ALIAS [--output PATH]
rel proxy import FILE [--alias ALIAS]
rel profile list
rel profile export NAME [--output PATH]
rel profile import FILE [--name NAME]
rel session list
rel session get SESSION_ID
rel session create [options]
rel session update SESSION_ID [options]
rel session pause SESSION_ID
rel session play SESSION_ID
rel session delete SESSION_ID
rel session close --group GROUP
rel-mcp
rel --help | -h
rel --version
```

`rel --agent` exists only in the proprietary binary bundled with the REL app.
It is not part of the public `rel-cli` package.

`health` and `status` inspect the currently running agent without launching the
app. Every other ordinary CLI command, including proxy reads and session reads,
starts the REL app in the background when its agent is unavailable. `rel-mcp`
starts its stdio protocol server without launching the app; its operational
tools start REL lazily after their arguments have been validated.
`REL_AGENT_PORT` overrides the default local port, `17319`.

`rel navigate` and `rel page attach` use `REL_SESSION_URL` when their otherwise
required positional URL is omitted. An explicit URL always wins. Argument-free
`rel read` and `rel capture` retain their current-page behavior and do not
reload from `REL_SESSION_URL`.

When a browser command targets a session while REL is in the background, REL
selects that session by default without activating the app. Turn off **REL →
Settings… → General → Follow browser commands** to keep the current session
selected instead.
Internal session synchronization and read-only resource commands do not change
the selection.

REL Free supports one persistent Session and one custom Profile, and does not
allow proxy creation, configuration, assignment, or use. Mutations that require
REL Pro fail with the non-retryable `PRO_REQUIRED` RPC error. Register Pro in
**REL → Settings… → Plan** to remove these limits. Read and delete operations
remain available so stored resources can be inspected or reduced after a
downgrade.

Capture with a URL remains the default URL-first command: `rel URL [options]`.
The explicit `rel capture URL [options]` form is equivalent. Argument-free
`rel capture` instead captures the current shorthand page selected by
`rel navigate`. The removed `ping`, `logs`, and
`--rotate-proxy-session` interfaces have no compatibility aliases; use
`status`, the app's Logs view, and `proxy rotate`, respectively.

`rel observe` returns bounded rendered semantics and observation-scoped element
refs. `--mode=hybrid` adds a synchronized viewport PNG resource; `visual`
returns minimal semantics plus the image. `--page-id` targets an attached page,
while the default uses the current shorthand page and accepts `--session-id`.
Use a returned ref without inventing a selector:

```sh
rel observe --mode=semantic
rel observation action OBSERVATION_ID \
  --request='{"ref":"e1","action":"click","mode":"semantic"}'
```

The action response contains a new observation. Old observation refs fail with
`OBSERVATION_STALE` after navigation, document replacement, or bounded eviction.

`rel read` is the smaller retrieval path for reading and research. It observes
the current shorthand page, or navigates first when `URL` is supplied, then
returns query-ranked content and links as bounded Markdown. A compact heading
outline covers the page, and an unqueried read samples across long documents
instead of stopping at their first sections. It is always
semantic-only and does not return action refs or an image. `--max-chars`
defaults to 12000 (range 512–32768), and `--max-sections` defaults to 24 (range
1–100). Use `rel observe` instead when interaction refs or visual verification
are needed:

```sh
rel read https://example.com/docs --query="installation" --max-chars=6000
rel read --session-id=Session1 --query="current plan"
```

The JSON envelope reports the source URL, title, observation ID, outline and
selection counts, available content and link counts, whether the query matched,
and independent source/output truncation.

## Quick examples

### Like curl, but from a real browser

Give REL a URL to load it in embedded Chromium and write the rendered HTML to
standard output:

```sh
rel https://rel.me
```

It follows the same output model as curl: page data goes to standard output and
capture events go to standard error. Redirect either stream independently when
saving them is more useful:

```sh
rel https://rel.me > rel.html 2> capture.ndjson
```

### Navigate, perform, and capture in one session

Create a session, then let later CLI calls select that newest session for the
complete stateful workflow:

```sh
rel session create --name Research --id-only
# Session12

rel navigate https://rel.me
rel perform '[{"action":"wait","seconds":0.5}]'
rel capture > rel.html
```

The final argument-free `rel capture` reads the page selected by `rel navigate`
after `rel perform` finishes. Each CLI process asks REL for the newest existing
session, so this sequence needs no shell variable or CLI state file.

To pin a sequence while another shell or process may create sessions, export an
ID and optional default URL under REL's standard environment variables:

```sh
session_id="$(rel session create --name Research --id-only)"
export REL_SESSION_ID="$session_id"
export REL_SESSION_URL="https://example.com"

rel navigate
rel perform '[{"action":"wait","seconds":0.5}]'
rel capture > example.html
```

An explicit `--session-id` takes precedence over `REL_SESSION_ID`, which takes
precedence over the newest existing session. An explicit URL takes precedence
over `REL_SESSION_URL`. REL's embedded terminal exports both variables for its
browser session when the shell starts or restarts.

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
Chromium work; the REL app, agent, and persistent browser session remain
available for later commands.

`rel-mcp` is a protocol process rather than an ordinary one-shot command. Its
standard output contains only newline-delimited JSON-RPC 2.0 messages for the
MCP client, and diagnostics use standard error. See [MCP](MCP.md) for its
version negotiation, tool results, and errors.

An RPC failure is printed to standard error as the complete structured error
envelope. Argument, transport, and protocol failures are plain text on standard
error. Clients should branch on the high numeric `error.code` or stable
`error.id`, not `error.message` or the HTTP status.

Exit status is:

| Status | Meaning |
| --- | --- |
| `0` | Help/version or a successful RPC operation. `status` also requires `overall_status:"ok"`. |
| `1` | Usage, transport, protocol, or RPC failure; unhealthy `status`; or the terminal exit code from an unsuccessful capture. |
| `130` | The shell terminated the CLI with Ctrl-C (`SIGINT`). |

For `rel-mcp`, clean stdin closure exits successfully; startup or stdio failure
exits unsuccessfully.

## MCP server

```sh
/Applications/REL.app/Contents/Resources/rel-mcp
```

The adapter accepts no MCP options; `--help` and `--version` are available for
direct inspection. MCP clients normally launch it and own its
stdin/stdout pipes rather than running it in an interactive terminal. It
supports current `2026-07-28` discovery and legacy initialization through
`2025-11-25`, and exposes exactly fourteen tools: `rel_status`,
`rel_notifications`, `rel_capture`,
`rel_page_attach`, `rel_navigate`, `rel_read`, `rel_page_action`, `rel_take_screenshot`,
`rel_observe`, `rel_find`, `rel_action`, `rel_list_sessions`,
`rel_close_session_group`, and `rel_list_proxies`.

Every tool forwards through `rel-client` and RPC v1. Capture aggregates its
validated NDJSON stream into `{request_id, exit_code, events}`. Tool results
normally include their complete JSON in both a text content block and
`structuredContent`; `rel_read` puts bounded Markdown only in text and keeps
metadata in `structuredContent` to avoid duplicating page content. Captured files use absolute `file:///` URIs at the MCP
boundary and are also returned as standard MCP `resource_link` content blocks.
Screenshot calls without an explicit output URI and hybrid or visual
observations additionally return standard MCP `image` content for multimodal
agents.

## Health and status

```sh
rel health
rel status
```

`health` calls `GET /v1/health` and reports agent worker readiness. `status`
calls `GET /v1/status` and reports the app, agent, Browser Proxy, and Chromium
bridge checks. Both responses include the installed build identity when the
agent was launched by a metadata-bearing app bundle. Neither command
synthesizes a local process report when the agent is unavailable.

## Shorthand page workflow

For sequential automation that does not need to manage page or session IDs:

```sh
rel navigate https://example.com
rel perform '[{"action":"wait-for","selector":"button.more"},{"action":"click","selector":"button.more"},{"action":"wait","seconds":0.5}]'
rel capture > final.html
```

`navigate` calls `POST /v1/navigate`, navigates the current shorthand page, and
prints the ordinary JSON response envelope. Its first call reuses the newest
existing session unless `--session-id` or `REL_SESSION_ID` supplies one,
creating a session only when none exists. Later calls without a session ID
resolve the newest session again and reuse its current page. Supplying
`--profile NAME` intentionally creates a new session from that profile and
conflicts with `--session-id`. It also accepts `--proxy`, `--output`,
`--timeout`, and `--wait`. The positional URL may be omitted when
`REL_SESSION_URL` is set; an explicit URL always wins.

Navigation becomes ready after REL observes the requested HTTP(S) main-frame
load, that main frame finishes, and its rendered source is available. Subframe
and page-initiated background loading does not delay completion. `--wait`
applies a bounded settling delay after main-frame readiness; if another
main-frame navigation starts during that delay, REL waits for it and restarts
the delay. Use a timed `wait` action when the workflow needs additional
settling time before its next step.

A submitted navigation replaces the session's active target immediately, even
if the proxy or browser fails before the page loads. Refresh and retry use that
target, so a second failed navigation does not send the session back to the
first URL. Unsubmitted address-field edits do not change the retry target.

Fragment-only navigation and same-document Back/Forward complete using the
existing document. They preserve its HTTP status without requiring a full load.

If the main frame returns HTTP 4xx or 5xx, `navigate` normally exits
unsuccessfully as soon as Chromium commits that response instead of waiting for
all background loading to stop. By default, REL first detects Cloudflare
Turnstile and managed challenge pages and gives them up to 15 seconds to
continue. A successful redirect completes normally; otherwise the original
target error is returned. Turn off **REL → Settings… → General → Wait for
Cloudflare Turnstile** to restore immediate failure for every HTTP error. The
`UPSTREAM_UNAVAILABLE` error message and details include the exact target status
and final URL. The rendered page remains selected in the session.

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
to `REL_SESSION_ID` when set, then the newest existing session. The agent keeps
a distinct current shorthand page for each session, so embedded terminals can
use these commands concurrently. An explicit option always wins. The shorthand
registry is process-local and a session's entry disappears when the agent
restarts or that session closes. Concurrent workflows that may create sessions
should set `REL_SESSION_ID` or pass `--session-id`; concurrent work within the
same session should use `page attach` and `page action` with explicit page IDs.

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
| `--wait SECONDS` | `wait` | Nonnegative finite settling delay after the final main-frame readiness; default `1`. Background loading does not restart it. |
| `--action JSON` | `actions[]` | One canonical action object; repeat the option for multiple actions. |
| `--actions JSON` | `actions` | A JSON array of canonical action objects, executed in order. |
| `--session-id ID` | `session_id` | Reuse an existing immutable `Session<number>` ID. When omitted, use `REL_SESSION_ID` if set, then the newest existing session. Create a persistent session only when none exists. |
| `--profile NAME` | `profile` | Create the session from this built-in or custom profile. Conflicts with `--session-id` and suppresses both implicit defaults. |
| `--group GROUP` | `group` | Label a newly created URL-capture session. Conflicts with `--session-id` and suppresses both implicit defaults. |
| `--proxy ALIAS` | `proxy` | Select a proxy by its unique alias for the created or reused session. |
| `--retry COUNT` | `retry` | Retry count from 0 through 100; default `1`. |
| `--retry-delay SECONDS` | `retry_delay` | Finite delay from 0 through 86400 seconds; default `3`. |

Exactly one URL is required before the options. Scheme-less localhost
addresses use HTTP; other scheme-less hosts use HTTPS. Only HTTP and HTTPS are
accepted.

When `--session-id`, `--profile`, and `--group` are omitted, the CLI uses
`REL_SESSION_ID` if it is set, then the newest existing session. The environment
variable is exported automatically by each embedded session terminal. An
explicit option always wins. If neither the environment variable nor an
existing session is present, capture creates a persistent browser session. Its
default label is `Session<ID>` and its immutable identifier is:

```text
Session<ID>
```

For a new session, `--proxy oxylabs` is shorthand for creating a persistent
session assigned to `oxylabs`, then capturing with it. Its canonical ID is
returned as `data.session_id` in the NDJSON capture events. Omitting `--proxy`
uses the selected profile, or the built-in **Default** profile when
`--profile` is omitted.
For an existing session, omission preserves its current assignment; an explicit
proxy updates the assignment.

Sessions controlled while not visible use the **Background Browser Size** preset in
**REL → Settings… → General**. The default viewport is 1,920 × 947 CSS pixels,
matching a common maximized browser on a 1,920 × 1,080 display. A visible
session follows the resizable REL window. This is a global app setting, not a
capture option.

```sh
rel https://example.com/ --proxy=oxylabs
```

Capture accepts the canonical objects in the [Actions reference](ACTIONS.md).
`--action` and `--actions` may be combined; actions execute in command-line
order and stop at the first failure.

A normal capture emits `capture.started`, `capture.browser_requested`,
`capture.page_ready`, `capture.rendered`, `capture.writing`, optional
`capture.retrying`, `capture.traffic`, `capture.completed` or
`capture.failed`, and finally `capture.finished`. The final event contains the
CLI exit code. When stdout is the destination, `capture.writing` and
`capture.completed` report `output_path:"-"`; the CLI's private staging path is
never exposed. A target website response such as HTTP 404 or 429 is reported
as `target_http_status`; it is not a REL RPC error.

Example:

```sh
rel https://example.com \
  --output /tmp/example.html \
  --session-id Session12 \
  --action '{"action":"wait","seconds":0.5}'
```

## Attached pages

Attach an ephemeral automation page:

```sh
rel page attach https://example.com \
  --profile AdBlock \
  --timeout 90 --wait 1
```

`page attach` accepts `--session-id`, `--profile`, `--proxy`, `--output`,
`--timeout`, and `--wait`. It also accepts `--group` when creating a session.
`--profile` conflicts with `--session-id`; either creation option suppresses
both implicit defaults. Otherwise the CLI uses `REL_SESSION_ID` when set, then
the newest existing session. Its positional URL defaults to `REL_SESSION_URL`
when omitted. Its result contains a process-local `page.id`.

Perform one canonical [browser action](ACTIONS.md) on that attachment:

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

Creating, updating, rotating, assigning, or using a proxy requires REL Pro.
REL Free can list, inspect, and delete proxies that were saved previously.

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

Export or import a proxy transfer file:

```sh
rel proxy export office
rel proxy import office.relproxy
rel proxy import office.relproxy --alias office-backup
```

Export writes `ALIAS.relproxy` in the current directory unless `--output`
supplies an exact path. It refuses to overwrite an existing file and creates
the file with owner-only permissions. Stored proxy passwords are protected by
the app and are not available to CLI export, so `credentials_included` is
`false`. Import accepts an unprotected, versioned `.relproxy` SQLite archive;
`--alias` overrides the immutable alias stored in the file. Use the app to
export or import credentials with a transfer passphrase.

## Profiles

List profiles or transfer their reusable settings:

```sh
rel profile list
rel profile export Research
rel profile import Research.relprofile
rel profile import Research.relprofile --name "Research Copy"
```

Export writes `NAME.relprofile` in the current directory unless `--output`
supplies an exact path. It refuses to overwrite an existing file and creates
the file with owner-only permissions. A profile transfer contains its name,
referenced Proxy configuration, AdBlock setting, and image policy. CLI export
does not include cookies, saved passwords, or Proxy credentials. CLI import
accepts an unprotected archive and creates a custom Profile; `--name` overrides
the stored name when that name already exists. Use the app for
passphrase-protected browser data or Proxy credentials.

Both transfer formats are SQLite containers with the same version 1 schema.
They are limited to 12 MiB. Legacy JSON transfer files are not accepted.

## Sessions

Read, control, and delete persistent browser sessions by their canonical IDs:

```sh
rel session list
rel session get Session12
rel session pause Session12
rel session play Session12
rel session delete Session12
```

Create a session:

```sh
rel session create \
  --name Research \
  --group pgm \
  --profile BandwidthSaver \
  --proxy office \
  --adblock-enabled true \
  --image-blocking-mode over_limit \
  --image-size-limit-kb 100
```

Every create option is optional. `--profile` accepts the unique name shown in
**REL → Settings… → Profiles**; omission uses **Default**. Omitted proxy and
filtering options use the selected profile. Use `--direct` to override it with
a direct connection. `--image-blocking-mode` is `none`, `all`, or
`over_limit`; `none` allows every image without changing AdBlock.
`--group` labels the new session without changing its unique name or canonical
ID. Group matching is case-insensitive.
`--id-only` changes successful output to the new canonical
session ID and a trailing newline instead of the JSON response envelope. Errors
remain on standard error with the ordinary nonzero exit status.

After a successful create, that session is the newest session and therefore the
default for later CLI commands. Creating or deleting sessions in another shell
can change this default; use `REL_SESSION_ID` or `--session-id` to pin concurrent
workflows.

REL does not impose a maximum session count. Sessions remain open until you
explicitly delete them.

`pause` idempotently cancels active requests and blocks new network work for
the session. `play` idempotently resumes network activity and reloads the
current page when the pause interrupted or deferred navigation. Both commands
return the RPC envelope with `data.session_id` and `data.network_paused`.

Close every session in a group. Repeating the command after the group is empty
succeeds and returns an empty `data.deleted_ids` array:

```sh
rel session close --group pgm
```

Partially update a session:

```sh
rel session update Session12 --name Research-2 --adblock-enabled false
rel session update Session12 --direct
```

`--direct` sends `proxy_alias:null`, selecting direct networking during creation
or clearing a proxy assignment during update. An update requires at least one
mutable option. Session name and filtering policy are mutable; the canonical
session ID is immutable.
