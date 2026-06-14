//! `config` resource commands for non-secret site settings.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, value};

use crate::cli::ConfigSetArgs;
use crate::config_lock::{ConfigLock, write_atomic};

const SITE_TEMPLATE: &str = include_str!("../templates/site.toml");

pub(crate) fn show(site_dir: &Path) -> Result<()> {
    ensure_site_dir(site_dir)?;
    let config = stencil_browser::site_config::load(site_dir)?;
    println!("key\tvalue");
    match config.onepassword_account.as_deref() {
        Some(account) => println!("onepassword_account\t{account}"),
        None => println!("onepassword_account\t(unset)"),
    }
    Ok(())
}

pub(crate) fn set(site_dir: &Path, args: ConfigSetArgs) -> Result<()> {
    ensure_site_dir(site_dir)?;
    if args.onepassword_account.is_none() && !args.clear_onepassword_account {
        bail!("nothing to set; pass --onepassword-account or --clear-onepassword-account");
    }
    let path = site_dir.join("site.toml");
    let _lock = ConfigLock::lock(site_dir)?;
    let mut doc = read_doc_or_template(&path)?;

    if args.clear_onepassword_account {
        doc.remove("onepassword_account");
    }
    if let Some(account) = args.onepassword_account {
        let account = validate_onepassword_account(&account)?;
        doc["onepassword_account"] = value(account);
    }

    write_atomic(&path, &doc.to_string())?;
    println!("updated {}", path.display());
    Ok(())
}

fn ensure_site_dir(site_dir: &Path) -> Result<()> {
    if !site_dir.is_dir() {
        bail!(
            "{} not found; run `stencilwright init <site>` first",
            site_dir.display()
        );
    }
    Ok(())
}

fn read_doc_or_template(path: &Path) -> Result<DocumentMut> {
    let raw = if path.exists() {
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        SITE_TEMPLATE.to_string()
    };
    raw.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))
}

fn validate_onepassword_account(account: &str) -> Result<String> {
    let trimmed = account.trim();
    if trimmed.is_empty() {
        bail!("onepassword account cannot be empty");
    }
    if trimmed.chars().any(char::is_whitespace) {
        bail!("onepassword account cannot contain whitespace");
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use stencil_core::SiteConfig;
    use tempfile::tempdir;

    #[test]
    fn set_onepassword_account_round_trips_through_site_config() {
        let tmp = tempdir().unwrap();

        set(
            tmp.path(),
            ConfigSetArgs {
                onepassword_account: Some("  my.1password.com  ".to_string()),
                clear_onepassword_account: false,
            },
        )
        .unwrap();

        let raw = fs::read_to_string(tmp.path().join("site.toml")).unwrap();
        let config: SiteConfig = toml::from_str(&raw).unwrap();
        assert_eq!(
            config.onepassword_account.as_deref(),
            Some("my.1password.com")
        );
    }

    #[test]
    fn clear_onepassword_account_removes_key() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("site.toml"),
            "onepassword_account = \"my.1password.com\"\n",
        )
        .unwrap();

        set(
            tmp.path(),
            ConfigSetArgs {
                onepassword_account: None,
                clear_onepassword_account: true,
            },
        )
        .unwrap();

        let raw = fs::read_to_string(tmp.path().join("site.toml")).unwrap();
        let config: SiteConfig = toml::from_str(&raw).unwrap();
        assert_eq!(config.onepassword_account, None);
    }
}
