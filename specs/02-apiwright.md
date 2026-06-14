# 02 — apiwright: the adapter runtime

Status: **draft**, skeleton in place.

Companion to [`01-stencil.md`](01-stencil.md) (the mapping harness, same repo)
and to the service adapters (separate repos, e.g.
[`adapter-example`](https://github.com/stencilwright/adapter-example)).

`apiwright` is a crate in this monorepo (`crates/apiwright`).

## 1. Goal

apiwright turns a **stencilwright map** + the **live site** into a clean, typed,
async API. It is the layer every adapter shares: load a site's map, drive the
real browser against raw DOM, recognize where you are, navigate to mapped
places, and extract structured data — all under a visibility model that keeps
the user aware and in control.

apiwright is what `apiwright` was always meant to be in the original
monorepo: "really a core library for the adapters." It was renamed because that
is its job — it lets you *wright* a local **API** out of a web app.

## 2. Relationship to stencilwright

| | stencilwright (binary) | apiwright |
|---|---|---|
| When | build-time (mapping) | run-time |
| Who runs it | developer + LLM, collaboratively | the user (or the user's automation) |
| DOM it sees | **masked** (trust boundary) | **raw** (unmasked) |
| Window | always headed | headed or off-screen-surfaceable |
| `raw` feature | OFF in `-p stencilwright` builds | ON |
| Output | `~/.stencilwright/<site>/*.toml` maps | structured data / an API |

The two never run in the same process. stencilwright writes the map; apiwright
reads it. Both are crates in this workspace and depend on the shared `stencil-*`
crates via `{ workspace = true }` — one lockfile, atomic cross-crate refactors.

### The `raw` boundary in a shared workspace

apiwright enables `stencil-browser`'s `raw` (unmasked-DOM) feature. Because
apiwright and stencilwright live in the **same** Cargo workspace, a
`cargo build/test --workspace` unifies that feature into the single shared
`stencil-browser` build. That does **not** breach the trust boundary, because:

1. The boundary is defined on the **stencilwright binary**, and the binary is
   built/run with `-p stencilwright` (`cargo run -p stencilwright -- …`), where
   `raw` is resolved OFF and `dump_raw` / `selector_text_raw` are not in the
   symbol table.
2. [`crates/stencilwright/tests/feature_gate.rs`](../crates/stencilwright/tests/feature_gate.rs)
   asserts this under `cargo test -p stencilwright`, and explicitly skips under
   `--workspace` (where unification is expected). Verify a built binary directly
   with `nm target/debug/stencilwright | grep dump_raw` (should be empty).

This is the same posture the predecessor monorepo shipped. A separate-workspace
split would make the boundary compile-*impossible* rather than
test-*enforced* — a future option if a published, widely-depended `apiwright`
ever warrants it, but unnecessary friction now.

## 3. Visibility & consent model

The load-bearing principle of the whole project: **you may automate anything in
your own name, so long as it is not surprising.** apiwright operationalizes that
in three rules:

1. **Never truly headless.** A real OS browser window always exists. "Headless"
   is not offered; the weakest setting is *off-screen*, which can always be
   surfaced.
2. **Headed by default.** [`Visibility::Headed`] — a visible, focused window —
   is the default. Off-screen ([`Visibility::Offscreen`]) is opt-in for
   batch/unattended runs.
3. **Surface on anything a human should see.** An off-screen session is pulled
   to the foreground per [`SurfacePolicy`] when the runner hits a
   [`SurfaceTrigger`]: `Login`, `Captcha`, `Unrecognized`, `Consent`, or an
   explicit `Requested`. Defaults surface on all of the first four; `Requested`
   always surfaces. `SurfacePolicy::unattended()` opts out of auto-surfacing for
   true background runs (the user accepts they won't be prompted).

Why off-screen and not headless: captcha solving, SSO/2FA, and ad-hoc oversight
must remain possible at any moment. A window you can move on-screen preserves
that; a headless context destroys it.

### Off-screen mechanism (open question)

Candidate implementations, in rough order of preference (see §7):
- position the window at large negative coordinates / on an unused virtual
  desktop, then re-position + focus to surface;
- native minimize/hide then un-minimize;
- a virtual display as a fallback.

The chosen mechanism must support a fast, reliable *surface* that also brings
the window to focus so the user can act (type a 2FA code, click a captcha).

## 4. Session lifecycle

```text
RuntimeConfig::new(site)            select map, headed by default
  [.offscreen()] [.surface_policy()]
        │
        ▼
AdapterSession::open(cfg)           attach raw daemon + load PlaceGraph
        │
        ├─ goto_place("…")          recognize-first; navigate on miss;
        │                           auto-surface on Login/Captcha/Unrecognized
        ├─ extract_text("sel")      raw selector text at current place
        ├─ surface() / maybe_surface(trigger)
        │
        ▼
   (dropped)                        daemon stays warm for the next call
```

The daemon is the same long-lived, real-Chrome, anti-fresh-launch session
`stencil-browser` already provides; apiwright attaches with `raw` enabled.

## 5. Extraction primitives

Adapters need more than single-selector reads. apiwright provides (incrementally):

- `extract_text(selector)` — raw text of matching nodes at the current place.
- **Structured extraction** over a place's mapped `[[place.element]]` set:
  return a row per repeated container with each named element's value.
- **List / virtualized collection** helper: scroll a container, collect rows as
  they materialize, dedup by a stable key, and stop on a configurable
  end-of-list signal (no new rows after N scrolls, or an explicit "no results"
  marker). This is the workhorse for search results, feeds, and tables.

## 6. Public API (Rust)

```rust
pub enum Visibility { Headed, Offscreen }
pub enum SurfaceTrigger { Login, Captcha, Unrecognized, Consent, Requested }
pub struct SurfacePolicy { /* on_login, on_captcha, on_unrecognized, on_consent */ }

pub struct RuntimeConfig { pub site: String, pub visibility: Visibility, pub surface: SurfacePolicy }
impl RuntimeConfig {
    pub fn new(site: impl Into<String>) -> Self;
    pub fn offscreen(self) -> Self;
    pub fn surface_policy(self, p: SurfacePolicy) -> Self;
}

pub struct AdapterSession { /* … */ }
impl AdapterSession {
    pub async fn open(cfg: RuntimeConfig) -> anyhow::Result<Self>;
    pub async fn surface(&self) -> anyhow::Result<()>;
    pub async fn maybe_surface(&self, t: SurfaceTrigger) -> anyhow::Result<bool>;
    pub async fn goto_place(&self, place: &str) -> anyhow::Result<()>;
    pub async fn extract_text(&self, selector: &str) -> anyhow::Result<Vec<String>>;
}
```

The stencil crates are re-exported (`apiwright::stencil_places`, etc.) so an
adapter depends on `apiwright` alone.

## 7. Open questions / deferred

1. **Off-screen mechanism** (§3) — pick and validate one on macOS first.
2. **Captcha / login recognition** — likely per-site (just another mapped
   interactive place), with apiwright surfacing when such a place is recognized.
3. **Consent checkpoints** — start with `maybe_surface(Consent)` called
   explicitly by adapters; revisit a declarative ("confirm before this place")
   form later.
4. **Action logging / audit trail** — a structured log of navigations and
   extractions for after-the-fact review ("what did it do in my name?").
5. **Map portability** — consume a `stencilwright export` bundle so adapters in
   other repos don't need a sibling stencilwright checkout.
6. **Compile-impossible boundary** — revisit a separate-workspace split for
   `apiwright` if it is ever published and widely depended upon (§2).
