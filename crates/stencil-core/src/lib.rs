//! Pure types shared by the rest of the stencilwright/apiwright
//! workspace. No I/O, no async.
//!
//! See `specs/01-stencil.md` §6 (artifact formats) and §7 (API).

use serde::{Deserialize, Serialize};

pub mod paths;

/// Non-secret site-local settings loaded from `site.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SiteConfig {
    /// 1Password account shorthand/domain passed to `op --account`.
    pub onepassword_account: Option<String>,
}

/// Non-secret context shown in the unmask approval dialog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UnmaskApprovalContext {
    /// Human-readable scope label, e.g. `place listing_main`.
    pub scope: Option<String>,
    /// Site id under `~/.stencilwright/<site>/`.
    pub site: Option<String>,
    /// Recognized place id, when the approval is place-scoped.
    pub place: Option<String>,
    /// Current browser URL at approval time.
    pub current_url: Option<String>,
    /// User/agent supplied reason for requesting unmask approval.
    pub reason: Option<String>,
    /// Matching criteria for the recognized place.
    pub signature: Option<Signature>,
}

/// User decision returned by the unmask approval dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnmaskApprovalDecision {
    pub approved: bool,
    /// Optional user-written guidance for the CLI/agent. Must not
    /// contain exact private values.
    pub feedback: Option<String>,
}

impl UnmaskApprovalDecision {
    pub fn new(approved: bool, feedback: impl Into<String>) -> Self {
        let feedback = clean_feedback(feedback.into());
        Self { approved, feedback }
    }
}

fn clean_feedback(feedback: String) -> Option<String> {
    let feedback = feedback.split_whitespace().collect::<Vec<_>>().join(" ");
    if feedback.is_empty() {
        None
    } else {
        Some(feedback)
    }
}

/// A named, recognizable destination on a site.
///
/// Loaded from `places.toml` under `[[place]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    /// Direct-navigation URL. Mutually exclusive with `from`.
    pub url: Option<String>,
    /// Parent place name when this place is reached by transition.
    pub from: Option<String>,
    /// Transition action from `from` to here (e.g. a click).
    pub via: Option<Transition>,
    /// Whether the human is expected to complete a flow here
    /// (login, push 2FA). Runner halts and waits.
    #[serde(default)]
    pub interactive: bool,
    /// Optional click to run immediately after all auto-fill fields
    /// for this place have been filled successfully.
    pub submit: Option<Submit>,
    pub signature: Signature,
    /// How we know an interactive flow has completed.
    pub completion: Option<Signature>,
    /// If set, the runner navigates to this URL whenever this place
    /// is recognized, then re-recognizes. Used for "you shouldn't be
    /// here, go elsewhere" markers like a permission-denied page that
    /// should redirect into a login flow. Mutually exclusive with
    /// `interactive` semantics in practice (we don't auto-fill on a
    /// place we're going to leave immediately).
    #[serde(default)]
    pub redirect: Option<String>,
    /// Per-place named elements.
    #[serde(default, rename = "element")]
    pub elements: Vec<Element>,
}

/// How to transit from a parent place to this one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub click: Option<String>,
    pub fill: Option<FillTransition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submit {
    pub click: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillTransition {
    pub selector: String,
    pub value: String,
}

/// Recognition criteria for a page state. AND-combined: every set
/// component must match.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub url: Option<String>,
    pub selector: Option<String>,
    pub visible_selector: Option<String>,
    pub absent_selector: Option<String>,
    pub text: Option<String>,
}

/// A named DOM element with optional auto-fill source and unmask flag.
///
/// Used both per-place (under `[[place.element]]`) and site-wide
/// (under `elements.toml`'s `[[element]]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub name: String,
    pub selector: String,
    /// Secret-provider reference or values.toml name reference for auto-fill.
    pub auto_fill: Option<String>,
    /// If true, the masking layer leaves this element's text unmasked
    /// in dumps (subject to numeric blacklist on substrings + length cap).
    #[serde(default, alias = "reveal_text")]
    pub unmasked: bool,
}

/// Slot identity for a redacted value. Output format: `[$<id> <description>]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slot {
    pub id: SlotId,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SlotId {
    /// User-given name from values.toml. Snake_case.
    Named(String),
    /// `sha256(value)[:8]` for unnamed values. 8 hex chars.
    Hash(String),
}

impl Slot {
    /// Render to the `[$<id> <description>]` form used in masked HTML.
    pub fn render(&self) -> String {
        let id = match &self.id {
            SlotId::Named(n) => n.clone(),
            SlotId::Hash(h) => h.clone(),
        };
        if self.description.is_empty() {
            format!("[${id}]")
        } else {
            format!("[${id} {}]", self.description)
        }
    }
}

/// Site-wide mask policy loaded from `mask.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MaskConfig {
    #[serde(default)]
    pub mask: MaskInner,
    #[serde(default = "default_max_unmasked_chars", alias = "max_revealed_chars")]
    pub max_unmasked_chars: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MaskInner {
    #[serde(default, rename = "pattern")]
    pub patterns: Vec<PatternRule>,
    #[serde(default, rename = "redact_selectors")]
    pub redact_selectors: Vec<RedactSelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRule {
    pub name: String,
    pub regex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactSelector {
    pub selector: String,
}

fn default_max_unmasked_chars() -> usize {
    200
}

/// `values.toml` content. Name → secret-provider reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValuesConfig {
    pub entries: std::collections::BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::UnmaskApprovalDecision;

    #[test]
    fn approval_decision_drops_blank_feedback() {
        let decision = UnmaskApprovalDecision::new(false, " \n\t ");
        assert_eq!(decision.feedback, None);
    }

    #[test]
    fn approval_decision_normalizes_feedback_whitespace() {
        let decision = UnmaskApprovalDecision::new(true, " private field\n\nmap it masked ");
        assert_eq!(
            decision.feedback.as_deref(),
            Some("private field map it masked")
        );
    }
}
