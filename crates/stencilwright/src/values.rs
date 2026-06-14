//! `value` resource commands.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use stencil_secrets::{DiscoveredSecretItem, SecretDiscoveryQuery};
use toml_edit::{DocumentMut, value};

use crate::browser;
use crate::cli::ValueSearchArgs;
use crate::config_lock::{ConfigLock, write_atomic};

pub(crate) fn add(site_dir: &Path, name: &str, reference: &str) -> Result<()> {
    validate_name(name)?;
    validate_secret_reference(reference)?;
    let path = site_dir.join("values.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(&path)?;
    if doc.contains_key(name) {
        bail!("value '{name}' already exists");
    }
    doc[name] = value(reference);
    write_doc(&path, &doc)?;
    println!("appended {name} to {}", path.display());
    Ok(())
}

pub(crate) fn list(site_dir: &Path) -> Result<()> {
    let path = site_dir.join("values.toml");
    let doc = read_doc(&path)?;
    if doc.is_empty() {
        println!("no values");
        return Ok(());
    }
    println!("name\treference");
    for (name, item) in doc.iter() {
        if let Some(reference) = item.as_str() {
            println!("{name}\t{reference}");
        }
    }
    Ok(())
}

pub(crate) fn remove(site_dir: &Path, name: &str) -> Result<()> {
    let path = site_dir.join("values.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc(&path)?;
    if doc.remove(name).is_none() {
        bail!("value '{name}' does not exist");
    }
    write_doc(&path, &doc)?;
    println!("removed {name} from {}", path.display());
    Ok(())
}

pub(crate) async fn search(site: &str, args: ValueSearchArgs) -> Result<()> {
    let query_text = args.query.trim();
    if query_text.len() < 2 {
        bail!("value search query must be at least 2 characters");
    }
    let categories = search_categories(&args);
    let query = SecretDiscoveryQuery {
        search: Some(query_text.to_string()),
        vault: args.vault.filter(|vault| !vault.trim().is_empty()),
        categories,
    };
    let limit = args.limit.clamp(1, 25);
    let session = browser::attach(site).await?;
    let matches = session.discover_secrets(&query, limit).await?;
    print_search_matches(&matches);
    Ok(())
}

fn search_categories(args: &ValueSearchArgs) -> Vec<String> {
    if args.all_categories {
        vec![]
    } else if args.categories.is_empty() {
        vec!["Login".to_string()]
    } else {
        args.categories.clone()
    }
}

fn print_search_matches(matches: &[DiscoveredSecretItem]) {
    if matches.is_empty() {
        println!("no matches");
        return;
    }
    for (idx, item) in matches.iter().enumerate() {
        println!("match {}", idx + 1);
        println!("  title: {}", item.title());
        println!("  vault: {}", item.vault_name().unwrap_or(item.vault_id()));
        println!("  vault_id: {}", item.vault_id());
        println!("  item_id: {}", item.item_id());
        if let Some(category) = item.category() {
            println!("  category: {category}");
        }
        if let Some(updated_at) = item.updated_at() {
            println!("  updated_at: {updated_at}");
        }
        if !item.urls().is_empty() {
            println!("  urls:");
            for url in item.urls() {
                let label = url.label().unwrap_or("url");
                let primary = if url.primary() { " primary" } else { "" };
                println!("    - {label}{primary}: {}", url.href());
            }
        }
        println!("  references:");
        for reference in item.references() {
            println!("    {}: {}", reference.field(), reference.reference());
        }
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

fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        bail!("value name cannot be empty");
    };
    if !(first.is_ascii_lowercase() || first == '_') {
        bail!("value name must start with lowercase ASCII or underscore");
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        bail!("value name must be snake_case ASCII");
    }
    Ok(())
}

fn validate_secret_reference(reference: &str) -> Result<()> {
    stencil_secrets::validate_reference(reference).map_err(|err| {
        anyhow::anyhow!(
            "value reference must be secret://1password/<vault-id>/<item-id>/<field>: {err}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn add_and_remove_value_round_trips() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("values.toml"), "").unwrap();

        add(
            tmp.path(),
            "example_username",
            "secret://1password/vault-id/item-id/username",
        )
        .unwrap();
        let raw = fs::read_to_string(tmp.path().join("values.toml")).unwrap();
        assert!(raw.contains("example_username"));

        remove(tmp.path(), "example_username").unwrap();
        let raw = fs::read_to_string(tmp.path().join("values.toml")).unwrap();
        assert!(!raw.contains("example_username"));
    }

    #[test]
    fn otp_reference_form_is_valid() {
        validate_secret_reference("secret://1password/vault-id/item-id/otp?").unwrap();
    }

    #[test]
    fn unsupported_provider_reference_is_rejected() {
        validate_secret_reference("secret://unsupported/item/field").unwrap_err();
    }

    #[test]
    fn concurrent_adds_preserve_all_values() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("values.toml"), "").unwrap();

        let site = tmp.path().to_path_buf();
        let workers = 12;
        let barrier = Arc::new(Barrier::new(workers));
        let mut handles = Vec::new();

        for i in 0..workers {
            let site = site.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let name = format!("value_{i}");
                let uri = format!("secret://1password/vault/item/value_{i}");
                barrier.wait();
                add(&site, &name, &uri).unwrap();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let raw = fs::read_to_string(tmp.path().join("values.toml")).unwrap();
        for i in 0..workers {
            assert!(raw.contains(&format!("value_{i}")));
        }
    }
}
