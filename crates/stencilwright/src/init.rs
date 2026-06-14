//! `stencilwright init <site>` — scaffold `~/.stencilwright/<site>/`.
//!
//! Creates the site config plus the four mapping artifacts
//! (`site.toml`, `places.toml`, `elements.toml`, `mask.toml`,
//! `values.toml`), the `profile/` directory at mode 0700, and an
//! empty `captures/` directory.
//!
//! Refuses to clobber an existing site directory.

use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use stencil_core::paths;

const MASK_TEMPLATE: &str = include_str!("../templates/mask.toml");
const ELEMENTS_TEMPLATE: &str = include_str!("../templates/elements.toml");
const SITE_TEMPLATE: &str = include_str!("../templates/site.toml");
const PLACES_EXAMPLE: &str = include_str!("../templates/places-example.toml");
const PLACES_DEFAULT: &str = include_str!("../templates/places-default.toml");
const VALUES_EXAMPLE: &str = include_str!("../templates/values-example.toml");
const VALUES_DEFAULT: &str = include_str!("../templates/values-default.toml");

/// Scaffold `~/.stencilwright/<site>/`. Returns Err if the site
/// directory already exists.
pub fn run(site: &str) -> Result<()> {
    run_at_root(&paths::root_dir(), site)
}

/// Scaffold `<root>/<site>/`. Used by tests and by the public
/// [`run`] wrapper.
pub(crate) fn run_at_root(root: &Path, site: &str) -> Result<()> {
    let site_dir = root.join(site);
    if site_dir.exists() {
        bail!(
            "{} already exists; refusing to clobber existing config. \
             Remove it manually if you want a fresh scaffold.",
            site_dir.display()
        );
    }

    create_private_dir(root)?;
    create_private_dir(&site_dir)?;

    // profile/ holds Chrome's user-data-dir — cookies and session state.
    // Mode 0700 keeps it out of other local users' reach.
    fs::DirBuilder::new()
        .recursive(false)
        .mode(0o700)
        .create(site_dir.join("profile"))
        .with_context(|| format!("creating {}/profile", site_dir.display()))?;

    fs::create_dir(site_dir.join("captures"))
        .with_context(|| format!("creating {}/captures", site_dir.display()))?;

    let (places, values) = templates_for(site);

    fs::write(site_dir.join("site.toml"), SITE_TEMPLATE)?;
    fs::write(site_dir.join("mask.toml"), MASK_TEMPLATE)?;
    fs::write(site_dir.join("elements.toml"), ELEMENTS_TEMPLATE)?;
    fs::write(site_dir.join("places.toml"), places)?;
    fs::write(site_dir.join("values.toml"), values)?;

    println!("scaffolded {}/", site_dir.display());
    println!("next steps:");
    println!(
        "  1. edit {}/values.toml to point at your 1Password entries",
        site_dir.display()
    );
    println!("     optional: stencilwright {site} config set --onepassword-account <account>");
    if site == "example" {
        println!("  2. stencilwright {site} place listing_main goto");
    } else {
        println!(
            "  2. add at least one [[place]] to {}/places.toml",
            site_dir.display()
        );
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(false)
        .mode(0o700)
        .create(path)
        .with_context(|| format!("creating {}", path.display()))
}

fn templates_for(site: &str) -> (&'static str, &'static str) {
    match site {
        "example" => (PLACES_EXAMPLE, VALUES_EXAMPLE),
        _ => (PLACES_DEFAULT, VALUES_DEFAULT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use stencil_core::{MaskConfig, ValuesConfig};
    use tempfile::tempdir;

    #[test]
    fn init_example_creates_expected_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "example").unwrap();

        let site = root.join("example");
        assert!(site.is_dir(), "site dir not created");
        for child in [
            "site.toml",
            "mask.toml",
            "elements.toml",
            "places.toml",
            "values.toml",
            "profile",
            "captures",
        ] {
            assert!(site.join(child).exists(), "missing scaffold child: {child}",);
        }

        let profile_mode = fs::metadata(site.join("profile"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            profile_mode & 0o777,
            0o700,
            "profile dir should be 0o700, got {:o}",
            profile_mode & 0o777,
        );
    }

    #[test]
    fn init_example_writes_parseable_site_toml() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "example").unwrap();

        let site_str = fs::read_to_string(root.join("example/site.toml")).unwrap();
        let config: stencil_core::SiteConfig =
            toml::from_str(&site_str).expect("site.toml must parse");
        assert_eq!(config.onepassword_account, None);
    }

    #[test]
    fn init_example_writes_parseable_mask_toml() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "example").unwrap();

        let mask_str = fs::read_to_string(root.join("example/mask.toml")).unwrap();
        let mask: MaskConfig = toml::from_str(&mask_str).expect("mask.toml must parse");
        assert!(
            mask.mask.patterns.len() >= 10,
            "expected the standard pattern set, got {}",
            mask.mask.patterns.len(),
        );
        for required in [
            "currency",
            "long_digits",
            "email",
            "ipv4",
            "ipv6",
            "datetime",
            "date",
            "time",
            "phone",
            "uuid",
            "hex_block",
            "percent",
        ] {
            assert!(
                mask.mask.patterns.iter().any(|p| p.name == required),
                "missing pattern '{required}' in default mask.toml",
            );
        }
        assert_eq!(mask.max_unmasked_chars, 200);
    }

    #[test]
    fn init_example_writes_parseable_values_toml() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "example").unwrap();

        let values_str = fs::read_to_string(root.join("example/values.toml")).unwrap();
        let values: ValuesConfig = toml::from_str(&values_str).expect("values.toml must parse");
        assert!(
            values.entries.is_empty(),
            "example values.toml should ship with commented placeholders, not real references",
        );
        assert!(values_str.contains("example_username"));
        assert!(values_str.contains("secret://1password/"));
    }

    #[test]
    fn init_example_places_toml_contains_expected_places() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "example").unwrap();

        let places_str = fs::read_to_string(root.join("example/places.toml")).unwrap();
        // No formal loader yet (PlaceGraph::from_dir lands in cp5);
        // verify TOML is syntactically valid + the expected place names
        // appear.
        let _: toml::Value = toml::from_str(&places_str).expect("places.toml must parse");
        for name in ["login_password", "login_otp", "home", "listing_main"] {
            assert!(
                places_str.contains(&format!("name = \"{name}\"")),
                "missing place '{name}'",
            );
        }
        assert!(places_str.contains("auto_fill = \"{example_username}\""));
        assert!(places_str.contains("auto_fill = \"{example_password}\""));
        assert!(places_str.contains("auto_fill = \"{example_totp}\""));
        assert!(places_str.contains("submit.click"));
    }

    #[test]
    fn init_refuses_to_clobber_existing_site() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "example").unwrap();

        let err = run_at_root(&root, "example").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("already exists"),
            "expected clobber-refusal error, got: {msg}",
        );
    }

    #[test]
    fn init_unknown_site_uses_default_templates() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stencils");
        run_at_root(&root, "synthsite").unwrap();

        let site = root.join("synthsite");
        assert!(site.join("mask.toml").exists());

        // Default places.toml is parseable TOML even with no places
        // declared — the user adds their own.
        let places_str = fs::read_to_string(site.join("places.toml")).unwrap();
        let _: toml::Value = toml::from_str(&places_str).expect("default places.toml must parse");
        assert!(
            !places_str.contains("secret://1password/<vault-id>"),
            "default places template should use values.toml references, not provider refs",
        );

        // Default values.toml parses and is empty (no resolved entries).
        let values_str = fs::read_to_string(site.join("values.toml")).unwrap();
        let values: ValuesConfig =
            toml::from_str(&values_str).expect("default values.toml must parse");
        assert!(
            values.entries.is_empty(),
            "default values.toml should ship empty"
        );
    }
}
