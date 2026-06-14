# Coding standards

Short, load-bearing conventions for this workspace. The privacy
constraints in [`CLAUDE.md`](../CLAUDE.md) and the architectural
specs in `specs/01-stencil.md` / `specs/02-positions-aggregator.md`
take precedence; this document covers code organization only.

## File size

Keep source files under ~300 lines. When a file grows past that,
split it before adding more — splitting later is harder because
imports and visibility have to be reworked.

The threshold is a smell, not a hard limit. A 320-line file that
genuinely is one cohesive thing (a parser, a state machine) is
fine. A 280-line file covering three unrelated concerns should be
split now.

## One struct, one file

A struct's definition and its `impl` blocks live in the same file.
Never `pub struct Foo` in `a.rs` and `impl Foo` in `b.rs`. Splitting
makes the surface area of `Foo` impossible to see at a glance and
invites duplicate definitions.

If a file with one struct grows too large, the answer is **not** to
move impl blocks elsewhere. The answer is to **decompose into
multiple structs**, each with their own file:

- A `Daemon` struct that handled session-file IO + lifecycle +
  the daemon body became three structs (`SessionInfo` /
  `FlockGuard` / the daemon `run` function), one per file, each
  with its own focused concern.
- If a single type genuinely needs >300 lines of impl, that's a
  signal it's doing too much and the *type* needs to be split, not
  the impl.

Trait impls for foreign types (`impl Display for MyError`) belong
with `MyError`, not with the trait.

## Separation of concerns

Each module / file has exactly one reason to change. Concrete tests:

- "What does this file do?" — one sentence answer, no `and`.
- "If requirement X changes, which files do I touch?" — ideally
  one. If three files always change together, they're one concern
  pretending to be three.
- "Could I describe this module to a new contributor without
  referencing the others?" — if not, the boundary is wrong.

When a module grows multiple concerns, prefer a submodule
directory (`foo.rs` + `foo/bar.rs`) over a single fat file. The
parent file becomes a thin re-export surface; submodules carry the
implementation.

## Where tests live

- **Unit tests** for purely-internal helpers: `#[cfg(test)] mod
  tests` at the bottom of the module. Use when the test needs
  access to private items.
- **Integration tests** in `crates/<crate>/tests/<topic>.rs`.
  Default to integration tests when the public API alone is enough
  to exercise the behavior — the test then doubles as documentation
  of the public surface and resists drift.
- A test file follows the same size rules. Split by topic
  (`tests/daemon.rs`, `tests/init.rs`) rather than by source file.

## Public API hygiene

Mark items `pub` only when they cross a crate boundary. Within a
crate use `pub(crate)` or `pub(super)` so the surface for external
consumers stays small and intentional. The crate-level trust
boundary in `stencil-browser` (raw vs. masked) depends on this:
`Page::dump_raw` is `pub` only under `#[cfg(feature = "raw")]`,
and `stencilwright`'s Cargo.toml does not enable that feature.

## Comments

The defaults from `CLAUDE.md` apply: write a comment only when the
*why* is non-obvious. No "what does this code do" narration, no
"used by X" referrers, no rotting cross-references. If you find
yourself wanting to write a comment block longer than three lines,
the code probably wants restructuring instead.

## Dependencies

Add a workspace dependency only when something needs it. Three
similar inline implementations are better than a dep that exists
"for the third one we'll write someday." When pulling in a crate,
prefer narrow ones (`libc` for syscalls) over kitchen-sink ones
(`nix`, `daemonize`) unless we'll use the breadth.
