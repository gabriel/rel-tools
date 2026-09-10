# REL app

The macOS app owns REL's embedded Chromium runtime, persistent Sessions, browser
Profiles, and AI chat. Keep REL running whenever local clients or scheduled
prompts need to use it.

REL's embedded browser includes the Clark Browser and ungoogled-Chromium patch
sets. The privacy layer removes built-in Google service integrations and
blocks substituted background-service destinations. Websites you visit can
still load Google resources, and you can open Google pages explicitly.

REL configures Sessions to retain cookies, site storage, and saved logins when
it quits. The privacy layer does not enable automatic clearing on exit. This
preserves website login state; it does not enable Chromium's password manager
or guarantee that every sign-in flow is compatible.

The patch sets also remove Safe Browsing malware/download reputation checks,
automatic extension updates, browser Google account synchronization, and
Google-backed Web Push. Third-party cookie restrictions and disabled FedCM can
affect federated sign-in. The current ungoogled download patch also removes
macOS quarantine metadata. These are retained source-policy tradeoffs, not
just telemetry removal.

## Start on Login

Enable **Settings → General → Startup → Start on Login** to open REL
automatically when you log in to your Mac. REL uses the native macOS login item
registration for the app. Turn the setting off to remove that registration.

If macOS requires approval, click **Open Login Items Settings…** and allow REL.
The setting refreshes from macOS when you return to REL, including changes made
in System Settings. Registration errors appear below the Startup controls.

## AI provider presets

In **Providers → Add Provider**, choose **Fireworks**, **Amazon Bedrock**,
or **Baseten** to fill in an OpenAI-compatible endpoint. Enter that service's API
key, then add the provider. Keys are stored in macOS Keychain. REL discovers the
available models for the Chat picker.

| Preset | Default endpoint |
| --- | --- |
| Fireworks | `https://api.fireworks.ai/inference/v1` |
| Amazon Bedrock | `https://bedrock-mantle.us-east-1.api.aws/v1` |
| Baseten | `https://inference.baseten.co/v1` |

For Amazon Bedrock, change `us-east-1` to your AWS region as needed and use an
Amazon Bedrock API key. AWS access key IDs and secret access keys are not accepted
by this preset. Model availability depends on the endpoint, region, and account.
All three presets use the existing `openai-compatible` provider kind and Chat
Completions API. Endpoints remain editable for custom deployments.

See the provider setup references for [Fireworks](https://docs.fireworks.ai/tools-sdks/openai-compatibility),
[Amazon Bedrock](https://docs.aws.amazon.com/bedrock/latest/userguide/inference-chat-completions-mantle.html),
and [Baseten](https://docs.baseten.co/reference/inference-api/overview).

## Session and workspace errors

Click the warning icon in the main toolbar to open **Session and Workspace
Errors**. The sheet shows session and workspace persistence errors separately,
with scrollable, selectable details. **Copy Details** copies both error messages.

For session errors, use **Refresh Sessions** to reload the session list, then
retry the failed action. **Open Settings** lets you review session limits and
the default Profile. Refresh is available while the local agent is running and
no session refresh or save is in progress.

**Save Current Workspace** requests a save of the current tabs and layout for
the next launch; it does not restore a previous layout. When the agent rejects a
save as invalid, REL preserves the current draft and allows another save after
the problem is corrected. If the save outcome is uncertain or the workspace
revision has changed, REL blocks further writes until you restart. The button
cannot bypass that protection. **Report a Bug** opens the
report form for further help.

In Debug builds, **Debug → Error Recovery** can trigger a session error, a
workspace error, or both. These simulated errors appear in the same toolbar
warning and details sheet without changing sessions, files, or permissions.
Use **Refresh Sessions** and **Save Current Workspace** to exercise recovery.

## Keeping and deleting Sessions

Close the REL window or quit REL to keep Sessions and their saved logins for
next time. Session deletion cannot be undone.

**Delete Session…** in a Session's context menu and the Session tab's close
button show a confirmation before deleting anything. **Close Session…** (⌘W)
shows the same confirmation when no browser popup is open; when a popup is
open, it closes only that popup. The confirmation names the Session and warns
that deletion removes its cookies, saved logins, saved passwords, and other
browser data. **Cancel** is the default action and keeps the Session intact.

**Settings → General → Browser Data → Delete All Sessions…** also requires
confirmation and warns that all Sessions and their browser data will be deleted.
These confirmations apply to app controls; API and CLI deletion operations
remain explicit destructive operations without an interactive confirmation.

## Database migration and recovery

REL validates its local database before starting normal service. Supported
schema versions 3 through 18 are supported, with older schemas upgraded to schema 18. Before any upgrade or
repair, REL creates a consistent SQLite snapshot including committed WAL data
under `Data/Recovery/<run-id>/original.sqlite3` in its Application Support
folder. Debug builds use their isolated worktree Application Support folder.
Legacy root-level databases and schemas older than version 3 are not imported.

Migration runs against a separate candidate. If a supported migration or data
validation fails, REL rebuilds the current schema and copies valid records.
Invalid optional descriptive metadata can be cleared independently. Records
that cannot be converted safely remain in the original snapshot and are omitted
from the active database. Sessions and Profiles referencing an unrecoverable
proxy are withheld as well; recovery never changes that assignment to a direct
connection. Invalid fingerprint settings withhold the affected record rather
than silently changing its browser identity. Valid sessions remain available.

REL checks types, application decoding, schema structure, and references before
committing the candidate. Activation is one SQLite transaction: interruption
leaves the original database or the complete upgraded database. A concurrent
writer causes recovery to stop so its changes are not overwritten. Restarting
retries from the current data. A successful recovery is not repeated on every
launch.

When records or fields need review, REL displays a recovery notice. Choose
**Show Report**, or **Settings → Service → Show Recovery Report**, to locate
`report.json`. It lists retained counts and affected tables, record IDs, fields,
and reasons. Original values remain in the snapshot. The database's recovery
history identifies committed runs; a folder left by an interrupted attempt is
not proof that its candidate was activated. Recovery does not access or export
Keychain credentials.

Chromium session folders, cookies, and saved logins are preserved. A durable
`Data/Recovery/preserve-browser-storage` marker prevents automatic orphan-folder
cleanup after recovery, including on subsequent launches. Existing session IDs
are reserved so new sessions cannot inherit withheld sessions' storage. Keep the
marker and original snapshot while reviewing recovery. Quarantined records are
not automatically restored; use the original snapshot for diagnosis and careful
repair. Recovery does not manufacture missing records or browser data.

Storage, permission, locking, unreadable database pages, and backup failures
stop recovery without resetting the database. A database from a newer REL build
requires updating REL and is never downgraded. The app remains open with the
service error and **Retry Local Service**. Correct the reported condition and
retry. The service only becomes ready after validation; the supervisor allows
startup backup and recovery to finish instead of restarting them after its
normal health-poll threshold.

## Anonymous diagnostics

On the first normal startup, REL asks whether to share anonymous app usage and
reliability events. Diagnostics remain off unless you select **Share
Diagnostics**. You can change the choice later under **REL → Settings… →
General → Diagnostics**.

The fixed event schema includes app and macOS versions, launch and update
outcomes, agent availability, and the number of open Sessions. Events use a
random identifier that lasts only for the current app launch. They do not
include an account or persistent installation ID, URLs, page content, prompts,
Profile names, credentials, or local logs. Delivery is best effort and failed
events are not stored for retry.

## Free and Pro

REL Free does not require registration. It includes one Session at a time, one
scheduled prompt, one custom Profile, and one configured AI model provider.
Proxies cannot be created, configured, assigned, or used on the Free plan.

REL Pro costs $20 as a single upfront payment for one year of access. It does
not renew automatically. Register the license in **REL → Settings… → Plan** to
use proxies and remove the Free plan limits. If Pro registration expires or is
removed, REL preserves existing Sessions and configuration instead of deleting
them. Free prevents additional creation beyond its limits, and any stored proxy
assignment runs as a direct connection until Pro access is restored.

You can also enter a `REL-PRO-...` promo code in the same Plan field when one
has been provided to you. Promo codes grant one, two, or three calendar months
of REL Pro without a checkout or payment method. Each trial can be redeemed on
one REL installation, and a campaign code stops working after its configured
number of redemptions.

REL displays the trial end date in Plan settings. It checks the grant with REL
at most once per day and supports up to seven days offline, without extending
access beyond that end date. At expiry, REL automatically returns to Free and
keeps existing Sessions and configuration under the Free plan limits described
above.

## Profiles and Sessions

A **Profile** is a reusable template for a new Session. Profiles select the
connection, network filters, and any browser data that should be copied when a
Session is created. A **Session** is the persistent browser created from that
template; later Profile changes do not modify existing Sessions.

Manage saved configurations in **Profiles**. There are no built-in Profiles.
Profiles can use a configured proxy and imported cookies or passwords. The Profiles
list includes a **Browser Identity** column showing Private, Custom Privacy,
or Native.

In **New Profile**, choose **Proxy → New Proxy…** to add a proxy without leaving
the profile draft. Saving selects the new proxy automatically. Cancelling returns
to the draft without changing its proxy selection. A saved proxy remains available
in Proxies even if you later cancel the profile.

**Create Session**, **New Session** (Command-T), and the session tab bar’s plus
button create a session immediately using the configured default Profile, or
Custom defaults when none is set. You can change AdBlock, image blocking, Proxy,
and Browser Identity afterward. Changing Browser Identity shows a banner so you can reload when ready.
Browser data is copied or imported rather than switched as a setting.

Use the session toolbar's **Proxy** menu to select a saved proxy, or **None** for
a direct connection. Saved proxies from earlier REL versions remain selectable
without recreating them or enabling provider-specific sticky sessions.
A proxy can enable either Oxylabs or Bright Data session handling. Changing
its active provider renews the assigned sessions' sticky IDs; editing targeting
within the same provider preserves them. Unrelated edits and transfers retain
saved targeting settings for disabled providers.
REL upgrades stored proxy settings automatically when the updated app starts.
The upgrade preserves saved proxies, sessions, sticky IDs, and login data; it
does not require recreating proxies or importing them again. Earlier app versions
cannot open the upgraded database.

The toolbar **(+) → New Session from Profile** submenu lists saved Profiles.
Selecting a Profile creates a session immediately with its settings and browser
data. The submenu appears only when saved Profiles exist.

Use **File → Create Session from Profile** (Option-Command-T) to choose settings
before creation or copy a saved Profile’s browser data. This form starts with
**Custom** and shows the Profile picker only
when saved Profiles exist. Selecting one loads its configuration into the form;
all settings remain editable and changes apply only to the new Session. AdBlock,
Browser Identity, Proxy, Image Blocking, and Browser Data share one section
with equal-height setting rows. Choosing **Custom Privacy** in Create Session
opens a separate editor; **Use Identity** applies it to the draft and **Cancel**
preserves the previous identity. **Edit** reopens a custom identity.
**Show Config** below the section opens a read-only popover with the session
settings and all browser privacy controls, including controls left native. Proxy
uses the same dropdown style as the other settings.
New Custom drafts start with **Allow all images**. **Proxy → New Proxy…** creates
and selects a proxy without losing the draft. Cancelling keeps the current selection.

**Settings → General → Default Profile** controls immediate session creation
in the app and clients that omit a profile, including CLI, SDK, MCP, and Python. With no saved default, they use Custom:
direct networking, AdBlock on, all images allowed, and Private. The creation
form uses its explicit settings and browser-data choice instead. **None** does
not inherit another Profile’s browser data. Renaming a saved default preserves
its selection; deleting it requires choosing another default or Custom.
Changing this preference restarts the local agent and preserves existing Sessions.

Schedules that referenced former built-in Profiles keep their settings as explicit
Custom session configurations. New schedules can create a Custom session without
requiring a saved Profile.

## Browser identity

New Sessions use the form’s **Browser Identity**. New Custom configurations
and Profile drafts use **Private**, which enables all
seven supported privacy controls. In Profile forms, **Show** to the left of its
value opens a read-only popover without expanding the form. Create Session uses
**Show Config** below the main section instead. In Profile forms, Browser Identity is in the
main section. Choose **New Browser Identity…** in its dropdown to customize the
current settings in a separate editor. **Use Identity** applies them to the draft
as **Custom Privacy**; **Cancel** leaves the previous identity unchanged. These
settings are saved with the Profile. Use **Edit** beside Custom Privacy to change
them later. Starting from **Native** leaves every override off, so you can enable
only the controls you need. **Native** uses Chromium's native values.

For identities with overrides, every creation path generates a fresh numeric
readback seed for the Session, then keeps it stable for that Session. Profile edits apply to future Sessions.
Use a Session's tab menu to change its identity; saving recreates only that
Session's Chromium context and returns it to the same page.

**Private** uses **Automatic** for language and locale. REL selects the default
language for the proxy's configured country using macOS locale data and keeps
that country as the locale's region. Bright Data country targeting and Oxylabs
country or US state targeting supply this location. For example, Germany uses
`de-DE`, Canada uses `en-CA`, and Belgium uses `nl-BE`. Without a configured
location, REL uses your macOS preferred language.

The proxy editor's **Detect Exit Locale** option is off by default. Enable it
to use the detected exit country instead of the configured target. For Automatic
language controls, REL requests `https://ipwho.is/` through the browser session's
agent-owned proxy before preparing the browser. IPWHOIS.io sees the proxy's exit
IP. Successful results are cached for up to 30 minutes per session and upstream
route; a provider session rotation changes that route. A failed lookup reports an
error rather than using a different locale. The option is preserved in proxy and
profile transfers; older transfers import with detection off.

A country does not identify every resident's preferred language. In Custom
Privacy, choose **Custom** in the Language row to set an explicit locale such as
`fr-CA`. Custom takes precedence over automatic selection and does not trigger a
lookup. Disabling the language control keeps native Chromium language and locale.
The former proxy-level manual locale is retained in storage and API responses,
but Automatic now uses country targeting or opt-in exit detection.

For HTTP clients, proxy create/update accepts `detect_exit_locale` (boolean,
default `false` on create and preserved when omitted on update). Proxy responses
include that setting. Session responses include `proxy_country` (configured ISO
country or null) and `proxy_detect_exit_locale`. With detection enabled,
`GET /v1/sessions/{id}/proxy-location` returns `{ "country": "DE" }` in the
standard response envelope, or an error if detection is disabled, the session has
no proxy, or the lookup fails. This lookup does not write a language to the proxy.

Privacy controls cover graphics, audio, device surfaces, language and locale,
time zone, network information, and the CPU thread count reported to pages.
When Network Information protection is enabled, JavaScript RTT/downlink and
opted-in HTTP RTT/Downlink hints use the same rounded session values.
Network measurements are not exposed through those hints. Disabling the control
retains native estimates. Sites must still opt in to receive the hints, and
Permissions Policy can suppress them. These reported values do not change actual
connection speed or route traffic through a proxy.

Chromium generates the User-Agent in every mode with its product version reduced
to `MAJOR.0.0.0` (for example, `Chrome/152.0.0.0`). The engine supplies its native
brand list and client hints; these are not editable. High-entropy client hints
can still expose the engine’s full version when requested by a site.

Graphics protection changes Canvas and WebGL readbacks together with the graphics identity and
makes WebGPU unavailable. Text geometry, native input, and other unlisted
surfaces remain native.

Native Chromium uses the same patched privacy layer. Its WebRTC default also
restricts non-proxied UDP connections. Audio protection makes small changes to
AudioBuffer's live sample arrays, which can also affect later playback.

An identity profile is a compatibility tool, not an anonymity guarantee. Its
seed is stable across sites in that Session, so sites may still correlate
visits. Network identity is also separate: use a Session proxy when traffic
must leave through another route. Proxied Sessions prevent WebRTC from using a
non-proxied UDP route, but REL does not turn a direct Session into a VPN.

## Navigation errors and retry

Navigation failures show a readable explanation and retain the original source
error. Proxy tunnel failures include the proxy name, upstream HTTP status line,
and available provider diagnostics such as `Proxy-Status` and Bright Data error
codes. For example, Bright Data's `403` / `policy_20000` restriction appears with
the provider's access-denied explanation instead of only Chromium's generic
connection error. Check the provider's policy or configuration before retrying
a persistent rejection.

Expand **Technical Details** for long diagnostics, or use **Copy Details** to
copy the explanation, full retained diagnostics, Chromium error, and requested
URL. Short errors are shown directly. Details remain selectable and the page
scrolls when needed. Credentials, authentication challenges, and cookies are
excluded from proxy diagnostics; retained text is bounded to 8,192 characters
with a truncation marker. Website-generated HTTP error documents remain visible
rather than being replaced by REL's failure page.

Submitting an address immediately makes it the Session's active URL. If the
page or proxy fails, the address field, Application panel, and **Try Again**
button refer to that request. After submitting a different address, refresh
retries the new URL even if it also fails. Typing without submitting does not
change the retry target.

Back and Forward work with history entries created within the same page.
Submitting an address that only changes its `#fragment` also keeps the current
document available without waiting for a full page reload.

Chromium's automatic retries keep the error visible until the page returns a
response. A browser startup failure can be retried with **Try Again**, refresh,
or a newly submitted address; REL recreates that Session's browser and keeps
the latest requested URL.

If AdBlock blocks the main page, REL shows **This Page Was Blocked** with the
requested URL and a filter explanation. Check **AdBlock** in the Session's
**Filters** panel before trying again; retrying with the same blocking rule still
blocks the page. Blocked scripts, images, or embedded frames remain filter log
events and do not mark the main page as failed.

### Proxy-provider AdBlock exclusions

In Sessions using a proxy, REL excludes known proxy-provider destinations and
their subdomains from AdBlock by default, including provider websites, APIs, gateways, and diagnostic URLs.
The maintained list covers Bright Data/Luminati, Oxylabs, Decodo/Smartproxy,
IPRoyal, Webshare, SOAX, and Rayobyte. For example, `geo.brdtest.com`,
`ip.oxylabs.io`, and `ip.decodo.com` can load with AdBlock enabled.

These exclusions apply only while a Session has a proxy configured, to main
pages and subresources, with cached or newly downloaded rules. Direct Sessions
use normal AdBlock rules. Removing a Session's proxy restores normal filtering;
assigning a proxy enables the exclusions again. Image blocking and image
size limits still apply. Unrelated requests from provider pages remain subject
to AdBlock, as do ordinary destinations reached through a proxy.

The provider-domain list is maintained with REL app updates. It does not discover every proxy domain
automatically; new provider domains need to be added to that list.

## Session logs

Open **Logs** in a Session's bottom panel to follow its activity. Logging runs
while the Session is active, even when the panel is closed, and works with both
direct and proxied connections.

- **Network → Requests** shows Chromium HTTP and HTTPS request results for pages,
  scripts, stylesheets, images, frames, and fetch/XHR traffic. Entries include
  the method, URL, HTTP status or failure, elapsed milliseconds, and received
  bytes. Redirects include their destination. Results appear when a request
  finishes, fails, or is canceled; an open stream appears when it ends.
- **Network → Filtered Requests** explains requests blocked by Session filters.
- **Chromium → Runtime** includes browser open/close, navigation starts, finishes
  and failures, Back, Forward, Reload, and network pause/resume activity.
- **Clients → Requests** includes browser operations and individual automation
  actions, with their completion or failure and elapsed time.

Chromium request and activity entries omit request/response bodies, headers,
entered text, URL credentials, and URL fragments. Query parameter names remain
visible with their values replaced by `REDACTED`. Proxy transport diagnostics
remain separate from Chromium request results, so a proxied request can have
both a transport entry and a browser result.

Use the category menu to filter the stream. **Clear Logs** clears only the
selected Session and live logging continues. Logs remain local to this app's
data directory.

Empty log views keep their table headers and show no placeholder, including when the category filter hides all entries. Log columns fit the available width, and a single Log Options menu in the lower-right corner contains category filters, Refresh Logs, and Clear Logs. A separate Snap to Bottom control appears only while you are reading earlier entries.

### Log record schema

The log inspector, copied records, and local NDJSON logs use the same flat JSON
object. NDJSON contains one object per line. There is no `data` wrapper, nested
object, or array. Optional fields are omitted when unavailable, never written as
`null`. All names use `snake_case`.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string, required | Opaque record ID, suitable for deduplication. |
| `created_at` | integer, required | UTC Unix timestamp in seconds. |
| `category` | string, required | `agent`, `chromium`, `chromium.profile`, `network`, `network.filtered`, or `proxy`. |
| `level` | string, required | Severity, normally `info`, `warn`, or `error`; diagnostic sources can also emit `trace`, `debug`, `warning`, or `fault`. |
| `message` | string, required | Short human-readable summary. Use structured fields for filtering and analysis instead of parsing this text. |
| `session_id` | string | The Session associated with the event. Omitted for unscoped records. This is the only Session identity field; there is no `browser_profile_id` or `tab_name`. |
| `event` | string | Machine-readable event name, such as `navigation_started`, `browser_request`, `browser_action`, or `agent_request`. Unstructured diagnostics may omit it. |
| `url` | string | Destination associated with the event. |
| `method` | string | HTTP method. |
| `path` | string | Local API request path. |
| `page_id` | string | Page handle associated with an API request. |
| `group` | string | Session group associated with an API request. |
| `operation` | string | Browser operation or action, such as `navigate`, `click`, or `type`. |
| `request_id` | string | Request or action correlation ID. Interpret within its Session and event source. |
| `status` | string | Outcome, such as `completed`, `failed`, `canceled`, or `redirect`. |
| `status_code` | integer | HTTP status code, when available. |
| `error_code` | integer | Nonzero Chromium/CEF error code, when available. |
| `resource_type` | string | Chromium resource classification, such as `document`, `script`, `image`, or `xhr`. |
| `reason` | string | Filter reason, such as `adblock`. |
| `duration_ms` | integer | Elapsed milliseconds. Zero is a valid measurement. |
| `received_bytes` | integer | Received byte count reported by Chromium. Zero is a valid measurement. |
| `redirect_url` | string | Redirect destination. |

Event-specific fields appear only when relevant. Additional scalar fields
(strings, numbers, or booleans) may be added; consumers should tolerate them.
Event fields cannot overwrite the required fields or `session_id`.

Navigation example:

```json
{"id":"e57b6b74-170c-4fcd-a9bf-e96a9af98448","created_at":1788762428,"category":"chromium","level":"info","session_id":"Session78","event":"navigation_started","message":"Navigation started: about:","url":"about:"}
```

Request result example:

```json
{"id":"b37c10ef-a84c-4e11-9ad8-4f531e96eaad","created_at":1788762429,"category":"network","level":"info","session_id":"Session78","event":"browser_request","message":"GET https://example.com/: HTTP 200","method":"GET","url":"https://example.com/","resource_type":"document","request_id":"1:42","status":"completed","status_code":200,"duration_ms":42,"received_bytes":1024}
```

Activity messages use `Summary: URL` when a URL is present. Request messages
use `METHOD URL: result`; timing, size, resource type, and redirect destination
remain in their own fields. Browser URLs retain the redaction described above.
Non-HTTP(S) URLs are reduced to their scheme, such as `about:` or `data:`, so
inline document content is never included.

This format replaces the nested log schema. Existing nested records are not
converted or supported by the new readers.

## Site permissions

Website permissions are stored by origin inside each Session's isolated
Chromium profile. A request for location, notifications, microphone, camera,
or clipboard access shows a browser-attached prompt with **Not Now**, **Don't
Allow**, and **Allow**. Not Now saves no decision. Don't Allow remains denied,
and Allow is reported only after Chromium stores the real permission.

Open the Session tab menu and choose **Site Permissions** to inspect the current
site. **Revoke** returns one capability to Chromium's default prompt state for
that origin and Session. Decisions do not move to another Session. Before a
microphone or camera allow decision, device enumeration hides device labels and
stable IDs.

## Settings configuration transfers

In **Settings → Profiles, Proxies, Schedules, or Providers**, the glass button
group contains **Add (+)**, **Edit**, a divider, **Import (down arrow)**, and
**Export (up arrow)**. Select a row to enable export. Import opens a text editor with a **Paste** button to insert the clipboard
contents; export shows selectable text with a **Copy** button. No file picker is involved.
Imports create new records and remain subject to the plan's creation limits.
Existing records are not overwritten.

Profiles, schedules, and providers use a versioned JSON envelope:
`{"format":"rel.<kind>","version":1,"configuration":{...}}`. The kind is
`profile`, `schedule`, or `provider`. Export produces one line; pasted JSON may
include whitespace. Paste the complete object, without Markdown fences. JSON
input is limited to 1 MiB. Other versions, mismatched kinds, malformed JSON,
and invalid configurations are rejected.

### Profiles: single-line JSON

```json
{"configuration":{"adblock_enabled":true,"fingerprint_profile":null,"image_blocking_mode":"none","image_size_limit_kb":40,"includes_cookies":false,"includes_passwords":false,"name":"Research","proxy_alias":null},"format":"rel.profile","version":1}
```

The configuration uses the profile-creation fields documented in the
[RPC guide](RPC.md). `name`, network filters, `fingerprint_profile`, and
`proxy_alias` are copied. A null fingerprint selects Native Chromium. Keep an
exported fingerprint object intact when preserving a configured identity.
`image_blocking_mode` is `none`, `all`, or `over_limit`; `image_size_limit_kb`
is an integer from 1 through 1048576.

Cookies, passwords, browser storage, and referenced proxy definitions are not
included. Both `includes_cookies` and `includes_passwords` must be false.
A non-null proxy alias must already exist on the importing device; import the
proxy first or set `proxy_alias` to null. The new profile receives a new ID.

### Proxies: curl command

```sh
curl --proxy 'http://proxy.example.com:8080' --proxy-user 'username:[replace with password]' 'https://example.com'
```

Import reads `--proxy` (`-x`) and optional `--proxy-user` (`-U`), including
quoted values and `--option=value` forms. The proxy must use HTTP with an
explicit port from 1 through 65535; bracket IPv6 addresses. Commands are limited
to 64 KiB. REL parses the command without executing it, expanding shell syntax,
or requesting the target URL. The populated proxy editor opens for review and
requires **Save** before creation.

Export includes the endpoint and configured username. Bright Data geographic
and ASN targeting and Oxylabs location targeting are encoded in the username.
Session suffixes are parsed as session templates, not reused as persistent
session IDs. Saved passwords are never retrieved for export. Replace
`[replace with password]` before running a copied command yourself; REL treats
that placeholder as an empty password on import. A pasted real password is
placed in the editor's password field and saved through REL's secure proxy
credential storage only when you save.

A curl transfer does not include the alias, locale override, or custom TLS CA
certificate. Review those fields in the editor. Recognized Bright Data endpoints
select Bright Data trust; other endpoints start with system trust. Options such
as `-k`, request headers, and the destination URL are not imported as proxy
settings. Use the archive API below if you need to preserve the complete proxy
configuration.

### Schedules: single-line JSON

```json
{"configuration":{"completionAction":{"type":"none"},"destination":{"existingSession":{"sessionID":"session-1","sessionName":"Work"}},"hour":9,"minute":30,"name":"Morning","prompt":"Check the page and report changes.","usesTimer":true,"weekdays":[2,3,4,5,6]},"format":"rel.schedule","version":1}
```

`name`, `prompt`, `destination`, `completionAction`, `weekdays`, `hour`, `minute`,
and `usesTimer` are required. Weekdays are 1 (Sunday) through 7 (Saturday), with
at least one day; hours are 0–23 and minutes 0–59 in the importing device's local
time zone. `usesTimer:false` creates a webhook-triggered schedule.

The destination is one of:

- `{"existingSession":{"sessionID":"...","sessionName":"..."}}`
- `{"newSession":{"profileID":"...","profileName":"..."}}`
- `{"configuredSession":{"settings":{...}}}`, preserving the exported custom
  session settings, including filters, proxy alias, and fingerprint draft.

Completion actions are `{"type":"none"}`, `{"type":"shortcut","name":"..."}`,
or `{"type":"webhook","id":"<UUID>"}`. Destination and completion references
are local to the importing device; their referenced sessions, profiles, proxies,
webhooks, and Shortcuts are not bundled. Review and repair them in the editor.

Every imported schedule receives a fresh ID and starts **disabled**, regardless
of the source schedule's state. Run history, errors, and timestamps are omitted.
Import never runs a prompt or completion action. Enable the schedule after
reviewing its destination, prompt, completion action, and local execution time.

### Providers: single-line JSON

```json
{"configuration":{"maxTurns":10,"name":"OpenAI","provider":"openai"},"format":"rel.provider","version":1}
```

Required fields are `name`, `provider`, and `maxTurns`; `baseURL` is optional
except for services that require a custom URL. Provider values are `openai`,
`openai-compatible`, `openrouter`, `anthropic`, `gemini`, and `ollama`. Provider
names start with an ASCII letter, contain only letters, digits, hyphens, or
underscores, and are at most 64 characters. Turn limits and URLs are validated
using the same rules as the provider editor.

Import opens a new provider draft for review. Enter the API key where required,
then save. API keys, record IDs, model discovery results, and the default-provider
preference are excluded. An existing default remains unchanged unless you choose
**Make Default**; the first provider becomes the default normally.

### CLI and RPC archive transfers

CLI/RPC `.relprofile` and `.relproxy` SQLite archive transfers remain separate
from Settings text transfers. See the [CLI](CLI.md) and [RPC](RPC.md) guides for
archive operations. Archives can preserve browser data, complete proxy settings,
and passphrase-protected credentials. The new Settings JSON envelope is not an
archive or a replacement input for the archive APIs.

## AI models

When Chat has no available model, select **Add Provider** in the empty state
or chat input to open the Add Provider form directly. Cancel returns to Chat.

Configure providers and choose the default AI model in **REL → Settings… →
Providers**. API keys are stored in macOS Keychain. Ollama connections can use
the local server at `http://127.0.0.1:11434` without an API key. Scheduled
prompts use the default provider and model when their new Session starts. REL
Free supports one configured provider; REL Pro supports multiple providers.

The Chat model picker uses the provider's display name when available, or the
exact model ID when no display name is supplied. This also applies to newly
discovered models. API requests always use the model ID.

Each Chat response stops after 12 model calls or a 64,000-token request budget.
REL uses the preceding model call's reported usage to avoid starting a call
that would predictably exceed the remaining budget. A retryable browser error
gets one recovery attempt. If the same error recurs through another tool or
argument set, REL removes browser tools for the rest of that response so the
model answers from collected evidence or explains the limitation. When an
exhaustive request exceeds a page or tool output bound, the response summarizes
the available evidence and states what was omitted.

## Chat restoration

REL saves each Session's open Chat tabs, their order and selection, conversation
messages, completed-work details, and unsent drafts in its local SQLite database.
Quitting and reopening REL restores them. Completed question-and-answer exchanges
are restored to the AI harness before you send a follow-up. An interrupted response
is not resumed automatically; its submitted prompt remains visible in the chat.
Database upgrades use transactional migrations. Existing workspace layout and token
usage are imported once from the current runtime’s old workspace file. If restoration
fails, REL reports the error and blocks replacement writes. A save failure preserves
the current draft in memory. Validation rejections allow another save; uncertain
save outcomes and revision conflicts require a restart before saving again.

Database recovery preserves healthy conversations and drafts. Damaged messages or
chats belonging to an unrecoverable Session remain in the original recovery snapshot
instead of the active workspace. Missing tab selections are cleared without discarding
healthy chats. The recovery report identifies affected records.

Closing a Chat tab removes its saved conversation and draft. Clearing a conversation
removes its saved messages. Deleting a Session removes its saved chats.

## Reading chat history

Chat follows new messages and activity while you are near the bottom. Scroll up
to read earlier messages without being pulled back down. Choose **Jump to latest**
to return to the newest content and resume following, or scroll back near the
bottom yourself.

## Global and Session chat

The right chat panel has two scopes:

- **Global** keeps a workspace conversation while you switch browser Sessions. It has separate history and drafts, and it can target any available Session. The header identifies it as **All sessions**. Name the intended Session when a request could apply to more than one.
- **Session** follows the selected browser Session. Its header shows that Session's name, and its browser tools remain pinned to that Session.

Switching scope opens the corresponding history; it never converts a conversation from one scope to the other. Use **Chat history** to reopen a conversation and **New Chat** to start one in the current scope. Histories, drafts, and the selected scope are saved in REL's local workspace database. Global chat is stored independently of browser Sessions.

Tab close buttons appear when you hover over a tab. With the native left sidebar open, its Session list replaces the horizontal Session tabs. The browser keeps a compact Session header aligned with the chat scope header. With the sidebar hidden, Session tabs return above the browser; the chat panel stays beside that column.

## Agent instructions and current-page context

Open **REL → Settings… → Agent** to edit the system prompt used by native Chat.
REL adds these instructions after its protected browser and tool rules, and
changes apply to the next message in existing chats.

Each **Session** chat turn also includes the current page URL from its attached
Session. Global chat receives the available Session names and IDs instead; switching the selected Session does not silently retarget it. The default system prompt uses that context for requests such as
“summarize this page” or “summarize the top 3 links”: it reads the current page,
identifies the requested links in page order, reads their destinations, and
then answers. Restoring the default prompt returns to this behavior.

## Actions

Open **Actions** from the toolbar or **Settings → Actions** to create reusable
work. Each Action owns its name, prompt, destination Session or Profile,
optional Shortcut or webhook completion behavior, and enabled state. Select
an Action to edit, run, or delete it. The list shows its status for the current
app launch. Disabling an Action pauses every trigger that uses it.

Schedules, incoming webhooks, and built-in browser events execute the same saved
Action. Editing an Action updates the work performed by all its triggers. REL
runs at most one execution per Action at a time, including manual runs.

Existing saved prompts become Actions with their original IDs, destinations,
and completion settings. Existing timers still reference those Actions;
webhook-only prompts appear in Actions without a schedule row. Existing webhook
routing IDs remain valid. Remove schedules and incoming webhooks referencing an
Action before deleting it.

### Built-in events

In the Action editor, enable **Page Changed** or **Notification Received** and
choose a **Source Session**. Both are off by default. Page Changed fires when
the source session's URL changes, including same-document URL changes. It does
not watch arbitrary DOM mutations or compare page contents. Notification
Received fires when that session displays an allowed website notification.
Website notification permissions still apply. Notification Actions are separate
from sharing notifications with the agent's recent-notifications feed.

Events pass the source session and URL or notification details as untrusted
data after the saved prompt. Event content cannot select the Action or its
completion destination. Events for other sessions are ignored. Events received
while the Action is running are skipped, and automatic events are limited to
one run per Action every 30 seconds to bound cascades. These events are live
and are not replayed after restarting REL.

## Scheduled prompts

Open **Schedules** and choose **Add Schedule**. Give it a name, select an Action,
and choose weekdays and a local time. Create the Action first in **Actions**.
Multiple schedules can select the same Action to run it at different times.
REL Free supports one schedule; REL Pro supports multiple schedules.

REL must be running when a schedule is due. It executes the selected Action
using the default AI model. Depending on the Action's destination, it uses an
existing Session or creates a persistent Session. The table shows the next run
and the outcome of the most recent scheduled or manual schedule run. Sessions
remain available for inspection after a failure.

Use **Run Now** to execute a schedule without changing its next repeating run.
Disable a schedule to pause its timer. A disabled or missing Action causes the
schedule run to fail clearly. If the Action is already running, the schedule
records that outcome and waits for its next normal time; it does not queue an
overlapping run. Times follow the Mac's current time zone.

## Notifications

Open **REL → Settings… → Notifications** in the Browser section to control
**Send notifications to the agent** and inspect recent shared website notifications.
Sharing is off by default. Websites must first receive permission to send
notifications. Sharing adds untrusted website data to the feed. To start a turn automatically,
enable **Notification Received** for an Action and choose its source session in
**Settings → Actions**.

The page refreshes automatically and shows up to 256 shared notifications, newest
first, with each notification's origin, title, body, session ID, and display time.
The Recent section appears only when shared notifications are available.
Turning sharing off stops new entries; existing entries remain until the local
agent restarts. The queue is not a permanent notification archive. Debug runtimes
with website notifications disabled show that status on the page.

## Webhooks

Open **REL → Settings… → Webhooks** to add a JSON webhook, Discord integration,
or WhatsApp Cloud API integration. A configuration can send messages, receive
events, or do both. Its URL and credentials are stored in the current REL app
variant's Keychain, separately from browser sessions. Settings can send an
explicit test message and delete a destination.

To deliver an Action's final response, edit it in **Actions** and choose
**Send Result to Webhook**. A completion action can use either a webhook or a
macOS Shortcut. Keep Discord results within 2,000 characters and WhatsApp text
results within 4,096 characters. Delivery errors mark the prompt run as failed;
REL does not automatically resend messages.

To execute an Action from an incoming event, create the Action first, then
select it under **Run an Action on incoming events** when adding the webhook.
No schedule is required. Keep the Action enabled. Incoming data is appended
as untrusted JSON; write the Action prompt to describe which fields to process.
REL runs one event at a time per Action and keeps events queued while the
Action is busy, disabled, or missing.

**Copy Local Callback** copies the loopback receive URL. External services need
a public HTTPS relay forwarding only that path. The [RPC webhook guide](RPC.md#webhooks)
documents signing, provider setup, callback responses, inbox limits, and direct
HTTP calls. The inbox holds up to 64 events and resets when REL quits; use a
separate durable relay if events must survive app restarts.

For Discord sending, paste the channel webhook URL. Incoming Discord Webhook
Events require the application's public key and event subscriptions in the
Developer Portal. These subscriptions are distinct from ordinary channel
messages delivered through the Discord Gateway. See the official
[Discord webhook reference](https://docs.discord.com/developers/resources/webhook)
and [Webhook Events setup](https://docs.discord.com/developers/events/webhook-events).

For WhatsApp sending, supply your versioned Graph API messages endpoint, access
token, and recipient. Incoming events require the Meta app secret and a
verification token; REL handles callback verification and ignores delivery
status receipts. Text messages require an open customer service window. For
messages outside that window, callers can send an approved template using the
RPC `payload` option. See Meta's
[WhatsApp Cloud API reference](https://www.postman.com/meta/whatsapp-business-platform/documentation/wlk6lh4/whatsapp-cloud-api).

## Browser configuration changes

Saving browser identity or proxy configuration leaves the current page running.
If a change requires a new browser context, REL shows a banner above the page:
**Browser configuration changed. Reload to apply.** Choose **Reload** when you
are ready. Saving alone does not reload the page, so unsaved form input remains
available until you reload. Reverting all pending context changes removes the
banner.

Changes to upstream routing apply to new connections; existing connections
continue until they close. Browser identity, proxy assignment, and certificate
trust changes that require a new context wait for Reload when there is page
state to preserve. An empty browser with no active page, popup, or navigation
history applies these changes automatically without a reload banner. Sessions
that have not opened a browser yet start with their latest configuration.

## Proxy certificate trust

In **Settings → Proxies**, create or edit a proxy and choose **HTTPS Certificates → Trust**:

- **System trust** uses Chromium's ordinary certificate verification and macOS trust. Existing proxies retain this setting.
- **Bright Data certificate** adds REL's bundled Bright Data root CA for `brd.superproxy.io:44445`. Creating a proxy with the Bright Data type preselects this option; an existing proxy requires an explicit change.
- **Custom certificate** imports a PEM bundle or DER CRT file. REL saves the certificate contents with the proxy, so the original file is no longer needed. PEM bundles may contain 1–16 CA certificates, up to 64 KiB; private keys and website leaf certificates are rejected.

Additional CAs are trusted only in REL sessions using that proxy. They permit the proxy provider to inspect those sessions' HTTPS traffic. Hostnames, expiry dates, and certificate chains remain checked for pages and subresources. REL never installs these roots in Keychain or disables TLS verification. Saving a certificate change shows the configuration banner in affected open browsers. Reload applies the new trust settings while preserving session storage. Switching to another proxy or a direct connection replaces or clears the additional roots.

CLI/RPC proxy and profile archives preserve certificate settings. Settings curl transfers omit custom certificates. The import sheet identifies transfers that add a trusted proxy CA. Older transfer versions import with system trust.
