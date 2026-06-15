# 01 — `stencilwright`: privacy-preserving structure mapping

Status: **draft**, ready to implement.

`stencilwright` is the foundational tooling built *before* any adapter.
Production adapters (`apiwright`-shaped binaries) consume the mapping artifacts `stencilwright` produces
and then go against raw DOM directly — they never touch the masking
layer.

## 1. Goal

A Rust workspace that lets a developer (or an LLM collaborator)
iteratively explore authenticated, private web pages without ever
exposing the user's specific real values. The output of an exploration
session is a set of reusable artifacts under
`~/.stencilwright/<site>/`:

- `site.toml` — non-secret site-local settings, such as the
  1Password account selector for multi-account setups
- `places.toml` — graph of recognizable destinations, with named
  per-place elements
- `elements.toml` — site-wide named elements (selectors that recur
  across many places: logout button, global nav, etc.)
- `mask.toml` — site-wide masking policy (numeric blacklist patterns,
  always-redact selectors, length cap)
- `values.toml` — name → opaque secret-provider references for
  variable values (account numbers etc.) used in URL/selector
  interpolation

`places.toml`, `elements.toml`, `mask.toml`, and `values.toml` are
the **mapping**. Production adapter crates
consume them and produce structured data without ever invoking the
masking layer.

## 2. Why this exists

The constraint: an LLM (Claude) must be able to develop adapters for
authenticated financial and social sites *without ever seeing the
user's specific real values*. Every page the assistant ingests gets
logged in transcripts and the model's training pipeline; account
numbers, balances, names, and emails would leak.

The agent does need to see *some* real content to map the site —
column headers, button labels, page titles, form labels, structural
markup. That's not the threat. The threat is *specific* values: the
account number, the balance, the email address, the IP in a security
log. The masking pipeline lets the first class through (with explicit
opt-in) while blocking the second.

`stencilwright` is the wedge: the agent reads only masked DOM, with
selective per-element unmasks, and writes mapping configuration the
production adapters later consume.

## 3. Non-goals

- **Not a general Playwright Rust binding.** `stencil-browser` is
  specifically a wrapper that adds masking + recognition + the
  long-lived session daemon on top of `playwright-rs`. Use
  `playwright-rs` or `chromiumoxide` directly if that's what you want.
- **Not a stealth / anti-bot framework.** Sites with aggressive
  detection need stealth measures we'll add as a
  separate concern, composed with stencilwright rather than baked in.
  Stencilwright's long-lived session model + `channel = "chrome"`
  defeats the cheapest fresh-launch fingerprints; TLS/JA3,
  mouse-timing, and WebGL patches are out of scope.
- **No unmasked path through `stencilwright`'s output channels.**
  Stdout, stderr, and disk go through the masking layer.
  `Page::dump_raw()` and `Page::selector_text_raw()` live in
  `stencil-browser` behind `#[cfg(feature = "raw")]` — a Cargo
  feature that `stencilwright` does not enable, so those functions
  are not in its symbol table. The single in-process surface
  where unmasked content reaches the user is the Iced approval
  dialog (§4) — pixels only, never an agent-observable channel.
  No `--unmasked-output` flag, no env-var, no runtime escape hatch.
- **No screenshot capture in v1.** Images are hard to mask reliably.
  DOM and text only.
- **Always headed.** `stencilwright` always launches a visible Chrome
  window. No headless flag. (Production adapters may run headless.)

## 4. Threat model and masking pipeline

### Trust boundary

The boundary that matters is **the agent's transcript** (stdout,
stderr, files the agent reads, tool results) — not `stencilwright`'s
process memory. The property to preserve is *unmasked content never
reaches an agent-observable channel*. Two layers enforce this:

- **Crate-level (compile-time).** `Page::dump_raw()` and
  `Page::selector_text_raw()` live in `stencil-browser` behind
  `#[cfg(feature = "raw")]`. The daemon (which serves both
  `stencilwright` and `apiwright`) enables `raw`; `stencilwright`
  the binary does not. The unmasked-DOM fetching functions are not
  in `stencilwright`'s symbol table.
- **Output-channel (runtime).** Anything `stencilwright` writes to
  disk (`captures/<place>.html`) or returns over stdout / its
  socket has been through the masking pipeline. `stencilwright`
  never reads back the files it writes; the live browser state is
  the source of truth, and the masked files are ephemeral
  follow-along snapshots for the human and the agent.

Everything in `profile/` (cookies, session state) is sensitive and
never crosses either layer.

### Runtime dimension: in-process approval dialog

The one operation that *expands* what the LLM sees — opting an
element's text out of default-deny — needs the user, not the agent,
to gate it. The architecture: when the agent invokes
`stencilwright element add --unmasked …` (or `element unmask …`),
the CLI sends an RPC to the daemon. The daemon, which has raw DOM
access, fetches the real text content of the proposed selector's
matches, and serializes them back to `stencilwright` as an
`UnmaskedSnippets` struct over RPC.

`stencilwright` then renders the snippets in a **native GUI
approval dialog** built with `iced` and embedded in the CLI binary.
The dialog draws to pixels (not stdout/stderr/files), shows the
user the actual unmasked text that will become visible to the LLM
if approved, and waits for an Approve / Deny click. Only the
resulting decision plus optional user-authored feedback flows back
into the CLI handler; the `UnmaskedSnippets` struct is dropped when
the dialog closes.

The trust property in this design: the unmasked text passes
through `stencilwright`'s memory and screen pixels, but never
through any channel the agent can observe. Defense-in-depth in the
same process:

- `UnmaskedSnippets` lives inside `stencil-browser`'s approval module
  with `pub(crate)` visibility — CLI handler modules cannot construct,
  hold, or pattern-match it. They see only
  `Page::approve_unmask_with_context(…) -> UnmaskApprovalDecision`.
- No `Debug`/`Display` impls, or a custom impl that prints
  `<unmasked, n=…>` only. Kills accidental `dbg!`/`println!`/
  `tracing` leaks.
- The dialog module's only outputs to other modules are Approve/Deny
  plus optional user-written feedback. No caching, no logging of the
  snippets struct.

The consequence: **every `stencilwright` subcommand can be
auto-approved in the harness allowlist.** The CLI surface contains
no command that can leak unmasked content; the only gate in the
system is the dialog click, which is mechanical (a window the user
sees and the agent does not), not a convention.

### Layered redaction

Every text node in a captured DOM passes through this pipeline. Order
matters; later layers operate on the result of earlier layers.

1. **Always-redact selectors** (from `mask.toml`'s
   `redact_selectors`): elements matching any of these have their
   entire text content collapsed to a single slot. Used for known
   sensitive regions whose contents the regex set might miss
   (`.js-balance`, `[data-account]`).

2. **Numeric blacklist** (from `mask.toml`'s `patterns`): regexes that
   match values categorically — currency, long digit runs (≥8), dates,
   times, IPs, hex blocks, emails, phones, UUIDs, percents. Each
   match becomes a slot. Patterns are pure detectors; they don't carry
   placeholder text of their own anymore.

3. **Default-deny on text content**: any remaining text not matched
   above is replaced with `[TEXT:<len>]` (length-tagged, no slot
   identity). The agent sees the structure but no text content.

4. **Per-element unmask** (from `[[place.element]] unmasked = true`
   in `places.toml` or `[[element]] unmasked = true` in
   `elements.toml`): if the text node is inside a matched unmasked
   selector, its text passes through verbatim *except* substrings
   still hit the numeric blacklist (so an unmasked `<th>Account
   12345678</th>` becomes `<th>Account [$brokerage_account 8-digit
   numeric]</th>` if the user has named that account). Unmasking
   is gated by the in-process approval dialog above. The dialog shows
   non-secret approval context (site, scope/place, current URL,
   request reason, selector, proposed element name, and place
   signature criteria) next to the raw text snippets. The dialog also
   has an optional feedback field for user guidance that is printed to
   the CLI; users should describe field meaning or next steps without
   typing exact private values. The agent cannot add
   `unmasked = true` to a TOML directly (see §7 permission model and
   the editor hook).

5. **Length cap** (from `mask.toml`'s `max_unmasked_chars`, default
   200): unmasked text longer than the cap is redacted to
   `[TEXT:<len>]` regardless. Defense against an opted-in `<label>`
   that contains a paragraph of user data.

6. **Attribute values** are walked separately, in three tiers:

   - *Identity-bearing* attributes — `username`, `display-name`,
     `author`, `author-id`, `user-id`, `account-id`, `email`,
     `post-upvote-ratio`, and **every** attribute of a `current-user`
     element — always collapse to a single whole-value slot, regardless
     of scope.
   - *Content-bearing* attributes carry free, human-readable text — the
     same kind a text node holds, and just as PII-prone: `aria-label`,
     `title`, `alt`, `placeholder`, `data-stringify-text`, an
     `<input>`/`<textarea>`'s `value`, and similar. These follow the
     **same default-deny ladder as text** (layers 3/4/5 above): masked
     to `[ATTR:<len>]` by default, or — inside an unmask scope — passed
     through with the numeric blacklist still applied and the length cap
     enforced. Without this, content like `aria-label="Jane Doe"` or a
     `data-stringify-text` display name leaks verbatim.
   - *Structural* attributes (`class`, `id`, `data-qa`, `href`, `role`,
     `aria-hidden`, `<option value>`, non-content `data-*`, …) stay
     legible so selectors can be built against them; only named
     `values.toml` substrings and numeric/email blacklist matches that
     appear *inside* them are redacted.

7. **Comments and raw-text containers**: HTML comments and
   `script` / `style` / `template` bodies are collapsed before the DOM
   walk. They are not useful for mapping and often carry framework
   IDs or serialized markup that the normal element walker cannot
   safely inspect.

DOM **structure** is always preserved: tag names, class lists, ids,
ARIA roles, and structural (non-content) data attributes — this is what
keeps the masked output useful for mapping. Content-bearing attributes,
by contrast, are masked like text (layer 6) so structure stays legible
without content leaking through it.

The asymmetry to internalize: **inputs we *send* are not masked.**
When the runner does `fill("input#login", "{username}")`, the real
username is interpolated and reaches the page. Masking only affects
what comes *back*.

### Slot derivation

When a value gets a slot (layers 1 and 2 above), the slot has the form:

```
[$<id> <description>]
```

- **`<id>`** — either a user-given **name** (if the value matches a
  resolved entry in `values.toml`'s value→name map), or the first 8
  hex chars of `sha256(value)` otherwise. User names are snake_case;
  hashes are hex; visually distinguishable.
- **`<description>`** — output of the **describer pipeline**, a small
  fixed set of heuristic detectors that label the value's shape:

  | Describer | Description string |
  |---|---|
  | currency regex matched | `currency` |
  | email regex matched | `email` |
  | IPv4 regex matched | `ipv4` |
  | IPv6 regex matched | `ipv6` |
  | ISO datetime matched | `datetime` |
  | ISO date matched | `date` |
  | Time-of-day matched | `time` |
  | Phone regex matched | `phone` |
  | UUID regex matched | `uuid` |
  | Hex block (16+) matched | `hex` |
  | Pure digits (8+) | `<N>-digit numeric` |
  | Percent regex matched | `percent` |
  | Caught only by selector blacklist | `text` |

  More-specific describers win (UUID > hex_block, IPv4 > generic
  numeric, etc.).

The slot system gives the agent **equality identity** (same value →
same slot wherever it occurs) plus **categorical type info** (the
describer). It does not reveal the value itself.

The slot map is **per-session, in-memory only**. There is no
`stable.toml` or any durable hash table. During a daemon session:

1. `values.toml` is loaded as provider references only.
2. Non-credential, non-TOTP references are resolved through the local
   secret-provider CLI when the daemon builds the value→name map for
   masking. Username, password, and TOTP fields are skipped for
   passive dumps so a capture does not prompt for login secrets merely
   to name slots.
3. Resolved values populate the value→name map in daemon memory.
4. As the session masks captures, unnamed values that match patterns
   get `[$<hash> ...]` slots; the value→hash map is held in memory for
   the lifetime of the daemon.

When the daemon exits, the map dies with it. Restart and rebuild on
the next `stencilwright place goto`.

## 5. Architecture

### Crate layout

```
example-adapter/
├── Cargo.toml                  (workspace)
├── crates/
│   ├── stencil-core/           types only (Place, Element, Signature, MaskPolicy data, Slot, …)
│   ├── stencil-mask/           lol_html-based apply; describer pipeline; slot derivation
│   ├── stencil-browser/        playwright-rs wrapper, daemon process, Unix-socket RPC,
│   │                           Page wrapper. dump_raw / selector_text_raw live here behind
│   │                           #[cfg(feature = "raw")].
│   ├── stencil-secrets/        1Password CLI shell-out, value resolution
│   ├── stencil-places/         load mapping artifacts, recognize, recover, place_goto
│   ├── stencilwright/          dev binary. Depends on stencil-places (stencil-browser
│   │                           WITHOUT raw feature). Always headed. Masked by default.
│   └── apiwright/           runtime binary. Depends on stencil-browser WITH raw
│                               feature, stencil-places, stencil-secrets. Configurable
│                               headed/headless. No masking. (Stub in v1.)

~/.stencilwright/<site>/         (created by `stencilwright init`;
    ├── site.toml                 non-secret local settings
    ├── places.toml               outside the project working dir
    ├── elements.toml             so agent harnesses cannot reach
    ├── mask.toml                 it via Edit/Write — see §7
    ├── values.toml               permission model)
    ├── profile/                (Chrome user-data-dir; 0700)
    ├── .session                (daemon pid + sock path)
    ├── .session.sock           (Unix socket)
    └── captures/<place>.html   (masked dump files)
```

Two trust boundaries layer on top of each other:

1. **Crate-level (compile-time):** `stencilwright`'s Cargo.toml
   does not enable the `raw` feature on `stencil-browser`, so
   `Page::dump_raw` and `Page::selector_text_raw` are not in its
   symbol table.
2. **Filesystem layout (runtime):** mapping artifacts live under
   `~/.stencilwright/<site>/`, outside the agent's typical
   write-scope. Unmasking edits can only land via
   `stencilwright element add --unmasked` / `element unmask`,
   both of which route through the in-process Iced approval
   dialog (§4).

### Long-lived session daemon, auto-started

The daemon owns the Playwright stack and a single visible **system
Chrome** window launched via `launch_persistent_context(profile, opts)`
with:

- `channel = "chrome"` — the user's installed Chrome
  (`/Applications/Google Chrome.app` on macOS), not Playwright's
  bundled Chromium. Native UA, build, fonts, ICU, V8, TLS fingerprint.
- `ignore_default_args = ["--enable-automation"]` — no automation
  banner, no `navigator.webdriver = true`.
- `headless = false`.

The daemon listens on `~/.stencilwright/<site>/.session.sock` and
writes `~/.stencilwright/<site>/.session` with
`{pid, sock, started_at}`.

Browser-touching CLI commands (`place goto`, `element add`,
`page goto/click/fill`, etc.) are short-lived RPC frontends:

1. Read `.session`. If missing/stale, **auto-start** the daemon
   (fork-detach; lock-file on `.session` to serialize concurrent
   starts; wait up to ~10 s for socket).
2. Connect, send a JSON-line command, receive a result. For secret
   operations the client sends references (`secret://...` or `{name}`)
   and non-secret mapping config; the daemon resolves values in
   memory and returns only masked capture output.
3. Disconnect and exit.

The daemon stays alive across many invocations. Browser uptime,
network history, and cookie-vs-session-age stay aligned with what a
real user looks like — defeating the cheapest fresh-launch bot
signals.

The daemon stops on:
- `stencilwright session stop <site>`
- The user closes the Chrome window (daemon notices and cleans up)
- SIGTERM

`stencilwright session start/stop/status` exist for explicit lifecycle
control but the developer rarely names them — `place goto` /
`page *` / etc. auto-start.

### The recognition runner

There is no script. `stencil-places` exposes one operation:

```
place_goto(target):
    here = recognize_current_page(places)
    if here matches target.signature:
        if target.interactive:
            handle_interactive(target)
        return mask_and_dump()                # fast-path: live page already at target
    navigate to target.url (interpolating any {var_name} from values.toml)
    loop:
        here = recognize_current_page(places)
        if here matches target.signature:
            if target.interactive:
                handle_interactive(target)
            return mask_and_dump()
        else if here is None:
            wait until any known place appears, then re-recognize
        else if here.interactive:
            run auto_fill(here.elements)         # username / password / TOTP
            if here.submit is set and all fills succeeded:
                click(here.submit.click)
            wait until here.completion or until recognition leaves here
            re-recognize without re-navigation
        else if here.redirect:
            navigate to here.redirect
        else if here is a known transit point toward target:
            execute(here.transition_toward(target))
        else:
            halt("landed at {here.name}, no path to {target.name}")
```

The fast-path matters: once the daemon is alive and the browser is
at place X, re-running `place goto X` after a policy or unmask
change re-fetches the live DOM and re-masks without paying any
navigation, auth, or recognition-recovery cost. The masked file
write at `captures/<place>.html` is a side effect (an ephemeral
snapshot for the human and the agent); `stencilwright` never reads
it back.

Recognition strength is critical. A signature combines structured URL,
required selector, visible selector, absent selector, optional text —
AND-combined to avoid false positives on auth bounces, hidden auth
panels, or error pages.

## 6. Artifact formats

### `places.toml`

```toml
target = "example"
description = "Example places: login, home, listing_<name>"

[[place]]
name = "login_password"
interactive = true
submit.click = "app-form#login button.login"
signature.url = "https://www.example.com/login/"
signature.visible_selector = "app-form#login"

  [[place.element]]
  name = "username_field"
  selector = "app-text-input#login-username input"
  auto_fill = "{example_username}"

  [[place.element]]
  name = "password_field"
  selector = "app-text-input#login-password input"
  auto_fill = "{example_password}"

[[place]]
name = "login_otp"
interactive = true
submit.click = "app-form#login-app-otp button.check-app-code"
signature.url = "https://www.example.com/login/"
signature.visible_selector = "app-form#login-app-otp"

  [[place.element]]
  name = "app_otp_field"
  selector = "app-text-input#one-time-code-appOtp input"
  auto_fill = "{example_totp}"

[[place]]
name = "home"
url = "https://www.example.com/"
signature.url = "https://www.example.com/"
signature.selector = "app-shell, [data-testid='home-feed']"
signature.absent_selector = "form[action*='/login']"

[[place]]
name = "listing_main"
url = "https://www.example.com/feed/main/"
signature.url = "https://www.example.com/feed/main/"
signature.selector = 'app-feed[reload-url*="name=main"]'
signature.absent_selector = "form[action*='/login']"

  [[place.element]]
  name = "post_titles"
  selector = 'a[id^="post-title-"][slot="title"]'
  unmasked = true        # only after Iced approval

  [[place.element]]
  name = "post_authors"
  selector = "app-post [data-author]"
  # unmasked omitted → still hashed (usernames are PII for our purposes)
```

Each `place` has:
- A signature (structured URL, required selector, optional visible
  selector, optional absent_selector, optional text)
- Either a `url` (direct) or a `from`/`via` transition from a parent
- An optional `redirect = "<url>"` field — when this place is
  recognized, the runner navigates to the URL and re-recognizes.
  Models "you shouldn't be here, go elsewhere" pages (e.g., Example's
  forbidden / permission-denied page that doesn't auto-bounce to
  login). Mutually exclusive with `interactive` semantics in
  practice — we don't auto-fill a place we're going to leave.
- Optional `interactive = true` + `completion` for human-completed
  flows (login, push 2FA)
- Optional `submit.click = "<selector>"` for interactive states whose
  form should be submitted immediately after successful auto-fill.
  This is how TOTP fields avoid crossing the 30-second code boundary.
- A `[[place.element]]` array of named selectors. Each element is
  a name+selector pair with optional `auto_fill` (provider reference
  or `{name}` for the runner to fill at navigation time) and optional
  `unmasked = true`
  (text content of matched nodes passes through the masking layer
  verbatim, modulo numeric blacklist on substrings + length cap).
  Default-deny: an element with neither flag is a pure label —
  surfaces the structural anchor to the masked output and gives
  other code a stable handle, but the text remains hashed.

`signature.url` is not a regex. It is parsed as a URL-shaped matcher:
scheme and host must match, path segments match with trailing slash
normalization, query params present in the signature must exist on the
current URL regardless of order, repeated query params require
repeated matches, and extra current query params are allowed. A
fragment is required only when present in the signature. `{name}` and
`*` match within any component; `**` as a path segment matches zero or
more path segments.

`signature.visible_selector` has the same comma-list OR semantics as
`signature.selector`, but each selector must match at least one
Playwright-visible element. Use it for same-URL flows where inactive
forms stay mounted in the DOM, such as Example's password and TOTP
login panels.

### `elements.toml`

Site-wide named elements. Implicitly available at every place. Conflicts
on name (also defined in a place) are configuration errors.

```toml
[[element]]
name = "logout_button"
selector = "[data-testid='logout-button']"
unmasked = true

[[element]]
name = "global_search"
selector = "input[type='search']"
```

### `mask.toml`

Site-wide masking policy. No site-wide unmask block — unmasking is
per-element via `unmasked = true` only in v1 (heuristic auto-unmask
is a future optimization).

```toml
# Numeric blacklist. Always applied. Regex-only; describers run separately.
[[mask.pattern]]
name = "currency"
regex = '\$[0-9][0-9,]*(?:\.[0-9]+)?'

[[mask.pattern]]
name = "long_digits"
regex = '[0-9]{8,}'

[[mask.pattern]]
name = "email"
regex = '[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}'

# Always-redact selectors (whole element's text → slot).
[[mask.redact_selectors]]
selector = ".js-balance, [data-test='balance']"

# Length cap on revealed text.
max_unmasked_chars = 200
```

`stencilwright init` ships a default `mask.toml` populated with the
standard pattern set (currency, long-digits, email, ipv4, ipv6, iso
datetime / date, time, phone, uuid, hex_block, percent).

### `values.toml`

Name → secret-provider reference mapping. Committed (references only;
no literals, no secrets). References are opaque id-based strings that
do not contain vault or item titles.

```toml
example_username   = "secret://1password/vault-id/example-item-id/username"
brokerage_account = "secret://1password/vault-id/example-item-id/account_number"
bank_username     = "secret://1password/vault-id/bank-item-id/username"
```

Resolved by the daemon on demand through the provider implementation.
Non-TOTP values are cached in memory for the daemon session. Resolved
non-credential values populate the in-memory value→name map for slot
derivation, and resolved values are interpolated into `{name}`
placeholders in URLs and selectors at navigation time.

`auto_fill` fields in places.toml/elements.toml that point at
secret-provider references are resolved the same way at fill time.

## 7. Public API and CLI

### Library API (key types)

```rust
// stencil-core
pub struct Place { name: String, url: Option<String>, from: Option<String>,
                   via: Option<Transition>, interactive: bool,
                   submit: Option<Submit>,
                   signature: Signature, completion: Option<Signature>,
                   elements: Vec<Element> }
pub struct Submit { click: Option<String> }
pub struct Element { name: String, selector: String, auto_fill: Option<String>,
                     unmasked: bool }
pub struct Signature { url: Option<String>, selector: Option<String>,
                       visible_selector: Option<String>,
                       absent_selector: Option<String>, text: Option<String> }
pub struct Slot { id: SlotId, description: String }
pub enum SlotId { Named(String), Hash(String) }

// stencil-mask
pub struct MaskPolicy { /* loaded from mask.toml + elements.toml + places.toml */ }
impl MaskPolicy {
    pub fn for_place(&self, place: &Place) -> EffectivePolicy;
}
impl EffectivePolicy<'_> {
    pub fn apply(&mut self, html: &str, value_to_name: &ValueNameMap) -> MaskedHtml;
}

// stencil-browser
pub struct Session { /* RPC handle to a running daemon */ }
impl Session {
    pub async fn attach(site: &str) -> Result<Session>;        // auto-starts if needed
    pub async fn close(self) -> Result<()>;
    pub async fn page(&self) -> Result<Page>;
}
pub struct Page;
impl Page {
    pub async fn goto(&self, url: &str) -> Result<()>;
    pub async fn click(&self, sel: &str) -> Result<()>;
    pub async fn fill(&self, sel: &str, value: &str) -> Result<()>;
    pub async fn select_option(&self, sel: &str, value: &str) -> Result<()>;
    pub async fn wait_for(&self, sel: &str, timeout: Duration) -> Result<()>;
    pub async fn dump(&self, policy: &mut EffectivePolicy<'_>,
                      vn: &ValueNameMap) -> Result<MaskedHtml>;

    /// Library-only; behind #[cfg(feature = "raw")]. Used by apiwright,
    /// the daemon's approval-snippet RPC handler, and downstream
    /// the downstream adapter crates.
    #[cfg(feature = "raw")]
    pub async fn dump_raw(&self, _: RawAccess) -> Result<RawHtml>;
    #[cfg(feature = "raw")]
    pub async fn selector_text_raw(&self, _: RawAccess, sel: &str)
        -> Result<Vec<RawSnippet>>;
}

#[cfg(feature = "raw")]
pub struct RawAccess(());
#[cfg(feature = "raw")]
impl RawAccess { pub fn acknowledged() -> Self { Self(()) } }

// stencil-places
pub struct PlaceGraph { /* loaded from places.toml + elements.toml */ }
impl PlaceGraph {
    pub fn from_dir(stencils_site: &Path) -> Result<Self>;
    pub fn place(&self, name: &str) -> Option<&Place>;
    pub async fn recognize(&self, page: &Page) -> Result<Option<PlaceMatch>>;
    /// Recognize first; navigate only on miss. Re-fetches and masks
    /// the live DOM each call. Writes `captures/<target>.html` as a
    /// follow-along side effect.
    pub async fn place_goto(&self, page: &Page, target: &str,
                            policy: &MaskPolicy, vn: &ValueNameMap)
        -> Result<MaskedHtml>;
}
```

### CLI

Two CLI patterns coexist:

- **Meta** (creates/manages the site itself): verb-first form —
  `init`, `load`, `session`.
- **Resource ops** (work *on* an existing site): site-first form
  — `<site> <resource-path> <verb>`, where the path is one of
  `place`, `place <name>`, `element`, `page`, or `value`.

The mapping loop:

```
1. <site> page goto <url>                  # land somewhere new (raw)
2. <site> place add <name> [<selector>]    # register current state as a place
3. <site> place <name> element add ...     # add anchors at that place
4. <site> page click / fill                # advance the live tab
5. <site> place add <next> [<selector>]    # register where we landed
6. <site> place <next> element add ...
... repeat ...
N. <site> place <name> goto                # production: validates + navigates
```

- **`page goto/click/fill/dump`** drive the live browser tab
  without place-graph validation. Used for landing at a fresh URL,
  driving sub-page interactions (captchas, lazy-loaded feeds), or
  taking a quick masked snapshot.
- **`place add`** captures the currently-loaded state as a named
  destination. The primary way new places enter the graph.
- **`place <name> goto`** is for places already in the graph:
  recognize, navigate on miss, auto-fill interactive
  intermediaries, dump. This is what apiwright runs in
  production. It will not navigate to an unregistered destination
  — `place add` is the registration verb.

All TOML-mutating resource commands take an exclusive site-local
`.config.lock` across the full read-modify-write cycle. The write
itself is a same-directory temp-file rename, so concurrent mutators do
not lose updates and non-mutating readers see either the previous
complete TOML document or the next one.

#### Meta commands

```
stencilwright init <site>
        # scaffold ~/.stencilwright/<site>/

stencilwright load <site> [<source-path>]
        # copy a site's configs into ~/.stencilwright/<site>/
        # (default source: ./stencils/<site>/; used for the
        # one-time CP6 migration). Errors if target exists;
        # --force to overwrite.

stencilwright session start  <site>   # explicit launch
stencilwright session stop   <site>   # SIGTERM the daemon
stencilwright session status <site>   # alive? uptime?
```

#### Resource ops (site-first)

```
# place — recognized, validated destinations.
<site> place add <name> [<selector>]
        # capture loaded daemon's current URL; substitute named
        # values from values.toml as URL templating (not privacy
        # — URL-content masking is out of scope); append a draft
        # [[place]] block. If <selector> is supplied, lands as
        # signature.selector verbatim (Playwright locator syntax;
        # comma-separated OR list allowed). If omitted, the
        # signature is URL-only — fine for places with
        # distinctive URLs; SPAs that bounce via JS without
        # changing the URL need a selector. User refines in TOML.
        # Requires a daemon at a non-blank URL.
<site> place list
        # pretty-print known places (table: name | url |
        # interactive | #auto-fill | #unmasked); when daemon is
        # up, append a "→ currently at: <name>" footer if
        # recognition succeeds.

# place <name> — operations at a registered place.
<site> place <name> goto
        # auto-start daemon; recognize-first; navigate on miss;
        # mask + dump to ~/.stencilwright/<site>/captures/<name>.html.
<site> place <name> element add <selector> \
        [--as <id>] [--auto-fill <secret://ref-or-{name}>] \
        [--unmasked] [--reason <user-visible purpose>]
        # idempotent on (place, selector). Same selector but
        # different --as: errors. --unmasked on a selector that
        # already exists: equivalent to `element unmask <selector>`
        # (dialog fires). --unmasked validates the live page is at
        # this place before opening the dialog; otherwise errors.
        # --reason is shown in the approval dialog; use it to name
        # the suspected field/purpose without including private
        # values.
<site> place <name> element unmask <selector> [--reason <user-visible purpose>]
        # toggle existing element's `unmasked = true`. Live-page
        # validation + approval dialog.
<site> place <name> element list
        # pretty-print at-place elements.

# element — site-wide anchors (under elements.toml).
<site> element add <selector> \
        [--as <id>] [--auto-fill <secret://ref-or-{name}>] \
        [--unmasked] [--reason <user-visible purpose>]
        # site-wide variant. --unmasked has no place context to
        # validate against; runs against the live page wherever it
        # is, dialog fires.
<site> element list
        # pretty-print site-wide elements.

# page — free-form ops on the live tab.
<site> page goto  <url>                 # raw navigation
<site> page click <selector> [--force]  # --force bypasses actionability checks
<site> page press <selector> <key>      # one key on a selector (e.g. Enter)
<site> page type  <selector> <text>     # per-character key events (rich editors)
<site> page key   <key>                 # key on the focused element (no selector)
<site> page fill  <selector> <value-or-op-uri>
<site> page dump
        # mask + dump current live DOM (default policy, no place
        # context). Writes ./_page.html in the current working
        # directory and prints the path. The agent reads the file
        # with offset/limit; multi-KB stdout would otherwise
        # bloat the conversation transcript.

# value — named references to secret-provider values (values.toml).
<site> value add <name> <reference>
        # append `<name> = "<reference>"` to
        # ~/.stencilwright/<site>/values.toml. Validates supported
        # reference shapes
        # (`secret://1password/<vault-id>/<item-id>/<field>` or
        # `secret://1password/<vault-id>/<item-id>/otp?`). Does
        # not resolve the reference — the secret never enters
        # stencilwright memory outside fill/interpolation paths.
<site> value search <query> [--limit N] [--category Login]
        # daemon-owned provider discovery. The daemon may fetch broad
        # provider metadata, filters in memory by keyword/URL fragment,
        # sorts matches by updated_at descending, and returns only the
        # bounded matching subset: title, vault, category, URLs,
        # timestamps, and generated opaque secret:// references. Secret
        # values are never read.
<site> value list
        # pretty-print values.toml: name → reference.
<site> value remove <name>
        # remove the named entry.

# config — non-secret site settings (site.toml).
<site> config show
        # pretty-print site.toml settings.
<site> config set --onepassword-account <account>
        # write onepassword_account to site.toml. The daemon passes
        # it to op as --account on first provider use. This is an
        # account selector, not a secret.
<site> config set --clear-onepassword-account
        # remove the explicit account selector; OP_ACCOUNT remains
        # a non-secret fallback when inherited by the daemon process.
```

There is no `element mask` counterpart to `unmask` — un-unmasking
is visibility-reducing and isn't gated; users edit the TOML
directly. The `--unmasked` flag and `element unmask` verb both
invoke the daemon-spawned in-process Iced approval dialog (§4); on
Deny, the TOML is not written, any user feedback is printed, and the
CLI exits non-zero with a masked `element add` command the agent can
run to map the field without unmasking it.

Login is just a place with `interactive = true`. Secrets live in a
provider and are referenced via opaque `secret://...` strings.

### Permission model

Every `stencilwright` subcommand auto-approves at the harness
allowlist level. The CLI surface contains no command that can leak
unmasked content to an agent-observable channel:

- Output to stdout, stderr, and disk is masked.
- The `--unmasked` / `unmask` paths route through an in-process
  Iced approval dialog (§4) that draws to pixels, not to any tool
  result the agent can read; the CLI handler only ever sees the
  resulting Approve/Deny decision plus optional user-written feedback.
- The crate-level `raw` Cargo feature is off in `stencilwright`,
  so `Page::dump_raw` / `selector_text_raw` are not in the binary's
  symbol table. Unmasked text reaches the dialog module by RPC
  from the daemon (which has `raw` enabled).

The trust property that the agent cannot bypass the dialog by
direct file edit is enforced by **filesystem layout**, not by a
harness hook. All configurable artifacts live under
`~/.stencilwright/<site>/`:

```
~/.stencilwright/<site>/site.toml
~/.stencilwright/<site>/places.toml
~/.stencilwright/<site>/elements.toml
~/.stencilwright/<site>/mask.toml
~/.stencilwright/<site>/values.toml
~/.stencilwright/<site>/profile/        # Chrome user-data-dir; mode 0700
~/.stencilwright/<site>/captures/       # masked dump files
~/.stencilwright/<site>/.session*       # daemon pid + sock
```

Agent harnesses typically scope `Edit`/`Write` operations to the
project working directory. Configs outside that scope are
unreachable to direct edits and only mutable through
`stencilwright`, which routes unmasking through the dialog. (A
harness configured to grant write access to `$HOME` bypasses this;
we document the trust assumption rather than enforcing it
OS-level.) A future `stencilwright export <site>` will materialize
a transportable bundle for apiwright on a different machine —
filed.

## 8. Secrets — provider references

All secret material is stored outside the repo and accessed through a
local secret-provider CLI. The first provider is 1Password via `op`.
`values.toml` and `auto_fill` fields contain provider references only;
never literal secrets.

Supported stored references:

```text
secret://1password/<vault-id>/<item-id>/<field>
secret://1password/<vault-id>/<item-id>/otp?
```

The `secret://1password/...` form is generated by provider discovery.
It stores 1Password vault/item IDs instead of user-visible vault/item
titles.

Resolution at fill / interpolation time:

```
op item get <item-id> --vault <vault-id> --fields label=username
```

The daemon captures the command's stdout in memory, uses the value to
fill the browser, interpolate a URL, or build the value→name mask map,
and never writes the value to disk or returns it to the CLI. Non-TOTP
values are cached in daemon memory for the lifetime of the session.
Passive dumps skip credential-shaped fields (`username`, `password`,
`one-time password`) when building the value→name map so captures do
not prompt for login secrets merely to name slots.

For TOTP fields, stencilwright uses a trailing-`?` sentinel:
`secret://1password/<vault-id>/<item-id>/otp?`. The daemon strips the
sentinel and calls `op item get ... --otp` just in time. Plain
`op read` on a TOTP field returns the long-lived `otpauth://` seed URI
on current 1Password CLI versions, so it must not be used for OTP
fields. We never store, cache, or generate TOTP codes ourselves.

Provider discovery has two allowed surfaces:

- Filtered terminal search: `value search <query>` requires an explicit
  keyword or URL fragment. The daemon owns the broad provider call,
  filters in memory, sorts matches by `updated_at` descending, caps the
  result set, and returns only matching metadata plus generated opaque
  `secret://...` references. It must not include item notes,
  `additional_information`, field values, or any secret material.
- Future native picker: same daemon RPC, but rendered as user-only GUI
  so the user can select item/field without exposing even filtered
  metadata to the agent transcript.

Push 2FA / SMS / hardware-key 2FA: not automatable; covered by the
generic interactive halt at login places.

Prerequisites: `op` CLI installed (`brew install 1password-cli`) and
the 1Password desktop CLI integration unlocked. The short-lived
`stencilwright` CLI process must not probe or sign in to the secret
provider during startup; `session start` only performs local browser /
profile checks and spawns the daemon. The daemon contacts the provider
on first use (`op item get`, `op item list`, or equivalent future
provider calls), so any biometric or sign-in challenge belongs to the
daemon-owned secret path. Multi-account setups should set
`onepassword_account` in `site.toml`, usually via
`stencilwright <site> config set --onepassword-account <account>`.
The daemon passes that selector to `op` as `--account`. `OP_ACCOUNT`
remains a non-secret fallback for ad-hoc commands and daemon processes
that inherit it; `OP_SESSION*` is scrubbed when spawning the daemon so
caller shell auth state is not silently reused.

## 9. Workflow / iteration loop

```
1. stencilwright init <site>                     [one-time scaffold]

2. Add provider references for any user-known variable values:
   stencilwright <site> value add example_username "secret://1password/<vault-id>/<item-id>/username"

3. stencilwright <site> place add <name> [selector]
                                                 [capture the current live
                                                  URL + optional selector as
                                                  a draft place]
   — usually starting with an interactive auth state
   (`login_password`, `login_otp`, etc.) and one navigation target
   (home / a listing / a settings page).

4. stencilwright <site> place <place> goto       [auto-starts daemon;
                                                  Chrome opens; recognize
                                                  bounces to login; user
                                                  authenticates; runner
                                                  resumes; masked HTML
                                                  dumped to captures/]

5. Read the masked dump. Three things to look for:
   - Recognition correct? (signature matched the expected place)
   - Slot identity intelligible? (recurring values share slots; values
     you named in values.toml show as $name)
   - Enough structure visible to map further? If not, identify a
     selector whose text would help, propose unmasking it.

6. stencilwright <site> place <place> element add <selector> --unmasked --as <name> --reason <purpose>
   [or, if the element already exists as a label/auto-fill:]
   stencilwright <site> place <place> element unmask <selector> --reason <purpose>
                                                  [in-process Iced dialog
                                                   pops up; user reviews
                                                   place/current URL/matching
                                                   criteria/request reason and
                                                   the actual unmasked text
                                                   that would become visible
                                                   to the agent; clicks
                                                   Approve or Deny; may add
                                                   feedback]

7. stencilwright <site> place <place> goto       [fast-path: page already
                                                  at place; recognize
                                                  succeeds; re-fetch live
                                                  DOM; re-mask with the
                                                  new unmasked element;
                                                  overwrite captures/<place>.html]

8. Repeat 5–7 until the place set covers what's needed.

9. The user (not the assistant) runs the downstream adapter crate
   (e.g., cargo run -p the-adapter) once to verify it returns
   sensible real data via Page::dump_raw. Agent never executes that
   binary.
```

The daemon stays alive across the entire loop. The browser ages
naturally. No relaunch, no fresh-load fingerprint. There is no
"offline re-mask" step — every iteration just re-runs `place goto`
against the loaded page; the file dump is a side effect.

## 10. First milestone — Example

Implementation target before anything else. Example chosen because:
mainstream account everyone has, public content everywhere, login
flow is standard (username + password + optional TOTP), modest bot
detection.

Acceptance criteria — all must hold:

1. **Workspace builds clean.** `cargo check --workspace` passes;
   `cargo build --workspace` produces `stencilwright` and (stub)
   `apiwright` binaries.

2. **Secret-provider integration smoke-tests.** A gated manual or
   ignored test in `stencil-secrets` resolves an opaque
   `secret://1password/...` field reference through `op item get
   --fields`, and resolves a trailing-`?` TOTP reference through
   `op item get --otp`. The test is skipped in CI.

3. **`stencilwright init example`** scaffolds
   `~/.stencilwright/example/` with `site.toml`, `places.toml`,
   `elements.toml`, `mask.toml`, `values.toml`, an empty
   `profile/`, and `captures/`. All five TOML files have sensible
   default content.

4. **Daemon auto-starts and masking works on a static fixture.** A
   unit test for `stencil-mask` against a synthetic Example-shaped
   HTML fixture: tag structure preserved; numeric/email/datetime
   patterns redacted; named values from a fixture `values.toml` show
   as `[$<name> ...]`; unnamed matches show as `[$<hash> ...]`;
   `unmasked = true` on a fixture element passes its text through
   with substring redaction still applied.

5. **End-to-end `place goto` from a fresh profile.**
   `stencilwright example place listing_main goto` against a fresh
   `~/.stencilwright/example/profile/`:
   - Auto-starts the daemon (visible Chrome opens, no automation
     banner).
   - Navigates to `https://www.example.com/feed/main/`.
   - Recognition determines the page is `login_password`, `captcha`,
     `needs_login`, or another mapped auth state as appropriate.
   - Auto-fills `username_field` and `password_field` from named
     `values.toml` references when `login_password` is active. TOTP,
     if required, is recognized as `login_otp`, resolved through the
     trailing-`?` OTP convention, and submitted promptly enough to
     avoid normal code expiry.
   - Halts/polls: the runner auto-fills what it can, then watches the
     interactive place's completion signature, or waits until
     recognition leaves that place when no completion is configured,
     while the user finishes any human-only steps (CAPTCHA, push,
     etc.).
   - Daemon resumes from recognition; navigates back to
     `https://www.example.com/feed/main/` only if needed.
   - Recognition matches `listing_main`.
   - Dumps masked DOM to `captures/listing_main.html`.

6. **Unmask cycle.** Before unmasking, post titles in the masked
   dump show as `[TEXT:<len>]`. Run `stencilwright example place
   listing_main element add "app-post a[slot='title']" --as
   post_titles --unmasked --reason "public post titles"`. The daemon
   validates the live page is at `listing_main`, fetches matching
   elements' real text, and the in-process Iced dialog opens showing
   the actual post titles plus request context; the user clicks
   Approve. The CLI writes the `[[place.element]]` table with
   `unmasked = true`. Re-run `stencilwright example place
   listing_main goto` (fast-path: already at place; no
   navigation; live DOM re-fetched + re-masked). The titles now
   appear in plain text in the new dump. (Numeric blacklist still
   hits any embedded numbers.) Verify the converse path: a second
   `element add --unmasked` invocation followed by Deny in the
   dialog leaves places.toml unchanged, prints any user feedback, and
   suggests the equivalent masked `element add` command.

7. **Re-running `place goto` preserves slot identity.** A second
   `stencilwright example place listing_main goto` (same daemon
   session) produces a dump where named values still show as
   `[$example_username ...]` and unnamed recurring values use the
   same hash slots within that session. (Across daemon restarts,
   slot hashes for unnamed values may or may not match — we don't
   persist.)

When all seven hold, stencilwright is ready for higher-stakes (e.g. financial) sites.

## 11. Open questions / deferred


0. **Attribute masking hardening.** Largely landed (§4 layer 6):
   identity-bearing attributes collapse to a whole-value slot, and
   *content-bearing* attributes (`aria-label`, `title`, `alt`,
   `data-stringify-text`, an `<input>`'s `value`, …) now default-deny
   exactly like text nodes — masked to `[ATTR:<len>]`, with the
   unmask-scope escape hatch — while structural attributes stay legible.
   Remaining: keep the content/identity attribute lists current as real
   sites surface new leak classes (a JSON/network-aware redactor is the
   next frontier — see the masked-network tracking issue). Still a
   high-scrutiny area before and during financial-site mapping.

0a. **Per-place `unmask_all` opt-in + `--unmask-as <place>`
   inspector.** Two opt-in shapes for blanket-unmasking a place's
   text content during mapping (numeric blacklist + length cap
   still apply): (a) committed `[[place]] unmask_all = true` for
   places the user judges safe (public Example posts, etc.) — never
   for financial sites; (b) one-shot `stencilwright place goto
   --unmask-as <name>` that recognizes the current page first and
   refuses to blanket-unmask if the recognized place differs from
   `<name>`. Both keep the user as gatekeeper of the trust
   boundary, with the same in-process approval dialog gating each
   blanket invocation. Useful once per-element unmask starts
   feeling tedious; not load-bearing for v1.

0b. **YAML-aware ARIA snapshot redactor.** The wire op
   (`aria_snapshot`) and raw client method are implemented. A
   masking layer for Playwright's ARIA-snapshot YAML format would
   let stencilwright surface the (much terser) accessibility-tree
   view of authenticated pages. PUNCHLIST CP4.5.

0c. **Stable per-site slot identity.** Today the value→hash map
   dies with the daemon; same value on two days yields different
   `[$<hash> …]` ids unless named in `values.toml`. Optional opt-in
   for stable hashing per-site so unmasked dumps across days line
   up.

0d. **Non-secret 1Password account selector.** Implemented:
   `site.toml` supports `onepassword_account = "my.1password.com"`.
   The daemon passes it to every `op` invocation as `--account`, with
   `OP_ACCOUNT` retained as a non-secret fallback. `session start`
   does not touch the secret provider; first provider contact happens
   inside the daemon path that needs a secret or user-only discovery.

0e. **Approval dialog context.** Implemented: the Iced dialog shows
   site/scope/place, current URL, request reason, selector, proposed
   element name, and relevant signature/matching criteria in addition
   to raw snippets. It also returns optional user feedback to stdout so
   the agent can learn, for example, that a denied private field is a
   balance total rather than a label. On Deny, the CLI leaves TOML
   unchanged and suggests the equivalent masked mapping command. This
   gives the user enough context to approve the intended scope without
   exposing raw text to the agent.

0f. **Secret-provider discovery abstraction.** Implemented in
   `stencil-secrets`: provider-neutral discovery/resolution traits,
   1Password item discovery via `op item list --long --format json`,
   and opaque id-based references of the form
   `secret://1password/<vault-id>/<item-id>/<field>`. A native
   user-only picker command is still pending; do not expose discovery
   metadata through terminal output.

1. **Heuristic auto-unmask rules** (e.g., "non-numeric `<td>` text
   inside an unmasked `<table>` defaults to unmasked"). Manual
   per-element `unmasked = true` covers v1; auto-rules are a
   quality-of-life optimization for v2.

2. **Recognition tie-breaks.** If two places' signatures match the
   current page, default: more components defined wins; ties go to
   the place currently being navigated toward. Revisit if real cases
   produce confusion.

3. **`place add` interactive variant.** Today `place add` emits a
   TOML stub the user fills in. A future variant could recognize
   the live page first and pre-populate the signature.

4. **DOM serialization fidelity.** `Page::content()` returns the
   live DOM serialized; shadow DOM and iframes may need explicit
   traversal. Defer until first site needs it.

5. **Per-place mask overrides.** Sites might want a different
   `max_unmasked_chars` or additional `redact_selectors` per place.
   YAGNI for v1; add `[place.mask]` block when the case appears.

6. **Chrome not installed.** `channel = "chrome"` requires Chrome on
   the user's system. Detect before daemon start and emit a clear error
   if missing; fallback to bundled Chromium with a fingerprint
   warning is a v2 option.

7. **Replay determinism.** Sites have nondeterministic content (ads,
   timestamps, A/B variants). Stencilwright makes no attempt at
   replay determinism; captures are best-effort.
