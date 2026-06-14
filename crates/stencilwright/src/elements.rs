//! `element` resource commands and TOML round-trips.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::config_lock::{ConfigLock, write_atomic};

pub(crate) struct ElementInput<'a> {
    pub selector: &'a str,
    pub name: Option<&'a str>,
    pub auto_fill: Option<&'a str>,
    pub unmasked: bool,
}

pub(crate) fn add_site(site_dir: &Path, input: ElementInput<'_>) -> Result<()> {
    let path = site_dir.join("elements.toml");
    add_to_doc(&path, Scope::Site, input)
}

pub(crate) fn add_place(site_dir: &Path, place: &str, input: ElementInput<'_>) -> Result<()> {
    let path = site_dir.join("places.toml");
    add_to_doc(&path, Scope::Place(place), input)
}

pub(crate) fn unmask_place(site_dir: &Path, place: &str, selector: &str) -> Result<()> {
    let path = site_dir.join("places.toml");
    unmask_in_doc(&path, Scope::Place(place), selector)
}

pub(crate) fn list_site(graph: &stencil_places::PlaceGraph) {
    println!("name\tselector\tauto_fill\tunmasked");
    for el in &graph.site_elements {
        println!(
            "{}\t{}\t{}\t{}",
            el.name,
            el.selector,
            el.auto_fill.as_deref().unwrap_or("-"),
            el.unmasked,
        );
    }
}

pub(crate) fn list_place(graph: &stencil_places::PlaceGraph, place: &str) -> Result<()> {
    let place = graph
        .place(place)
        .ok_or_else(|| anyhow::anyhow!("unknown place: '{place}'"))?;
    println!("name\tselector\tauto_fill\tunmasked");
    for el in &place.elements {
        println!(
            "{}\t{}\t{}\t{}",
            el.name,
            el.selector,
            el.auto_fill.as_deref().unwrap_or("-"),
            el.unmasked,
        );
    }
    Ok(())
}

pub(crate) fn needs_unmask(site_dir: &Path, scope: Scope<'_>, selector: &str) -> Result<bool> {
    let path = match scope {
        Scope::Site => site_dir.join("elements.toml"),
        Scope::Place(_) => site_dir.join("places.toml"),
    };
    let mut doc = read_doc(&path)?;
    let aot = element_tables_mut(&mut doc, scope)?;
    if let Some(table) = find_by_selector_mut(aot, selector) {
        return Ok(!is_unmasked(table));
    }
    Ok(true)
}

#[derive(Clone, Copy)]
pub(crate) enum Scope<'a> {
    Site,
    Place(&'a str),
}

fn add_to_doc(path: &Path, scope: Scope<'_>, input: ElementInput<'_>) -> Result<()> {
    let site_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent: {}", path.display()))?;
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(path)?;
    let aot = element_tables_mut(&mut doc, scope)?;

    if let Some(existing) = find_by_selector_mut(aot, input.selector) {
        validate_existing(existing, &input)?;
        if input.unmasked && !is_unmasked(existing) {
            set_unmasked(existing);
            write_doc(path, &doc)?;
            println!("updated existing element for selector '{}'", input.selector);
        } else {
            println!(
                "element for selector '{}' already exists; no change",
                input.selector
            );
        }
        return Ok(());
    }

    let name = input
        .name
        .map(str::to_string)
        .unwrap_or_else(|| format!("auto_{}", aot.len()));
    let mut table = Table::new();
    table["name"] = value(name.clone());
    table["selector"] = value(input.selector);
    if let Some(auto_fill) = input.auto_fill {
        table["auto_fill"] = value(auto_fill);
    }
    if input.unmasked {
        table["unmasked"] = value(true);
    }
    aot.push(table);
    write_doc(path, &doc)?;
    println!("appended element {name} to {}", path.display());
    Ok(())
}

fn unmask_in_doc(path: &Path, scope: Scope<'_>, selector: &str) -> Result<()> {
    let site_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent: {}", path.display()))?;
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(path)?;
    let aot = element_tables_mut(&mut doc, scope)?;
    let Some(table) = find_by_selector_mut(aot, selector) else {
        bail!("no element with selector '{selector}'");
    };
    if is_unmasked(table) {
        println!("element for selector '{selector}' is already unmasked");
        return Ok(());
    }
    set_unmasked(table);
    write_doc(path, &doc)?;
    println!("unmasked element for selector '{selector}'");
    Ok(())
}

fn validate_existing(table: &Table, input: &ElementInput<'_>) -> Result<()> {
    if let Some(name) = input.name {
        let existing = table.get("name").and_then(Item::as_str).unwrap_or("");
        if existing != name {
            bail!(
                "selector '{}' already exists as '{}', not '{}'",
                input.selector,
                existing,
                name,
            );
        }
    }
    if let Some(auto_fill) = input.auto_fill {
        let existing = table.get("auto_fill").and_then(Item::as_str);
        if existing != Some(auto_fill) {
            bail!(
                "selector '{}' already exists with different auto_fill",
                input.selector
            );
        }
    }
    Ok(())
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    raw.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))
}
fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    write_atomic(path, &doc.to_string())
}

fn element_tables_mut<'a>(
    doc: &'a mut DocumentMut,
    scope: Scope<'_>,
) -> Result<&'a mut ArrayOfTables> {
    match scope {
        Scope::Site => ensure_doc_aot(doc, "element", "element"),
        Scope::Place(place_name) => {
            let places = doc
                .as_table_mut()
                .get_mut("place")
                .and_then(Item::as_array_of_tables_mut)
                .ok_or_else(|| anyhow::anyhow!("places.toml has no [[place]] entries"))?;
            for place in places.iter_mut() {
                if place.get("name").and_then(Item::as_str) == Some(place_name) {
                    return ensure_table_aot(place, "element", "place.element");
                }
            }
            bail!("unknown place: '{place_name}'")
        }
    }
}

fn ensure_doc_aot<'a>(
    doc: &'a mut DocumentMut,
    key: &str,
    label: &str,
) -> Result<&'a mut ArrayOfTables> {
    if !doc.as_table().contains_key(key) {
        doc.as_table_mut()
            .insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    doc.as_table_mut()
        .get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| anyhow::anyhow!("{label} is not an array of tables"))
}

fn ensure_table_aot<'a>(
    table: &'a mut Table,
    key: &str,
    label: &str,
) -> Result<&'a mut ArrayOfTables> {
    if !table.contains_key(key) {
        table.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    table
        .get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| anyhow::anyhow!("{label} is not an array of tables"))
}

fn find_by_selector_mut<'a>(aot: &'a mut ArrayOfTables, selector: &str) -> Option<&'a mut Table> {
    aot.iter_mut()
        .find(|table| table.get("selector").and_then(Item::as_str) == Some(selector))
}

fn is_unmasked(table: &Table) -> bool {
    table
        .get("unmasked")
        .and_then(Item::as_bool)
        .or_else(|| table.get("reveal_text").and_then(Item::as_bool))
        .unwrap_or(false)
}

fn set_unmasked(table: &mut Table) {
    table.remove("reveal_text");
    table["unmasked"] = value(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn add_place_element_writes_unmasked_flag() {
        let tmp = tempdir().unwrap();
        let site = tmp.path();
        fs::write(
            site.join("places.toml"),
            r#"
[[place]]
name = "dashboard"
signature.selector = "main"
"#,
        )
        .unwrap();

        add_place(
            site,
            "dashboard",
            ElementInput {
                selector: "table.accounts th",
                name: Some("headers"),
                auto_fill: None,
                unmasked: true,
            },
        )
        .unwrap();

        fs::write(site.join("elements.toml"), "").unwrap();
        fs::write(site.join("mask.toml"), "").unwrap();
        fs::write(site.join("values.toml"), "").unwrap();
        let graph = stencil_places::PlaceGraph::from_dir(site).unwrap();
        let el = &graph.place("dashboard").unwrap().elements[0];
        assert_eq!(el.name, "headers");
        assert!(el.unmasked);
    }

    #[test]
    fn old_reveal_text_counts_as_unmasked() {
        let tmp = tempdir().unwrap();
        let site = tmp.path();
        fs::write(
            site.join("places.toml"),
            r#"
[[place]]
name = "dashboard"
signature.selector = "main"

  [[place.element]]
  name = "headers"
  selector = "table.accounts th"
  reveal_text = true
"#,
        )
        .unwrap();

        assert!(
            !needs_unmask(site, Scope::Place("dashboard"), "table.accounts th").unwrap(),
            "legacy reveal_text should be treated as already unmasked",
        );
    }
}
