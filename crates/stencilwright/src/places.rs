//! `place` resource commands.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use stencil_browser::Session;
use stencil_places::recognize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::{
    browser,
    config_lock::{ConfigLock, write_atomic},
};

pub(crate) async fn goto(site: &str, place_name: &str) -> Result<()> {
    let runtime = browser::load_site(site)?;
    let session = browser::attach(site).await?;
    let page = session.page();
    let masked = runtime.graph.place_goto(&page, place_name).await?;

    let captures_dir = runtime.site_dir.join("captures");
    fs::create_dir_all(&captures_dir)?;
    let path = captures_dir.join(format!("{place_name}.html"));
    fs::write(&path, &masked.0)?;
    println!("→ wrote {} ({} bytes)", path.display(), masked.0.len());
    Ok(())
}

pub(crate) async fn add(site: &str, name: &str, selector: Option<&str>) -> Result<()> {
    let runtime = browser::load_site(site)?;
    let session = browser::attach(site).await?;
    let page = session.page();
    let url = page.url_template(&runtime.graph.values).await?;
    if url.is_empty() || url == "about:blank" {
        bail!("current page has no usable URL; run `<site> page goto <url>` first");
    }

    append_place(&runtime.site_dir, name, &url, selector)?;

    println!("captured live state:");
    println!("  url: {}", url);
    match selector {
        Some(selector) => println!("  selector: {selector}"),
        None => println!("  selector: (none; signature is URL-only)"),
    }
    println!(
        "appended [[place]] {name} to {}",
        runtime.site_dir.join("places.toml").display()
    );
    Ok(())
}

pub(crate) async fn list(site: &str) -> Result<()> {
    let runtime = browser::load_site(site)?;
    println!("place\turl\tinteractive\tauto-fill\tunmasked");
    for place in &runtime.graph.places {
        let auto_fill = place
            .elements
            .iter()
            .filter(|el| el.auto_fill.is_some())
            .count();
        let unmasked = place.elements.iter().filter(|el| el.unmasked).count();
        println!(
            "{}\t{}\t{}\t{}\t{}",
            place.name,
            place.url.as_deref().unwrap_or("-"),
            place.interactive,
            auto_fill,
            unmasked,
        );
    }
    if let Some(current) = current_place(&runtime.site_dir, &runtime.graph).await? {
        println!("→ currently at: {current}");
    }
    Ok(())
}

async fn current_place(
    site_dir: &Path,
    graph: &stencil_places::PlaceGraph,
) -> Result<Option<String>> {
    let Some(sock) = stencil_browser::daemon::live_socket(site_dir)? else {
        return Ok(None);
    };
    let session = Session::connect(&sock).await?;
    let page = session.page();
    Ok(recognize::recognize(graph, &page)
        .await?
        .map(|m| m.place_name))
}

fn append_place(site_dir: &Path, name: &str, url: &str, selector: Option<&str>) -> Result<()> {
    let path = site_dir.join("places.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(&path)?;
    let places = ensure_places(&mut doc)?;
    if places
        .iter()
        .any(|table| table.get("name").and_then(Item::as_str) == Some(name))
    {
        bail!("place '{name}' already exists");
    }

    let mut table = Table::new();
    table["name"] = value(name);
    table["url"] = value(url);
    let mut sig = Table::new();
    sig["url"] = value(url);
    if let Some(selector) = selector {
        sig["selector"] = value(selector);
    }
    table["signature"] = Item::Table(sig);
    places.push(table);
    write_doc(&path, &doc)
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    raw.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    write_atomic(path, &doc.to_string())
}

fn ensure_places(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables> {
    if !doc.as_table().contains_key("place") {
        doc.as_table_mut()
            .insert("place", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    doc.as_table_mut()
        .get_mut("place")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| anyhow::anyhow!("place is not an array of tables"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_place_writes_parseable_toml() {
        let tmp = tempdir().unwrap();
        let site = tmp.path();
        fs::write(site.join("places.toml"), "target = \"x\"\n").unwrap();
        fs::write(site.join("elements.toml"), "").unwrap();
        fs::write(site.join("mask.toml"), "").unwrap();
        fs::write(site.join("values.toml"), "").unwrap();

        append_place(site, "dashboard", "https://example.test/", Some("main")).unwrap();

        let graph = stencil_places::PlaceGraph::from_dir(site).unwrap();
        let place = graph.place("dashboard").unwrap();
        assert_eq!(place.url.as_deref(), Some("https://example.test/"));
        assert_eq!(place.signature.selector.as_deref(), Some("main"));
    }
}
