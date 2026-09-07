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
and Browser Identity afterward. Changing Browser Identity reopens the session.
Browser data is copied or imported rather than switched as a setting.

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

**Private** uses **Automatic** for language and locale. REL resolves an
explicit Custom locale first, then the locale configured on the session's proxy,
then the macOS user's preferred/default locale. It applies a language/locale
override only when the resolved value differs from native Chromium. An enabled
language control can therefore leave native values untouched.

Set **Language and Locale** in a proxy's editor to associate a BCP-47 locale such
as `fr-CA` with that proxy. Leave it blank to use your user/default locale.
A country selection alone never picks a language, including in multilingual
countries. In Custom Privacy, choose **Automatic** or **Custom** in the Language
row; Custom exposes the explicit locale field. Disabling that control keeps
native language and locale regardless of proxy settings.

Privacy controls cover graphics, audio, device surfaces, language and locale,
time zone, network information, and the CPU thread count reported to pages.
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

## Profile and Proxy transfers

Use **Import Profile…** and **Export Profile…** in that settings tab to move a
template in a versioned `.relprofile` SQLite archive. An export can include the
Profile's cookies, saved passwords, referenced Proxy configuration, and saved
Proxy credentials. REL requires a transfer passphrase whenever any of those
secrets are selected. On import, REL previews the included data and asks for
the passphrase before creating the Profile and restoring its browser data.

Manage upstream connections in **REL → Settings… → Proxies**. **Import Proxy…**
and **Export Proxy…** read and write versioned `.relproxy` SQLite archives. An
export can omit credentials or include the saved username and password in
passphrase-protected form. Choose the credential-free option when the file
only needs to recreate routing settings.

Both file types use the same versioned SQLite schema: `metadata`, `proxies`,
`profiles`, `cookies`, and `passwords`. A standalone Proxy archive and a Proxy
embedded in a Profile archive use the identical `proxies` table. Secret values
are stored only as encrypted BLOBs; the passphrase is not written into the
file. Import creates a new Profile or Proxy and does not overwrite an existing
name or alias. Version 1 imports accept only this SQLite format; legacy JSON
transfers are not supported.

## AI models

Configure providers and choose the default AI model in **REL → Settings… →
Providers**. API keys are stored in macOS Keychain. Ollama connections can use
the local server at `http://127.0.0.1:11434` without an API key. Scheduled
prompts use the default provider and model when their new Session starts. REL
Free supports one configured provider; REL Pro supports multiple providers.

Each Chat response stops after 12 model calls or a 64,000-token request budget.
REL uses the preceding model call's reported usage to avoid starting a call
that would predictably exceed the remaining budget. A retryable browser error
gets one recovery attempt. If the same error recurs through another tool or
argument set, REL removes browser tools for the rest of that response so the
model answers from collected evidence or explains the limitation. When an
exhaustive request exceeds a page or tool output bound, the response summarizes
the available evidence and states what was omitted.

## Agent instructions and current-page context

Open **REL → Settings… → Agent** to edit the system prompt used by native Chat.
REL adds these instructions after its protected browser and tool rules, and
changes apply to the next message in existing chats.

Every native Chat turn also includes the current page URL from its attached
Session. The default system prompt uses that context for requests such as
“summarize this page” or “summarize the top 3 links”: it reads the current page,
identifies the requested links in page order, reads their destinations, and
then answers. Restoring the default prompt returns to this behavior.

## Scheduled prompts

Open **Schedules** to create saved prompts. Each schedule
contains:

- a name;
- the Profile used to create a fresh Session;
- the prompt that runs in that Session;
- an optional repeating timer with weekdays and local time;
- an optional Shortcut or webhook completion action; and
- an enabled or disabled state.

REL Free supports one saved schedule; REL Pro supports multiple schedules.

Create separate schedule rows when the same prompt should run at multiple times
on the selected days. Times follow the Mac's current time zone.

REL must be running when a schedule is due. At that time REL creates a new
persistent Session from the selected Profile, starts chat with the default AI
model, submits the prompt, and waits for the assistant response. The Scheduled
table shows the next run and whether the last run completed or failed. A failed
run leaves its new Session available for inspection.

Use **Run Now** to execute a schedule immediately without changing its next
repeating run. Disable a row to pause it without deleting its configuration.
If its Profile is later deleted, REL marks the Profile as missing and the
schedule cannot run until it is edited to select an available Profile.

## Webhooks

Open **REL → Settings… → Webhooks** to add a JSON webhook, Discord integration,
or WhatsApp Cloud API integration. A configuration can send messages, receive
events, or do both. Its URL and credentials are stored in the current REL app
variant's Keychain, separately from browser sessions. Settings can send an
explicit test message and delete a destination.

To deliver a prompt's final response, edit it in **Schedules** and choose
**Send Result to Webhook**. A completion action can use either a webhook or a
macOS Shortcut. Keep Discord results within 2,000 characters and WhatsApp text
results within 4,096 characters. Delivery errors mark the prompt run as failed;
REL does not automatically resend messages.

To run a prompt from an event, create the prompt first, then select it under
**Run a prompt on incoming events** when adding the webhook. Turn off **Run on a
schedule** in the prompt editor for webhook-only operation. Keep **Enabled** on.
Incoming data is appended to the run as untrusted JSON; write the saved prompt
to describe which fields it should process. REL runs one event at a time per
prompt and keeps events queued while the prompt is busy or disabled.

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

## Proxy certificate trust

In **Settings → Proxies**, create or edit a proxy and choose **HTTPS Certificates → Trust**:

- **System trust** uses Chromium's ordinary certificate verification and macOS trust. Existing proxies retain this setting.
- **Bright Data certificate** adds REL's bundled Bright Data root CA for `brd.superproxy.io:44445`. Creating a proxy with the Bright Data type preselects this option; an existing proxy requires an explicit change.
- **Custom certificate** imports a PEM bundle or DER CRT file. REL saves the certificate contents with the proxy, so the original file is no longer needed. PEM bundles may contain 1–16 CA certificates, up to 64 KiB; private keys and website leaf certificates are rejected.

Additional CAs are trusted only in REL sessions using that proxy. They permit the proxy provider to inspect those sessions' HTTPS traffic. Hostnames, expiry dates, and certificate chains remain checked for pages and subresources. REL never installs these roots in Keychain or disables TLS verification. Saving a certificate change restarts affected browser views while preserving session storage. Switching to another proxy or a direct connection replaces or clears the additional roots.

Proxy and profile transfers preserve certificate settings. The import sheet identifies transfers that add a trusted proxy CA. Older transfer versions import with system trust.
