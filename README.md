# stencilwright

**Privacy-preserving web-mapping harness.** Drive an authenticated web app in a
real browser, capture its DOM with every *specific value* masked by default, and
write reusable **map artifacts** (places, elements, selectors, secret
references) that runtime adapters later consume.

stencilwright is the **build-time / mapping** half of an automation toolchain
whose mission is simple: **freedom of automation** — you should be able to
automate anything in your own name, so long as it isn't surprising.

```
 stencilwright            apiwright                 *-adapter
 (this repo)              (runtime lib)             (one per service)
 ───────────────         ───────────────           ───────────────
 map a site, masked,     consume the map,          ergonomic API over
 collaboratively  ─────▶ drive raw DOM,     ─────▶ a walled-garden web
 (LLM-safe)              headed/surfaceable        app (e.g. Acme)
```

- **stencilwright** (this repo) — map a site *with an LLM collaborator* without
  leaking the user's real values into transcripts. Always headed; masking always
  on. Output: `~/.stencilwright/<site>/{places,elements,mask,values}.toml`.
- **apiwright** (`crates/apiwright`) — the **runtime** library. Consumes the
  maps stencilwright produces and drives the site against raw (unmasked) DOM.
  Adapters are built on it. It enables `stencil-browser`'s `raw` feature; the
  stencilwright binary stays raw-free when built with `-p stencilwright`
  (see [specs/02-apiwright.md](specs/02-apiwright.md)).
- **adapters** (e.g. [adapter-example](https://github.com/stencilwright/adapter-example))
  — turn one walled-garden web app into an easy local API.

## Why a separate mapping tool?

An LLM can help develop adapters for authenticated sites *only* if it never sees
the user's specific real values (account numbers, balances, message contents).
Every page an assistant ingests is logged. stencilwright masks all text **and
content-bearing attributes** by default — page *structure* passes through (tags,
classes, ids, roles, structural data attributes), while specific values become
typed, length-tagged slots — with per-element opt-in unmasking gated by a native
approval dialog the **user** clicks (the agent never sees the raw text). See
[specs/01-stencil.md](specs/01-stencil.md) for the full trust model.

## Crates

| crate | role |
|---|---|
| `stencil-core` | shared types (Place, Element, Signature, Slot…), no I/O |
| `stencil-mask` | `lol_html` masking, describer pipeline, slot derivation |
| `stencil-browser` | `playwright-rs` wrapper, session daemon, RPC, masked `Page`. `raw` + `approval-dialog` features |
| `stencil-secrets` | secret-provider references / discovery (1Password via `op`) |
| `stencil-places` | place graph, recognition, `place_goto` runner |
| `apiwright` | the adapter **runtime** lib — drive a mapped site against raw DOM, headed/surfaceable. `raw` ON |
| `stencilwright` | the dev binary: `init` / `place` / `element` / `page` / `value`. Always headed, `raw` OFF |

## Quickstart

```sh
cargo run -p stencilwright -- init <site>
cargo run -p stencilwright -- <site> place add <name> [selector]
cargo run -p stencilwright -- <site> place <name> goto      # opens Chrome, masks, dumps
```

Full workflow and artifact formats: [specs/01-stencil.md](specs/01-stencil.md).

## Status

Extracted (copied) from a private monorepo where the harness reached its eighth
checkpoint: end-to-end Example mapping from a blank profile, masked captures with
verified zero PII leakage, and the native unmask-approval cycle exercised.

Since then: the masker was hardened to default-deny **content-bearing
attributes** (`aria-label`, `title`, `data-stringify-text`, … — a leak class
found while mapping Acme), and the first real adapter,
[adapter-example](https://github.com/stencilwright/adapter-example), searches Acme
end-to-end. Open follow-ups: [issues](https://github.com/stencilwright/stencilwright/issues).

## License

MIT
