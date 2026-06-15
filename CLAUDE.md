# stencilwright — working notes for agents

`stencilwright` is a privacy-preserving structure-mapping harness. An LLM uses it to
map authenticated web apps **without ever seeing the user's specific real
values**. Read [specs/01-stencil.md](specs/01-stencil.md) before proposing
architectural changes; also see
[specs/coding-standards.md](specs/coding-standards.md),
[specs/field-notes.md](specs/field-notes.md), and
[specs/example-scenario-map.md](specs/example-scenario-map.md).

## Non-negotiable constraints

- The agent must never see the user's real values. **Masked captures are the
  trust boundary.** Read stencilwright's masked outputs (under
  `~/.stencilwright/<site>/captures/`), never raw browser captures or any
  `apiwright`/adapter output.
- Don't ask the user to paste secrets or real account data into chat.
- If masking leaks, **stop, name the leak class, and fix the redaction rule** —
  do not repeat the leaked content.
- `stencilwright` is **always headed**; the `raw` feature stays **off** in this
  binary (it is not in the symbol table — that's the compile-time half of the
  trust boundary).
- Unmasking is gated by the native Iced approval dialog. The dialog shows the
  user real text as pixels and returns only Approve/Deny (+ optional feedback)
  to the CLI. Never edit `unmasked = true` into a TOML by hand — the dialog click
  *is* the gate.
- Secret-provider access (1Password via `op`) is daemon-owned. The short-lived
  CLI never contacts the provider.

## Repo boundary

This repo is **one workspace**: the mapping tool (`stencilwright`), the runtime
lib (`apiwright`), and the shared `stencil-*` crates. `apiwright` enables
`stencil-browser`'s `raw` (unmasked-DOM) feature, so `--workspace` builds unify
`raw` into the shared `stencil-browser`. The trust boundary is held by
building/testing the dev binary with **`-p stencilwright`** (raw OFF) plus
[`crates/stencilwright/tests/feature_gate.rs`](crates/stencilwright/tests/feature_gate.rs).
Service adapters (e.g. adapter-example) are separate repos that depend on
`apiwright`.

## Verify

```sh
cargo check --workspace
cargo test  --workspace
```
