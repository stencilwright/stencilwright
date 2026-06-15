//! Load `places.toml`, `elements.toml`, `mask.toml`, `values.toml`
//! into a single in-memory representation.
//!
//! Validation done at load time:
//!   - place names are unique
//!   - element names are unique within a place
//!   - per-place element names don't shadow site-wide elements

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use stencil_browser::Page;
use stencil_core::{Element, MaskConfig, Place, ValuesConfig};
use stencil_mask::MaskedHtml;

#[derive(Debug)]
pub struct PlaceGraph {
    pub places: Vec<Place>,
    pub site_elements: Vec<Element>,
    pub mask_config: MaskConfig,
    pub values: ValuesConfig,
}

/// Top-level shape of `places.toml`. `target` and `description` are
/// informational; we ignore them but accept them so the file parses.
#[derive(Deserialize)]
struct PlacesDoc {
    #[serde(default)]
    #[allow(dead_code)]
    target: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    description: Option<String>,
    #[serde(default, rename = "place")]
    places: Vec<Place>,
}

#[derive(Deserialize)]
struct ElementsDoc {
    #[serde(default, rename = "element")]
    elements: Vec<Element>,
}

impl PlaceGraph {
    pub fn from_dir(stencils_site: &Path) -> Result<Self> {
        let places = load_toml::<PlacesDoc>(&stencils_site.join("places.toml"))?
            .map(|d| d.places)
            .unwrap_or_default();
        let site_elements = load_toml::<ElementsDoc>(&stencils_site.join("elements.toml"))?
            .map(|d| d.elements)
            .unwrap_or_default();
        let mask_config =
            load_toml::<MaskConfig>(&stencils_site.join("mask.toml"))?.unwrap_or_default();
        let values =
            load_toml::<ValuesConfig>(&stencils_site.join("values.toml"))?.unwrap_or_default();

        validate(&places, &site_elements)?;

        Ok(Self {
            places,
            site_elements,
            mask_config,
            values,
        })
    }

    /// Build a graph from in-memory TOML instead of a site directory — for a
    /// standalone adapter that ships its map via `include_str!`. `None` for any
    /// document means "use defaults", matching a missing file in [`Self::from_dir`].
    pub fn from_toml_strs(
        places: Option<&str>,
        elements: Option<&str>,
        mask: Option<&str>,
        values: Option<&str>,
    ) -> Result<Self> {
        let places = parse_toml::<PlacesDoc>(places, "places.toml")?
            .map(|d| d.places)
            .unwrap_or_default();
        let site_elements = parse_toml::<ElementsDoc>(elements, "elements.toml")?
            .map(|d| d.elements)
            .unwrap_or_default();
        let mask_config = parse_toml::<MaskConfig>(mask, "mask.toml")?.unwrap_or_default();
        let values = parse_toml::<ValuesConfig>(values, "values.toml")?.unwrap_or_default();

        validate(&places, &site_elements)?;

        Ok(Self {
            places,
            site_elements,
            mask_config,
            values,
        })
    }

    pub fn place(&self, name: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.name == name)
    }

    /// Site-wide elements followed by the place's own. Conflicts on
    /// name across the two would have been caught in `from_dir`.
    pub fn elements_at(&self, place_name: &str) -> Vec<&Element> {
        let mut out: Vec<&Element> = self.site_elements.iter().collect();
        if let Some(p) = self.place(place_name) {
            out.extend(p.elements.iter());
        }
        out
    }

    /// Recognize first, navigate only on miss, then dump masked DOM
    /// for the target place.
    pub async fn place_goto(&self, page: &Page, target: &str) -> Result<MaskedHtml> {
        crate::runner::place_goto(self, page, target).await
    }
}

fn load_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_toml(Some(&raw), &path.display().to_string())
}

fn parse_toml<T: for<'de> Deserialize<'de>>(raw: Option<&str>, what: &str) -> Result<Option<T>> {
    match raw {
        None => Ok(None),
        Some(s) => Ok(Some(
            toml::from_str(s).with_context(|| format!("parsing {what}"))?,
        )),
    }
}

fn validate(places: &[Place], site_elements: &[Element]) -> Result<()> {
    let mut seen_places: HashSet<&str> = HashSet::new();
    for p in places {
        if !seen_places.insert(p.name.as_str()) {
            bail!("duplicate place name: '{}'", p.name);
        }
    }

    let mut seen_site_elements: HashSet<&str> = HashSet::new();
    for e in site_elements {
        if !seen_site_elements.insert(e.name.as_str()) {
            bail!("duplicate site-wide element name: '{}'", e.name);
        }
    }

    for p in places {
        let mut seen_local: HashSet<&str> = HashSet::new();
        for e in &p.elements {
            if !seen_local.insert(e.name.as_str()) {
                bail!("duplicate element name '{}' in place '{}'", e.name, p.name,);
            }
            if seen_site_elements.contains(e.name.as_str()) {
                bail!(
                    "element '{}' in place '{}' shadows a site-wide element of the same name",
                    e.name,
                    p.name,
                );
            }
        }
    }
    Ok(())
}
