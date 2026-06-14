//! Page recognition: figure out which known place the current
//! browser state matches.
//!
//! Signature semantics (AND-combined):
//!   - `url`             — structured absolute-URL matcher
//!   - `selector`        — at least one CSS selector in the comma-list
//!                          must have a `locator_count > 0`
//!   - `visible_selector` — at least one CSS selector in the comma-list
//!                          must have a visible locator
//!   - `absent_selector` — none of the comma-list may have a match
//!   - `text`            — not implemented in v1
//!
//! `recognize` returns the FIRST place whose signature satisfies all
//! its components. Order in `places.toml` is the priority — put more
//! specific places before more generic ones.

use anyhow::{Context, Result};
use stencil_browser::Page;
use stencil_core::Signature;

use crate::PlaceGraph;
use crate::url_match;

#[derive(Debug, Clone)]
pub struct PlaceMatch {
    pub place_name: String,
}

pub async fn recognize(graph: &PlaceGraph, page: &Page) -> Result<Option<PlaceMatch>> {
    let url = page.url().await?;
    for place in &graph.places {
        if signature_matches(&place.signature, &url, page).await? {
            return Ok(Some(PlaceMatch {
                place_name: place.name.clone(),
            }));
        }
    }
    Ok(None)
}

pub async fn signature_matches(sig: &Signature, url: &str, page: &Page) -> Result<bool> {
    if let Some(signature_url) = &sig.url {
        if !url_match::matches_signature_url(signature_url, url)
            .with_context(|| format!("matching signature url: {signature_url}"))?
        {
            return Ok(false);
        }
    }
    if let Some(selector) = &sig.selector {
        if !any_selector_matches(page, selector).await? {
            return Ok(false);
        }
    }
    if let Some(selector) = &sig.visible_selector {
        if !any_visible_selector_matches(page, selector).await? {
            return Ok(false);
        }
    }
    if let Some(absent) = &sig.absent_selector {
        if any_selector_matches(page, absent).await? {
            return Ok(false);
        }
    }
    // sig.text: not implemented in v1.
    Ok(true)
}

/// CSS selector lists are comma-separated with OR semantics. lol_html
/// (in masking) and the recognition runner both have to peel commas.
async fn any_selector_matches(page: &Page, selector_list: &str) -> Result<bool> {
    for sel in selector_list
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if page.locator_count(sel).await? > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn any_visible_selector_matches(page: &Page, selector_list: &str) -> Result<bool> {
    for sel in selector_list
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if page.locator_visible_count(sel).await? > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}
