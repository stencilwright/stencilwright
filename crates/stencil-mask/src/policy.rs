//! Mask policy: load, compile, apply.
//!
//! The runtime policy is built from three inputs:
//!   - `mask.toml` (site-wide patterns + redact_selectors + length cap)
//!   - per-place `[[place.element]] unmasked = true` selectors
//!   - site-wide `[[element]] unmasked = true` selectors
//!
//! `apply` walks HTML via `lol_html` and produces masked HTML.
//! See `specs/01-stencil.md` §4 for the layer order.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use lol_html::html_content::{ContentType, Element as HtmlElement, TextChunk};
use lol_html::{ElementContentHandlers, RewriteStrSettings, Selector, end_tag, rewrite_str};
use regex::Regex;
use stencil_core::{Element, MaskConfig, Place};

use crate::{ValueNameMap, derive_slot};

/// Compiled, site-wide policy. Cheap to create per place via
/// [`MaskPolicy::for_place`].
#[derive(Debug)]
pub struct MaskPolicy {
    pub patterns: Vec<CompiledPattern>,
    pub redact_selectors: Vec<String>,
    pub site_unmask_selectors: Vec<String>,
    pub max_unmasked_chars: usize,
}

#[derive(Debug)]
pub struct CompiledPattern {
    pub name: String,
    pub regex: Regex,
}

impl MaskPolicy {
    /// Build from a `MaskConfig` (mask.toml content) and the site-wide
    /// elements list (elements.toml's `[[element]]` array).
    pub fn build(cfg: &MaskConfig, site_elements: &[Element]) -> Result<Self> {
        let mut patterns = Vec::with_capacity(cfg.mask.patterns.len());
        for p in &cfg.mask.patterns {
            let regex = Regex::new(&p.regex)
                .with_context(|| format!("invalid regex in pattern '{}'", p.name))?;
            patterns.push(CompiledPattern {
                name: p.name.clone(),
                regex,
            });
        }

        let redact_selectors: Vec<String> = cfg
            .mask
            .redact_selectors
            .iter()
            .flat_map(|r| split_selector_list(&r.selector))
            .collect();

        let site_unmask_selectors: Vec<String> = site_elements
            .iter()
            .filter(|e| e.unmasked)
            .flat_map(|e| split_selector_list(&e.selector))
            .collect();

        // mask.toml may set `max_unmasked_chars = 0` accidentally; treat
        // that as unset and fall back to the documented default.
        let max_unmasked_chars = if cfg.max_unmasked_chars == 0 {
            200
        } else {
            cfg.max_unmasked_chars
        };

        Ok(Self {
            patterns,
            redact_selectors,
            site_unmask_selectors,
            max_unmasked_chars,
        })
    }

    /// Layer in a place's per-place unmask selectors and produce an
    /// effective policy for masking captures of that place.
    pub fn for_place<'a>(&'a self, place: &'a Place) -> EffectivePolicy<'a> {
        let mut unmask_selectors = self.site_unmask_selectors.clone();
        for el in &place.elements {
            if el.unmasked {
                unmask_selectors.extend(split_selector_list(&el.selector));
            }
        }
        EffectivePolicy {
            patterns: &self.patterns,
            redact_selectors: &self.redact_selectors,
            unmask_selectors,
            max_unmasked_chars: self.max_unmasked_chars,
        }
    }
}

/// Split a CSS selector list (`"a, b, c"`) into individual selectors.
/// `lol_html` parses one selector at a time, so we have to peel commas.
/// Naive split — does not handle commas inside quoted attribute values,
/// which we don't currently use in our mask.toml.
fn split_selector_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

#[derive(Debug)]
pub struct EffectivePolicy<'a> {
    pub patterns: &'a [CompiledPattern],
    pub redact_selectors: &'a [String],
    pub unmask_selectors: Vec<String>,
    pub max_unmasked_chars: usize,
}

impl<'a> EffectivePolicy<'a> {
    /// Apply the policy to raw HTML. Returns masked HTML.
    ///
    /// Per text node (assembled across `lol_html` chunks), the layer
    /// order is the one documented in `specs/01-stencil.md` §4 with
    /// the punchlist's clarification that the substring numeric
    /// blacklist runs only under unmask:
    ///
    ///   - `redact_depth > 0` → whole text → one slot
    ///   - else if `unmask_depth > 0` → substring blacklist; original
    ///     length over `max_unmasked_chars` collapses to `[TEXT:N]`
    ///   - else → `[TEXT:N]`
    ///
    /// Whitespace-only text nodes pass through unchanged so layout is
    /// preserved.
    ///
    /// **Attributes.** Identity-bearing attributes (`is_sensitive_attribute`)
    /// always collapse to one slot. *Content-bearing* attributes
    /// (`is_content_attribute` — `aria-label`, `title`, `alt`,
    /// `data-stringify-text`, an `<input>`'s `value`, …) carry the same kind of
    /// free text a text node does, so they follow the **same** depth-driven
    /// default-deny ladder (`[ATTR:N]` when masked). Everything else is treated
    /// as structural (`class`/`id`/`data-qa`/`href`/`role`/…) and stays legible
    /// so selectors can be built against it — only known real values and
    /// numeric-blacklist matches inside it are redacted.
    pub fn apply(&self, html: &str, vn: &ValueNameMap) -> Result<MaskedHtml> {
        let html = mask_raw_text_elements(&mask_comments(html));

        struct State {
            redact_depth: usize,
            unmask_depth: usize,
            text_buf: String,
        }
        let state = Rc::new(RefCell::new(State {
            redact_depth: 0,
            unmask_depth: 0,
            text_buf: String::new(),
        }));

        let mut element_content_handlers: Vec<(Cow<'_, Selector>, ElementContentHandlers<'_>)> =
            Vec::new();

        for sel_str in self.redact_selectors.iter() {
            let selector: Selector = sel_str
                .parse()
                .map_err(|e| anyhow!("invalid redact selector '{sel_str}': {e:?}"))?;
            let st = state.clone();
            let handler =
                ElementContentHandlers::default().element(move |el: &mut HtmlElement<'_, '_>| {
                    let st_end = st.clone();
                    // For void elements (e.g., <input>), on_end_tag fails.
                    // Skip silently — there's no inner text to redact anyway.
                    if el
                        .on_end_tag(end_tag!(move |_| {
                            st_end.borrow_mut().redact_depth -= 1;
                            Ok(())
                        }))
                        .is_ok()
                    {
                        st.borrow_mut().redact_depth += 1;
                    }
                    Ok(())
                });
            element_content_handlers.push((Cow::Owned(selector), handler));
        }

        for sel_str in self.unmask_selectors.iter() {
            let selector: Selector = sel_str
                .parse()
                .map_err(|e| anyhow!("invalid unmask selector '{sel_str}': {e:?}"))?;
            let st = state.clone();
            let handler =
                ElementContentHandlers::default().element(move |el: &mut HtmlElement<'_, '_>| {
                    let st_end = st.clone();
                    if el
                        .on_end_tag(end_tag!(move |_| {
                            st_end.borrow_mut().unmask_depth -= 1;
                            Ok(())
                        }))
                        .is_ok()
                    {
                        st.borrow_mut().unmask_depth += 1;
                    }
                    Ok(())
                });
            element_content_handlers.push((Cow::Owned(selector), handler));
        }

        let patterns_attr = self.patterns;
        let vn_attr = vn.clone();
        let max_unmasked_attr = self.max_unmasked_chars;
        let st_attr = state.clone();
        let star_attrs: Selector = "*".parse().expect("'*' is a valid lol_html selector");
        let attr_handler =
            ElementContentHandlers::default().element(move |el: &mut HtmlElement<'_, '_>| {
                let tag_name = el.tag_name();
                // This handler is registered after the redact/unmask handlers,
                // so for an element that is itself a scope root its own depth is
                // already applied here — content attributes on it mask exactly
                // as its text would.
                let (redact_depth, unmask_depth) = {
                    let st = st_attr.borrow();
                    (st.redact_depth, st.unmask_depth)
                };
                let attrs: Vec<(String, String)> = el
                    .attributes()
                    .iter()
                    .map(|attr| (attr.name(), attr.value()))
                    .collect();
                for (name, value) in attrs {
                    let masked = compute_masked_attribute(
                        &tag_name,
                        &name,
                        &value,
                        redact_depth,
                        unmask_depth,
                        max_unmasked_attr,
                        patterns_attr,
                        &vn_attr,
                    );
                    if masked != value {
                        el.set_attribute(&name, &masked)
                            .map_err(|e| anyhow!("invalid rewritten attribute '{name}': {e}"))?;
                    }
                }
                Ok(())
            });
        element_content_handlers.push((Cow::Owned(star_attrs), attr_handler));

        // One text handler matches inside any element. We accumulate
        // chunks per text node and decide on the last chunk.
        let patterns = self.patterns;
        let max_unmasked_chars = self.max_unmasked_chars;
        let st_text = state.clone();
        let star: Selector = "*".parse().expect("'*' is a valid lol_html selector");
        let text_handler = ElementContentHandlers::default().text(move |t: &mut TextChunk<'_>| {
            // Borrow the buffer briefly to append the chunk.
            {
                let mut st = st_text.borrow_mut();
                st.text_buf.push_str(t.as_str());
            }
            if t.last_in_text_node() {
                // Pull out the buffered text and the current depths,
                // then drop the borrow before calling t.replace.
                let (buf, redact_depth, unmask_depth) = {
                    let mut st = st_text.borrow_mut();
                    (
                        std::mem::take(&mut st.text_buf),
                        st.redact_depth,
                        st.unmask_depth,
                    )
                };
                let output = compute_masked_text(
                    &buf,
                    redact_depth,
                    unmask_depth,
                    patterns,
                    max_unmasked_chars,
                    vn,
                );
                t.replace(&output, ContentType::Text);
            } else {
                // Mid-chunk: swallow into the buffer; the last chunk
                // emits the assembled output.
                t.remove();
            }
            Ok(())
        });
        element_content_handlers.push((Cow::Owned(star), text_handler));

        let settings = RewriteStrSettings {
            element_content_handlers,
            ..RewriteStrSettings::default()
        };
        let masked = rewrite_str(&html, settings)
            .context("lol_html rewrite_str failed during mask apply")?;
        Ok(MaskedHtml(masked))
    }
}

fn mask_comments(html: &str) -> String {
    let re = Regex::new(r"(?s)<!--(.*?)-->").expect("comment regex is valid");
    re.replace_all(html, |caps: &regex::Captures<'_>| {
        let len = caps
            .get(1)
            .map(|m| m.as_str().chars().count())
            .unwrap_or_default();
        format!("<!--[TEXT:{len}]-->")
    })
    .into_owned()
}

fn mask_raw_text_elements(html: &str) -> String {
    let mut out = html.to_string();
    for tag in ["script", "style", "template"] {
        let pattern = format!(r"(?is)<{tag}([^>]*)>(.*?)</{tag}>");
        let re = Regex::new(&pattern).expect("raw-text element regex is valid");
        out = re
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                let len = caps
                    .get(2)
                    .map(|m| m.as_str().chars().count())
                    .unwrap_or_default();
                format!("<{tag}{attrs}>[TEXT:{len}]</{tag}>")
            })
            .into_owned();
    }
    out
}

fn compute_masked_attribute(
    tag_name: &str,
    attr_name: &str,
    value: &str,
    redact_depth: usize,
    unmask_depth: usize,
    max_unmasked_chars: usize,
    patterns: &[CompiledPattern],
    vn: &ValueNameMap,
) -> String {
    if value.trim().is_empty() {
        return value.to_string();
    }
    // Identity-bearing attributes: always one slot, regardless of scope.
    if is_sensitive_attribute(tag_name, attr_name) {
        return derive_slot(value.trim(), vn).render();
    }
    // Content-bearing attributes carry free, human-readable text (display
    // names, labels, tooltips, typed input values) — the same kind of content
    // a text node holds and just as PII-bearing. So they get the *same*
    // default-deny treatment as text (mirroring `compute_masked_text`): masked
    // unless the element is in an explicit unmask scope. Without this, content
    // like `aria-label="Jane Doe"` or `data-stringify-text` leaks verbatim.
    if is_content_attribute(tag_name, attr_name) {
        if redact_depth > 0 {
            return derive_slot(value.trim(), vn).render();
        }
        let len = value.chars().count();
        if unmask_depth > 0 {
            if len > max_unmasked_chars {
                return format!("[ATTR:{len}]");
            }
            return apply_substring_blacklist(value, patterns, vn);
        }
        return format!("[ATTR:{len}]");
    }

    // Structural attributes (`class`, `id`, `data-qa`, `role`, `href`, …) stay
    // legible so selectors can be built against them; we only redact known real
    // values and numeric-blacklist matches that happen to appear inside.
    let mut replacements: Vec<(&str, String)> = vn
        .entries()
        .filter(|(real, _)| !real.is_empty() && value.contains(real))
        .map(|(real, _)| (real, derive_slot(real, vn).render()))
        .collect();
    replacements.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    let mut out = value.to_string();
    for (real, slot) in replacements {
        out = out.replace(real, &slot);
    }
    apply_substring_blacklist(&out, patterns, vn)
}

fn is_sensitive_attribute(tag_name: &str, attr_name: &str) -> bool {
    if tag_name.contains("current-user") {
        return true;
    }
    matches!(
        attr_name,
        "username"
            | "display-name"
            | "author"
            | "author-id"
            | "user-id"
            | "account-id"
            | "email"
            | "post-upvote-ratio"
    )
}

/// Attributes whose values carry free, human-readable text rather than page
/// structure. These leak content the way text nodes would, so they are
/// default-denied (see [`compute_masked_attribute`]). Kept deliberately
/// separate from structural attributes (`class`/`id`/`data-qa`/`href`/`role`/
/// `aria-hidden`/…), which must stay legible to build selectors against.
///
/// Discovered live: Acme carries display names in `data-stringify-text` and
/// substantive content in `aria-label` — on a financial site the same classes
/// would carry balances, account numbers, and names. Hardened before any such
/// mapping (see HANDOFF "masker leak classes").
fn is_content_attribute(tag_name: &str, attr_name: &str) -> bool {
    if matches!(
        attr_name,
        "title"
            | "alt"
            | "placeholder"
            | "label"
            | "aria-label"
            | "aria-description"
            | "aria-roledescription"
            | "aria-placeholder"
            | "aria-valuetext"
            | "data-stringify-text"
            | "data-tooltip"
            | "data-original-title"
    ) {
        return true;
    }
    // A form control's `value` is user-entered content (a typed name, email,
    // or account number) — unlike `<option value>`, which is a structural code.
    attr_name == "value" && matches!(tag_name, "input" | "textarea")
}

fn compute_masked_text(
    buf: &str,
    redact_depth: usize,
    unmask_depth: usize,
    patterns: &[CompiledPattern],
    max_unmasked_chars: usize,
    vn: &ValueNameMap,
) -> String {
    // Whitespace-only nodes pass through to keep the masked HTML
    // layout-readable.
    if buf.trim().is_empty() {
        return buf.to_string();
    }
    let orig_len = buf.chars().count();
    if redact_depth > 0 {
        // Whole text → one slot. Trim so the slot identifies the value
        // and not the indentation around it.
        derive_slot(buf.trim(), vn).render()
    } else if unmask_depth > 0 {
        if orig_len > max_unmasked_chars {
            format!("[TEXT:{orig_len}]")
        } else {
            apply_substring_blacklist(buf, patterns, vn)
        }
    } else {
        format!("[TEXT:{orig_len}]")
    }
}

/// Walk every numeric-blacklist regex over the text and replace each
/// match with its slot. Overlapping matches resolve longest-first at the
/// same start; later overlaps are dropped.
fn apply_substring_blacklist(
    text: &str,
    patterns: &[CompiledPattern],
    vn: &ValueNameMap,
) -> String {
    let mut matches: Vec<(usize, usize, String)> = Vec::new();
    for p in patterns {
        for m in p.regex.find_iter(text) {
            let slot = derive_slot(m.as_str(), vn).render();
            matches.push((m.start(), m.end(), slot));
        }
    }
    if matches.is_empty() {
        return text.to_string();
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| (b.1 - b.0).cmp(&(a.1 - a.0))));

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (start, end, slot) in matches {
        if start < cursor {
            continue;
        }
        out.push_str(&text[cursor..start]);
        out.push_str(&slot);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Newtype over masked HTML — its existence is proof the masking
/// pipeline ran.
#[derive(Debug, Clone)]
pub struct MaskedHtml(pub String);

/// Raw, unmasked HTML. Only constructable in the `raw` feature path
/// of `stencil-browser`.
#[derive(Debug, Clone)]
pub struct RawHtml(pub String);
