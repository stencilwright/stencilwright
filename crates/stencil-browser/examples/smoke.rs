//! CP3+CP4 end-to-end smoke. Assumes a stencilwright daemon is
//! already running for the site you point at.
//!
//! Procedure:
//!   stencilwright init example                              # if not already
//!   stencilwright session start example                     # opens Chrome
//!   cargo run -p stencil-browser --example smoke -- ~/.stencilwright/example
//!   stencilwright session stop example                      # closes Chrome
//!
//! Expected output: the masked HTML of https://example.com/. Body
//! text appears as `[TEXT:N]` markers (default-deny) and the
//! structural `<h1>`, `<p>`, etc. tags pass through verbatim.

use std::path::PathBuf;

use anyhow::{Context, Result};
use stencil_browser::{Session, daemon};
use stencil_core::{MaskConfig, MaskInner, Place, Signature};
use stencil_mask::{MaskPolicy, ValueNameMap};

#[tokio::main]
async fn main() -> Result<()> {
    let site_dir: PathBuf = std::env::args()
        .nth(1)
        .context("usage: smoke <site-dir> (e.g. ~/.stencilwright/example)")?
        .into();

    // Connect to the already-running daemon.
    let sock = daemon::live_socket(&site_dir)?
        .context("no live daemon for this site; run `stencilwright session start <site>` first")?;
    println!("→ connecting to {}", sock.display());
    let session = Session::connect(&sock).await?;
    let page = session.page();

    println!("→ goto https://example.com/");
    page.goto("https://example.com/").await?;

    let url = page.url().await?;
    println!("→ page.url() = {url}");

    // Default-deny everything (no patterns, no unmask selectors).
    let cfg = MaskConfig {
        mask: MaskInner::default(),
        max_unmasked_chars: 200,
    };
    let policy = MaskPolicy::build(&cfg, &[])?;
    let place = Place {
        name: "smoke".into(),
        url: None,
        from: None,
        via: None,
        interactive: false,
        submit: None,
        signature: Signature::default(),
        completion: None,
        redirect: None,
        elements: vec![],
    };
    let effective = policy.for_place(&place);

    let masked = page.dump(&effective, &ValueNameMap::new()).await?;
    println!("─── masked HTML ({} bytes) ───", masked.0.len());
    println!("{}", masked.0);
    println!("───");

    // Heuristic confirmation: structure preserved, text default-denied.
    let h1_present = masked.0.contains("<h1>");
    let text_redacted = masked.0.contains("[TEXT:");
    let real_text_leaked = masked.0.contains("Example Domain") || masked.0.contains("illustrative");

    println!("checks:");
    println!("  <h1> tag preserved      : {h1_present}");
    println!("  [TEXT:N] markers present: {text_redacted}");
    println!("  raw page text NOT leaked: {}", !real_text_leaked);

    if h1_present && text_redacted && !real_text_leaked {
        println!("smoke OK — structure visible, content masked");
    } else {
        anyhow::bail!("smoke FAILED — see masked HTML above");
    }
    Ok(())
}
