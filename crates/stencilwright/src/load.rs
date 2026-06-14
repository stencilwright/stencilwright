//! Import an existing site mapping into `~/.stencilwright/<site>/`.

use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use stencil_core::paths;

const TOML_FILES: &[&str] = &["places.toml", "elements.toml", "mask.toml", "values.toml"];
const SITE_TEMPLATE: &str = include_str!("../templates/site.toml");

pub fn run(site: &str, source: &Path, force: bool) -> Result<()> {
    run_at_root(&paths::root_dir(), site, source, force)
}

pub(crate) fn run_at_root(root: &Path, site: &str, source: &Path, force: bool) -> Result<()> {
    if !source.is_dir() {
        bail!("source site dir not found: {}", source.display());
    }

    let target = root.join(site);
    if target.exists() {
        if force {
            fs::remove_dir_all(&target)
                .with_context(|| format!("removing {}", target.display()))?;
        } else {
            bail!(
                "{} already exists; pass --force to replace it",
                target.display()
            );
        }
    }

    create_private_dir(root)?;
    create_private_dir(&target)?;

    for file in TOML_FILES {
        let from = source.join(file);
        let to = target.join(file);
        if from.exists() {
            fs::copy(&from, &to)
                .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        } else {
            bail!("source is missing required file {}", from.display());
        }
    }
    copy_site_toml(&source.join("site.toml"), &target.join("site.toml"))?;

    copy_dir_or_create_private(&source.join("profile"), &target.join("profile"), 0o700)?;
    copy_dir_or_create_private(&source.join("captures"), &target.join("captures"), 0o755)?;

    println!("loaded {}/ from {}", target.display(), source.display());
    Ok(())
}

fn copy_site_toml(source: &Path, target: &Path) -> Result<()> {
    if source.exists() {
        fs::copy(source, target)
            .with_context(|| format!("copying {} to {}", source.display(), target.display()))?;
    } else {
        fs::write(target, SITE_TEMPLATE)
            .with_context(|| format!("writing {}", target.display()))?;
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

fn copy_dir_or_create_private(source: &Path, target: &Path, mode: u32) -> Result<()> {
    if !source.exists() {
        fs::DirBuilder::new()
            .recursive(false)
            .mode(mode)
            .create(target)
            .with_context(|| format!("creating {}", target.display()))?;
        return Ok(());
    }
    copy_dir(source, target, mode)
}

fn copy_dir(source: &Path, target: &Path, mode: u32) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(false)
        .mode(mode)
        .create(target)
        .with_context(|| format!("creating {}", target.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&from, &to, mode)?;
        } else if ty.is_file() {
            fs::copy(&from, &to)
                .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_copies_existing_site() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("stencils/example");
        fs::create_dir_all(&source).unwrap();
        for file in TOML_FILES {
            fs::write(source.join(file), b"").unwrap();
        }

        let root = tmp.path().join("home");
        run_at_root(&root, "example", &source, false).unwrap();

        for file in TOML_FILES {
            assert!(root.join("example").join(file).exists(), "missing {file}");
        }
        assert!(root.join("example/site.toml").exists(), "missing site.toml");
        assert!(root.join("example/profile").is_dir());
        assert!(root.join("example/captures").is_dir());
    }

    #[test]
    fn load_copies_site_toml_when_present() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("stencils/example");
        fs::create_dir_all(&source).unwrap();
        for file in TOML_FILES {
            fs::write(source.join(file), b"").unwrap();
        }
        fs::write(
            source.join("site.toml"),
            "onepassword_account = \"my.1password.com\"\n",
        )
        .unwrap();

        let root = tmp.path().join("home");
        run_at_root(&root, "example", &source, false).unwrap();

        let raw = fs::read_to_string(root.join("example/site.toml")).unwrap();
        assert!(raw.contains("my.1password.com"));
    }

    #[test]
    fn load_refuses_existing_without_force() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        for file in TOML_FILES {
            fs::write(source.join(file), b"").unwrap();
        }

        let root = tmp.path().join("home");
        run_at_root(&root, "example", &source, false).unwrap();
        let err = run_at_root(&root, "example", &source, false).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }
}
