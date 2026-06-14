//! Load non-secret site settings from `site.toml`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use stencil_core::SiteConfig;
use stencil_secrets::OpConfig;

pub fn load(site_dir: &Path) -> Result<SiteConfig> {
    let path = site_dir.join("site.toml");
    if !path.exists() {
        return Ok(SiteConfig::default());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

pub fn load_op_config(site_dir: &Path) -> Result<OpConfig> {
    Ok(OpConfig::from_site_config(&load(site_dir)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_site_toml_is_default_config() {
        let tmp = tempdir().unwrap();
        let config = load(tmp.path()).unwrap();
        assert_eq!(config, SiteConfig::default());
    }

    #[test]
    fn loads_onepassword_account() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("site.toml"),
            "onepassword_account = \"my.1password.com\"\n",
        )
        .unwrap();

        let config = load(tmp.path()).unwrap();
        assert_eq!(
            config.onepassword_account.as_deref(),
            Some("my.1password.com")
        );
    }
}
