//! Masking pipeline. See `specs/01-stencil.md` §4.
//!
//! Layers, in order of application to each text node:
//!   1. Always-redact selectors (mask.toml `redact_selectors`)
//!   2. Numeric blacklist (mask.toml `pattern` regex set)
//!   3. Default-deny on text content → `[TEXT:<len>]`
//!   4. Per-element unmask (`unmasked = true`) — text passes
//!      verbatim except substrings still hit layer 2
//!   5. Length cap on unmasked text (`max_unmasked_chars`)
//!
//! Slot derivation:
//!   `[$<id> <description>]` where `<id>` is the user-given name
//!   from values.toml resolution, or `sha256(value)[:8]` otherwise,
//!   and `<description>` comes from a small fixed describer pipeline.

pub mod describer;
pub mod policy;
pub mod slot;

pub use policy::{EffectivePolicy, MaskPolicy, MaskedHtml, RawHtml};
pub use slot::{ValueNameMap, derive_slot};
