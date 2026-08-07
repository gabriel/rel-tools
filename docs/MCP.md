# Rel MCP server

Rel includes a local Model Context Protocol server for agents that support MCP.
Run `rel mcp` as a stdio subprocess; the adapter exposes a focused set of
browser tools and forwards every tool call through the public
[`rel-client`](SDK.md) crate and [RPC v1](RPC.md). It does not read SQLite,
logs, or Chromium state directly.

Related documents: [CLI](CLI.md), [RPC](RPC.md), and [Rust SDK](SDK.md).

## Configure an MCP client

Use the CLI bundled in the installed app or install the public `rel-cli`
package as described in the [CLI guide](CLI.md). An absolute path to the bundled
binary is the most reliable choice for GUI clients that do not inherit an
interactive shell's `PATH`:

```json
{
  "mcpServers": {
    "rel": {
      "command": "/Applications/Rel.app/Contents/Resources/rel",
      "args": ["mcp"]
    }
  }
}
```

For clients that use TOML configuration:

```toml
[mcp_servers.rel]
command = "/Applications/Rel.app/Contents/Resources/rel"
args = ["mcp"]
```

The Settings command-line task creates `rel` only in a writable directory
already in `PATH`, but an MCP host may use a different process environment.

### Codex

The Codex desktop app, CLI, and IDE extension use the same MCP configuration on
one machine. The most direct desktop setup is:

1. Open **Codex Settings → MCP servers**.
2. Add a STDIO server named `rel`.
3. Set the command to
   `/Applications/Rel.app/Contents/Resources/rel` and add `mcp` as its only
   argument.
4. Save the server and restart Codex.

The equivalent global entry in `~/.codex/config.toml` is:

```toml
[mcp_servers.rel]
command = "/Applications/Rel.app/Contents/Resources/rel"
args = ["mcp"]
```

Use `.codex/config.toml` in a trusted project instead when Rel should only be
available in that project. If the `codex` command is installed in the shell's
`PATH`, it can create and inspect the same configuration:

```sh
codex mcp add rel -- /Applications/Rel.app/Contents/Resources/rel mcp
codex mcp list
```

After restarting Codex, start with a read-only prompt:

```text
Use the Rel MCP server. Call rel_status, then rel_list_sessions. Do not navigate anywhere.
```

Codex should discover the six tools listed below, and `rel_status` should report
the installed app, local agent, Browser Proxy, and embedded Chromium bridge.
In the Codex terminal UI, `/mcp` also shows configured servers and their tools.

For an end-to-end browser test, use:

```text
Use rel_capture to capture https://example.com and report the saved output URI.
```

Unlike the first check, this loads a website and saves rendered HTML. Omitting
`session_id` can also create a persistent Rel browser session.

### Direct protocol smoke test

An MCP host is not required to verify the adapter. This legacy handshake lists
the tools and calls the read-only status tool over the STDIO transport:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"rel-smoke-test","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rel_status","arguments":{}}}' \
  | /Applications/Rel.app/Contents/Resources/rel mcp
```

The server writes three JSON-RPC responses: initialization information, the
tool list, and the status result. Protocol messages use standard output;
diagnostics use standard error.

`rel mcp` accepts no options. At startup it checks the local agent and launches
Rel.app in the background once when needed. It then keeps serving its original
stdin/stdout connection until the MCP client closes stdin or the process is
terminated. `REL_AGENT_PORT` changes the loopback RPC port from its default,
`17319`.

Browser tool calls also use the [RPC tab-selection behavior](RPC.md#transport):
while Rel is inactive, their target tab is selected by default without bringing
the app forward. The General setting **Follow browser commands** controls this.

## Transport and protocol versions

The server uses the standard MCP stdio transport. Each input and output message
is one UTF-8 JSON-RPC 2.0 object on one physical line. Standard output is
reserved for protocol messages; diagnostics go to standard error. Notifications
do not receive responses.

Rel supports both MCP protocol eras used by current clients:

| Protocol revision | Connection flow |
| --- | --- |
| `2026-07-28` | The client calls `server/discover`; subsequent requests carry the current per-request MCP metadata. |
| `2024-11-05`, `2025-03-26`, `2025-06-18`, or `2025-11-25` | The client sends `initialize`, receives the selected legacy revision, then sends `notifications/initialized`. |

Discovery and initialization advertise only the `tools` capability. Rel does
not expose MCP resources or prompts, and its fixed tool list does not emit
list-changed notifications. `ping`, `tools/list`, and `tools/call` are available
after the client's protocol flow is established.

Tool calls run independently so a long capture does not block `ping` or other
stdio messages. `notifications/cancelled` suppresses the cancelled MCP result.
The current adapter keeps that tool's RPC connection open until its worker
finishes, so the notification alone does not stop browser work. Closing the MCP
client's stdin makes the adapter exit immediately. Process exit closes all
outstanding RPC connections and cancels the matching agent and Chromium
operations.

## Tools

The server exposes exactly six tools:

| Tool | RPC operation | Purpose |
| --- | --- | --- |
| `rel_status` | `GET /v1/status` | Read app, agent, Browser Proxy, and Chromium status. |
| `rel_capture` | `POST /v1/captures` | Load a page, perform optional actions, and save its rendered HTML. |
| `rel_page_attach` | `POST /v1/pages` | Attach an ephemeral automation page to a persistent browser session. |
| `rel_page_action` | `POST /v1/pages/{page_id}/actions` | Perform one action on an attached page. |
| `rel_list_sessions` | `GET /v1/sessions` | List persistent browser sessions and their opaque IDs. |
| `rel_list_proxies` | `GET /v1/proxies` | List configured proxy aliases and non-secret configuration. |

`rel_status`, `rel_list_sessions`, and `rel_list_proxies` accept an empty object.
The browser tools accept the same fields and validation rules as their RPC
operations:

### `rel_capture`

`url` is required. Optional fields are `output_uri`, `timeout`, `wait`, `actions`,
`session_id`, `proxy`, `retry`, and `retry_delay`. Omitting `session_id` creates
a persistent session using the configured Session defaults. The action objects
are the canonical `click`, `wait-for`, `wait`, and `click-link` shapes described
in [the CLI guide](CLI.md#capture). `output_uri`, when present, must be an
absolute local `file:///` URI.

### `rel_page_attach`

`url` is required. Optional fields are `session_id`, `proxy`, `output_uri`,
`timeout`, and `wait`. The result contains a process-local page ID for later
`rel_page_action` calls. Omitting `session_id` creates a persistent session and
navigates it to `url`. Providing `session_id` attaches its current page, whose
normalized URL must match `url`. `output_uri`, when present, must be an absolute
local `file:///` URI.

### `rel_page_action`

`page_id` and one canonical `action` object are required. Optional fields are
`output_uri`, `timeout`, and `wait`. The attached page remains pinned to the URL,
session, and proxy selected by `rel_page_attach`; page IDs expire when the agent
restarts. `output_uri`, when present, must be an absolute local `file:///` URI.

## Results and errors

Every tool execution result contains its complete JSON value in two forms:

- `content` contains a text block whose text is the serialized JSON;
- `structuredContent` contains the same value as structured JSON.

When a result contains captured HTML, every RPC `output_path` is exposed at the
MCP boundary as an absolute percent-encoded `output_uri`. `content` also includes
one standard MCP `resource_link` block per unique file, with `mimeType` set to
`text/html`. Rel deliberately keeps native filesystem paths inside RPC and uses
file URIs for MCP.

Status, page, session-list, and proxy-list tools preserve the ordinary RPC v1
success envelope with `status`, `request_id`, and `data`.

Capture consumes and validates the complete RPC NDJSON stream before returning.
Its structured result is:

```json
{
  "request_id": "req_...",
  "exit_code": 0,
  "events": [
    {
      "status": "ok",
      "request_id": "req_...",
      "event": "capture.completed",
      "data": {
        "output_uri": "file:///private/tmp/rel/captures/example.html"
      }
    }
  ]
}
```

The text block contains the serialization of this same object. A second content
block links the file directly:

```json
{
  "type": "resource_link",
  "uri": "file:///private/tmp/rel/captures/example.html",
  "name": "example.html",
  "mimeType": "text/html"
}
```

`events`
includes the terminal `capture.finished` event, and `exit_code` is taken from
that event. A target website status such as 404 remains capture data and can
produce exit code 1 and `isError:true`; it is not a Rel RPC or MCP protocol
error.

Malformed JSON-RPC messages, unsupported methods, and unknown tools use
JSON-RPC errors. Invalid arguments or another failure while executing a known
tool produce a tool result with `isError:true`, with the same complete error
JSON in its text and `structuredContent`. When the agent returned a structured
RPC error, that value preserves its high numeric code, stable error ID,
retryability, message, and optional details. Clients should branch on the code
or stable ID rather than parse the message.

## Runtime and trust boundary

The MCP process is a transient adapter owned by the MCP client. It is separate
from the app-supervised `rel --agent` process and from the private framed stdio
bridge between that agent and Rel.app. There is no MCP HTTP route and no second
browser backend.

The stdio connection is private to the launching MCP client, but forwarded RPC
is unauthenticated loopback traffic. Browser tools can create persistent
sessions, write capture files, navigate websites, and perform page actions that
have effects on those sites. MCP hosts should show tool calls for user review.
`rel_list_proxies` never returns stored proxy passwords.
