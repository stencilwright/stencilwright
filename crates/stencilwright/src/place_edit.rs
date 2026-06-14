//! Place metadata edits for CLI-only mapping.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::cli::{PlaceSetArgs, SignatureSetArgs};
use crate::config_lock::{ConfigLock, write_atomic};

pub(crate) fn set(site_dir: &Path, place: &str, args: PlaceSetArgs) -> Result<()> {
    let path = site_dir.join("places.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(&path)?;
    let table = place_mut(&mut doc, place)?;

    if let Some(interactive) = args.interactive {
        if interactive {
            table["interactive"] = value(true);
        } else {
            table.remove("interactive");
        }
    }
    if args.clear_url {
        table.remove("url");
    }
    if let Some(url) = args.url {
        table["url"] = value(url);
    }
    if args.clear_redirect {
        table.remove("redirect");
    }
    if let Some(redirect) = args.redirect {
        table["redirect"] = value(redirect);
    }
    if args.clear_submit {
        table.remove("submit");
    }
    if let Some(click) = args.submit_click {
        let mut submit = Table::new();
        submit["click"] = value(click);
        table["submit"] = Item::Table(submit);
    }

    write_doc(&path, &doc)?;
    println!("updated [[place]] {place}");
    Ok(())
}

pub(crate) fn set_signature(site_dir: &Path, place: &str, args: SignatureSetArgs) -> Result<()> {
    set_signature_table(site_dir, place, "signature", args)?;
    println!("updated signature for {place}");
    Ok(())
}

pub(crate) fn set_completion(site_dir: &Path, place: &str, args: SignatureSetArgs) -> Result<()> {
    set_signature_table(site_dir, place, "completion", args)?;
    println!("updated completion for {place}");
    Ok(())
}

pub(crate) fn clear_completion(site_dir: &Path, place: &str) -> Result<()> {
    let path = site_dir.join("places.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(&path)?;
    place_mut(&mut doc, place)?.remove("completion");
    write_doc(&path, &doc)?;
    println!("cleared completion for {place}");
    Ok(())
}

fn set_signature_table(
    site_dir: &Path,
    place: &str,
    key: &str,
    args: SignatureSetArgs,
) -> Result<()> {
    let path = site_dir.join("places.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(&path)?;
    let sig = ensure_table(place_mut(&mut doc, place)?, key)?;

    set_or_clear(sig, "url", args.url, args.clear_url);
    set_or_clear(sig, "selector", args.selector, args.clear_selector);
    set_or_clear(
        sig,
        "visible_selector",
        args.visible_selector,
        args.clear_visible_selector,
    );
    set_or_clear(
        sig,
        "absent_selector",
        args.absent_selector,
        args.clear_absent_selector,
    );
    set_or_clear(sig, "text", args.text, args.clear_text);

    write_doc(&path, &doc)
}

fn set_or_clear(table: &mut Table, key: &str, val: Option<String>, clear: bool) {
    if clear {
        table.remove(key);
    }
    if let Some(val) = val {
        table[key] = value(val);
    }
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    raw.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    write_atomic(path, &doc.to_string())
}

fn place_mut<'a>(doc: &'a mut DocumentMut, name: &str) -> Result<&'a mut Table> {
    let places = doc
        .as_table_mut()
        .get_mut("place")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| anyhow::anyhow!("places.toml has no [[place]] entries"))?;
    for place in places.iter_mut() {
        if place.get("name").and_then(Item::as_str) == Some(name) {
            return Ok(place);
        }
    }
    bail!("unknown place: '{name}'")
}

fn ensure_table<'a>(table: &'a mut Table, key: &str) -> Result<&'a mut Table> {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    table
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("{key} is not a table"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{PlaceSetArgs, SignatureSetArgs};
    use tempfile::tempdir;

    #[test]
    fn set_place_metadata_and_completion_round_trip() {
        let tmp = tempdir().unwrap();
        let site = tmp.path();
        fs::write(
            site.join("places.toml"),
            r#"
[[place]]
name = "login"
url = "https://www.example.com/login/"
signature.selector = "app-form#login"
"#,
        )
        .unwrap();

        set(
            site,
            "login",
            PlaceSetArgs {
                interactive: Some(true),
                url: None,
                clear_url: false,
                redirect: None,
                clear_redirect: false,
                submit_click: Some("button.login".into()),
                clear_submit: false,
            },
        )
        .unwrap();
        set_completion(
            site,
            "login",
            SignatureSetArgs {
                url: Some("https://www.example.com/".into()),
                selector: None,
                visible_selector: Some("main".into()),
                absent_selector: None,
                text: None,
                clear_url: false,
                clear_selector: false,
                clear_visible_selector: false,
                clear_absent_selector: false,
                clear_text: false,
            },
        )
        .unwrap();

        fs::write(site.join("elements.toml"), "").unwrap();
        fs::write(site.join("mask.toml"), "").unwrap();
        fs::write(site.join("values.toml"), "").unwrap();
        let graph = stencil_places::PlaceGraph::from_dir(site).unwrap();
        let place = graph.place("login").unwrap();
        assert!(place.interactive);
        assert_eq!(
            place.submit.as_ref().unwrap().click.as_deref(),
            Some("button.login")
        );
        assert_eq!(
            place.completion.as_ref().unwrap().url.as_deref(),
            Some("https://www.example.com/"),
        );
        assert_eq!(
            place
                .completion
                .as_ref()
                .unwrap()
                .visible_selector
                .as_deref(),
            Some("main"),
        );
    }
}
