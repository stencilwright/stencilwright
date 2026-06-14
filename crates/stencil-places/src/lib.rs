//! Place graph: load mapping artifacts, recognize the current page,
//! and run the recognition+recovery loop that produces masked
//! captures of named places.
//!
//! See `specs/01-stencil.md` §5 for the runner pseudocode and §6 for
//! the artifact formats.

pub mod graph;
pub mod recognize;
pub mod runner;
mod url_match;

pub use graph::PlaceGraph;
pub use recognize::PlaceMatch;
