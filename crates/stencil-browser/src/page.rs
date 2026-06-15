//! Page client: thin RPC façade over the daemon's Playwright Page.
//!
//! Each method serializes a JSON command, sends it to the daemon,
//! and deserializes the response. The trust boundary lives here:
//!
//! - [`Page::dump_masked`] is the stencilwright path: the daemon
//!   resolves values and returns already-masked HTML.
//! - [`Page::dump`] is the legacy library path: it runs the masking
//!   pipeline before yielding anything.
//! - [`Page::dump_raw`] returns unmasked HTML and is gated behind
//!   `#[cfg(feature = "raw")]`. `stencilwright` does not enable that
//!   feature, so the symbol is not in its build.
//! - [`Page::aria_snapshot_raw`] is the same gate around the
//!   accessibility tree YAML — much more concise than DOM HTML, but
//!   masking it requires a YAML-aware redactor that lives in
//!   `stencil-mask` (deferred to a follow-up). Until then, only
//!   raw-feature consumers can pull it.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};

#[cfg(feature = "raw")]
use crate::RawSnippet;
use stencil_core::{
    Element, MaskConfig, Place, UnmaskApprovalContext, UnmaskApprovalDecision, ValuesConfig,
};
#[cfg(feature = "raw")]
use stencil_mask::RawHtml;
use stencil_mask::{EffectivePolicy, MaskedHtml, ValueNameMap};

use crate::session::SessionInner;

/// Handle to the daemon's single Page. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct Page {
    session: Arc<SessionInner>,
}

impl Page {
    pub(crate) fn new(session: Arc<SessionInner>) -> Self {
        Self { session }
    }

    pub async fn goto(&self, url: &str) -> Result<()> {
        self.rpc("goto", json!({ "url": url })).await?;
        Ok(())
    }

    pub async fn goto_template(&self, url: &str, values: &ValuesConfig) -> Result<()> {
        self.rpc("goto_template", json!({ "url": url, "values": values }))
            .await?;
        Ok(())
    }

    pub async fn click(&self, selector: &str) -> Result<()> {
        self.rpc("click", json!({ "selector": selector })).await?;
        Ok(())
    }

    /// Click bypassing Playwright actionability checks (visible / stable /
    /// receives-events). For controls that are present but fail the default
    /// checks — e.g. Acme's IA4 top-nav search button.
    pub async fn click_force(&self, selector: &str) -> Result<()> {
        self.rpc("click", json!({ "selector": selector, "force": true }))
            .await?;
        Ok(())
    }

    /// Press a single key on `selector` (e.g. `"Enter"` to submit a search).
    pub async fn press(&self, selector: &str, key: &str) -> Result<()> {
        self.rpc("press", json!({ "selector": selector, "key": key }))
            .await?;
        Ok(())
    }

    /// Type `text` into `selector` with real per-character key events. For
    /// rich editors that ignore `fill` (Acme's Quill-based inputs).
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        self.rpc("type", json!({ "selector": selector, "text": text }))
            .await?;
        Ok(())
    }

    /// Press a key on the page's currently focused element (e.g. `"Enter"` to
    /// submit a focused but hard-to-select search box). Unlike [`Self::press`],
    /// this does not take a selector, so it won't move focus.
    pub async fn key(&self, key: &str) -> Result<()> {
        self.rpc("key", json!({ "key": key })).await?;
        Ok(())
    }

    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        self.rpc("fill", json!({ "selector": selector, "value": value }))
            .await?;
        Ok(())
    }

    pub async fn fill_ref(
        &self,
        selector: &str,
        value_ref: &str,
        values: &ValuesConfig,
    ) -> Result<()> {
        self.rpc(
            "fill_ref",
            json!({ "selector": selector, "value_ref": value_ref, "values": values }),
        )
        .await?;
        Ok(())
    }

    pub async fn select_option(&self, selector: &str, value: &str) -> Result<()> {
        self.rpc(
            "select_option",
            json!({ "selector": selector, "value": value }),
        )
        .await?;
        Ok(())
    }

    pub async fn wait_for(&self, selector: &str, timeout: Duration) -> Result<()> {
        self.rpc(
            "wait_for",
            json!({
                "selector": selector,
                "timeout_ms": timeout.as_millis() as u64,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn url(&self) -> Result<String> {
        let v = self.rpc("url", json!({})).await?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("daemon `url` op returned non-string: {v}"))
    }

    pub async fn url_template(&self, values: &ValuesConfig) -> Result<String> {
        let v = self
            .rpc("url_template", json!({ "values": values }))
            .await?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("daemon `url_template` op returned non-string: {v}"))
    }

    pub async fn locator_count(&self, selector: &str) -> Result<usize> {
        let v = self
            .rpc("locator_count", json!({ "selector": selector }))
            .await?;
        v.as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| anyhow!("daemon `locator_count` op returned non-integer: {v}"))
    }

    pub async fn locator_visible_count(&self, selector: &str) -> Result<usize> {
        let v = self
            .rpc("locator_visible_count", json!({ "selector": selector }))
            .await?;
        v.as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| anyhow!("daemon `locator_visible_count` op returned non-integer: {v}"))
    }

    /// Dump masked DOM. The raw HTML returned by the `content` RPC
    /// is run through the masking pipeline before this function
    /// returns; nothing unmasked escapes.
    pub async fn dump(
        &self,
        policy: &EffectivePolicy<'_>,
        vn: &ValueNameMap,
    ) -> Result<MaskedHtml> {
        let raw = self.rpc_content().await?;
        policy.apply(&raw, vn).context("masking captured DOM")
    }

    /// Dump masked DOM via the daemon. The client sends only
    /// non-secret masking config and value references; the daemon
    /// resolves values in memory and returns already-masked HTML.
    pub async fn dump_masked(
        &self,
        mask_config: &MaskConfig,
        site_elements: &[Element],
        place: Option<&Place>,
        values: &ValuesConfig,
    ) -> Result<MaskedHtml> {
        let args = MaskedDumpArgs {
            mask_config,
            site_elements,
            place,
            values,
        };
        let v = self.rpc("dump_masked", serde_json::to_value(args)?).await?;
        let html = v
            .as_str()
            .ok_or_else(|| anyhow!("daemon `dump_masked` op returned non-string: {v}"))?;
        Ok(MaskedHtml(html.to_string()))
    }

    /// **Library-only**, behind the `raw` feature: returns unmasked
    /// DOM. Construct [`RawAccess`] via [`RawAccess::acknowledged`]
    /// to make every call site visible to grep.
    #[cfg(feature = "raw")]
    pub async fn dump_raw(&self, _: RawAccess) -> Result<RawHtml> {
        let raw = self.rpc_content().await?;
        Ok(RawHtml(raw))
    }

    /// **Library-only**, behind the `raw` feature: returns text
    /// contents for every element matching `selector`.
    #[cfg(feature = "raw")]
    pub async fn selector_text_raw(&self, _: RawAccess, selector: &str) -> Result<Vec<RawSnippet>> {
        let v = self
            .rpc("selector_text_raw", json!({ "selector": selector }))
            .await?;
        serde_json::from_value(v).context("parse selector_text_raw response")
    }

    /// **Library-only**, behind the `raw` feature: the value of `attr` on every
    /// element matching `selector`, in document order (missing → empty string).
    /// Used for data carried in attributes rather than text — e.g. a result
    /// row's permalink `href`.
    #[cfg(feature = "raw")]
    pub async fn selector_attr_raw(
        &self,
        _: RawAccess,
        selector: &str,
        attr: &str,
    ) -> Result<Vec<String>> {
        let v = self
            .rpc(
                "selector_attr_raw",
                json!({ "selector": selector, "attr": attr }),
            )
            .await?;
        serde_json::from_value(v).context("parse selector_attr_raw response")
    }

    /// Fetch raw snippets and show the in-process approval dialog.
    ///
    /// Only the resulting decision crosses the crate boundary.
    #[cfg(feature = "approval-dialog")]
    pub async fn approve_unmask(
        &self,
        scope: &str,
        selector: &str,
        proposed_name: Option<&str>,
    ) -> Result<UnmaskApprovalDecision> {
        let context = UnmaskApprovalContext {
            scope: Some(scope.to_string()),
            ..UnmaskApprovalContext::default()
        };
        self.approve_unmask_with_context(&context, selector, proposed_name)
            .await
    }

    /// Fetch raw snippets and show the in-process approval dialog with
    /// non-secret page context.
    ///
    /// Only the resulting decision crosses the crate boundary.
    #[cfg(feature = "approval-dialog")]
    pub async fn approve_unmask_with_context(
        &self,
        context: &UnmaskApprovalContext,
        selector: &str,
        proposed_name: Option<&str>,
    ) -> Result<UnmaskApprovalDecision> {
        let v = self
            .rpc(
                "request_unmask_approval",
                json!({
                    "context": context,
                    "selector": selector,
                    "proposed_name": proposed_name,
                }),
            )
            .await?;
        let snippets = serde_json::from_value(v).context("parse unmask approval response")?;
        crate::approval::approve(snippets)
    }

    /// **Library-only**, behind the `raw` feature: ARIA-snapshot YAML
    /// for `<body>`. Concise structural view of the page (roles,
    /// names, levels) — see the agent research note in PUNCHLIST
    /// CP4.5. The YAML carries every accessible name and value, so
    /// it's PII-bearing and not safe for `stencilwright` until a
    /// YAML-aware redactor lands in `stencil-mask`.
    #[cfg(feature = "raw")]
    pub async fn aria_snapshot_raw(&self, _: RawAccess) -> Result<String> {
        let v = self.rpc("aria_snapshot", json!({})).await?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("daemon `aria_snapshot` op returned non-string: {v}"))
    }

    /// Internal — both `dump` and `dump_raw` use this. NOT pub:
    /// unmasked DOM only escapes via the gated `dump_raw`.
    async fn rpc_content(&self) -> Result<String> {
        let v = self.rpc("content", json!({})).await?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("daemon `content` op returned non-string: {v}"))
    }

    async fn rpc(&self, op: &str, args: Value) -> Result<Value> {
        self.session.rpc(op, args).await
    }
}

#[derive(Serialize)]
struct MaskedDumpArgs<'a> {
    mask_config: &'a MaskConfig,
    site_elements: &'a [Element],
    place: Option<&'a Place>,
    values: &'a ValuesConfig,
}

/// Marker proving the caller intentionally opted into unmasked output.
/// Construct via [`RawAccess::acknowledged`].
#[cfg(feature = "raw")]
pub struct RawAccess(());

#[cfg(feature = "raw")]
impl RawAccess {
    pub fn acknowledged() -> Self {
        Self(())
    }
}
