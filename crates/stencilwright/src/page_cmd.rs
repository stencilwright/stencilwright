//! Free-form live page commands.

use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::browser;

pub(crate) async fn goto(site: &str, url: &str) -> Result<()> {
    let session = browser::attach(site).await?;
    session.page().goto(url).await?;
    println!("goto {url}");
    Ok(())
}

pub(crate) async fn click(site: &str, selector: &str) -> Result<()> {
    let session = browser::attach(site).await?;
    session.page().click(selector).await?;
    println!("clicked '{selector}'");
    Ok(())
}

pub(crate) async fn fill(site: &str, selector: &str, value: &str) -> Result<()> {
    let runtime = browser::load_site(site)?;
    let session = browser::attach(site).await?;
    if is_secret_ref(value) {
        session
            .page()
            .fill_ref(selector, value, &runtime.graph.values)
            .await?;
    } else {
        session.page().fill(selector, value).await?;
    }
    println!("filled '{selector}'");
    Ok(())
}

pub(crate) async fn dump(site: &str, cwd: &Path) -> Result<()> {
    let runtime = browser::load_site(site)?;
    let session = browser::attach(site).await?;
    let masked = session
        .page()
        .dump_masked(
            &runtime.graph.mask_config,
            &runtime.graph.site_elements,
            None,
            &runtime.graph.values,
        )
        .await?;
    let path = cwd.join("_page.html");
    fs::write(&path, &masked.0)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn is_secret_ref(value: &str) -> bool {
    stencil_secrets::is_secret_reference(value)
        || value
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .is_some()
}
