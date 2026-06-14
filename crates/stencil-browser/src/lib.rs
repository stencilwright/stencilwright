//! Playwright-rs wrapper + session daemon + Unix-socket RPC + masked
//! Page API.
//!
//! See `specs/01-stencil.md` §5 for architecture (auto-start daemon,
//! `channel = "chrome"`, `ignore_default_args = ["--enable-automation"]`).
//!
//! `dump_raw` and `aria_snapshot_raw` are gated behind
//! `#[cfg(feature = "raw")]` and are the audited paths for unmasked
//! content. `stencilwright` does not enable the feature; `apiwright`
//! and provider crates do.

mod approval;
pub mod daemon;
pub mod page;
pub mod rpc;
mod secrets;
pub mod session;
pub mod site_config;

pub use page::Page;
pub use session::Session;

#[cfg(feature = "raw")]
pub use approval::RawSnippet;
#[cfg(feature = "raw")]
pub use page::RawAccess;

/// True iff this build of `stencil-browser` was compiled with the
/// `raw` feature enabled. Downstream binaries that must NOT pull in
/// `dump_raw` / `aria_snapshot_raw` (notably `stencilwright`)
/// can reference this in a guard test.
///
/// Caveat: under `cargo test --workspace` cargo may unify features
/// across workspace members, so this reads `true` even from
/// stencilwright's tests when apiwright (which enables `raw`) is
/// in the same compilation. The production guarantee comes from
/// `cargo build -p stencilwright` resolving features in isolation —
/// verify with `nm target/debug/stencilwright | grep dump_raw`
/// (should be empty).
pub const COMPILED_WITH_RAW: bool = cfg!(feature = "raw");
