//! Dump masked HTML of the daemon's CURRENT page (no navigation).
//!
//! Use when the runner can't yet handle a flow — e.g. Example serves
//! a CAPTCHA inline at the target URL — and you need to read the
//! masked DOM to design a new place's signature.
//!
//! Usage:
//!   OP_ACCOUNT=… cargo run -p stencilwright --example dump -- ~/.stencilwright/example captcha
//!
//! Writes to `<site>/captures/<name>.html`. Sends the site's mask
//! policy + values.toml references to the daemon so the dump uses
//! the same masking rules as a real `place goto`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use stencil_browser::{Session, daemon};
use stencil_places::PlaceGraph;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let site_dir: PathBuf = args
        .next()
        .context("usage: dump <site-dir> [output-name]")?
        .into();
    let out_name = args.next().unwrap_or_else(|| "_dump".to_string());

    let sock = daemon::live_socket(&site_dir)?
        .context("no live daemon; start one with `stencilwright session start <site>`")?;
    let session = Session::connect(&sock).await?;
    let page = session.page();

    let url = page.url().await?;
    println!("→ current url: {url}");

    let graph = PlaceGraph::from_dir(&site_dir)?;
    let masked = page
        .dump_masked(
            &graph.mask_config,
            &graph.site_elements,
            None,
            &graph.values,
        )
        .await?;

    let captures_dir = site_dir.join("captures");
    std::fs::create_dir_all(&captures_dir)?;
    let path = captures_dir.join(format!("{out_name}.html"));
    std::fs::write(&path, &masked.0)?;
    println!("→ wrote {} ({} bytes)", path.display(), masked.0.len());
    Ok(())
}
