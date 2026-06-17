//! Daemon-side RPC dispatch: parse a [`Request`], run the matching
//! Playwright operation, return a [`Response`].
//!
//! Op coverage matches the punchlist CP4 set plus `aria_snapshot`
//! (the agent-research finding — concise YAML view of the page).
//! Adding an op = one match arm + one typed args struct.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use playwright_rs::protocol::ClickOptions;
use playwright_rs::protocol::page::Page as PwPage;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::approval::{RawSnippet, UnmaskedSnippets};
use crate::rpc::{Request, Response};
use crate::secrets::SecretResolver;
use stencil_core::{Element, MaskConfig, Place, Signature, UnmaskApprovalContext, ValuesConfig};
use stencil_mask::MaskPolicy;
use stencil_secrets::OpConfig;

use super::window;

#[derive(Clone)]
pub(super) struct DaemonState {
    page: PwPage,
    secrets: Arc<Mutex<SecretResolver>>,
}

impl DaemonState {
    pub(super) fn new(page: PwPage, op_config: OpConfig) -> Self {
        Self {
            page,
            secrets: Arc::new(Mutex::new(SecretResolver::new(op_config))),
        }
    }
}

/// Top-level entry point. Always returns a `Response` (errors are
/// converted to `Response::err`); the caller's job is to write it
/// back on the socket.
pub async fn handle_request(state: &DaemonState, req: Request) -> Response {
    let id = req.id;
    match dispatch_op(state, &req.op, req.args).await {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, format!("{e:#}")),
    }
}

async fn dispatch_op(state: &DaemonState, op: &str, args: Value) -> Result<Value> {
    let page = &state.page;
    match op {
        "goto" => {
            let GotoArgs { url } = parse_args(args)?;
            page.goto(&url, None).await.context("page.goto")?;
            Ok(Value::Null)
        }
        "goto_template" => {
            let GotoTemplateArgs { url, values } = parse_args(args)?;
            let resolved = {
                let mut secrets = state.secrets.lock().await;
                secrets.interpolate(&url, &values).await?
            };
            page.goto(&resolved, None).await.context("page.goto")?;
            Ok(Value::Null)
        }
        "click" => {
            let ClickArgs { selector, force } = parse_args(args)?;
            let opts = force.then(|| ClickOptions::builder().force(true).build());
            page.locator(&selector)
                .await
                .click(opts)
                .await
                .context("locator.click")?;
            Ok(Value::Null)
        }
        "press" => {
            let PressArgs { selector, key } = parse_args(args)?;
            page.locator(&selector)
                .await
                .press(&key, None)
                .await
                .context("locator.press")?;
            Ok(Value::Null)
        }
        "type" => {
            let TypeArgs { selector, text } = parse_args(args)?;
            page.locator(&selector)
                .await
                .press_sequentially(&text, None)
                .await
                .context("locator.press_sequentially")?;
            Ok(Value::Null)
        }
        "key" => {
            let KeyArgs { key } = parse_args(args)?;
            page.keyboard()
                .press(&key, None)
                .await
                .context("keyboard.press")?;
            Ok(Value::Null)
        }
        "fill" => {
            let FillArgs { selector, value } = parse_args(args)?;
            page.locator(&selector)
                .await
                .fill(&value, None)
                .await
                .context("locator.fill")?;
            Ok(Value::Null)
        }
        "fill_ref" => {
            let FillRefArgs {
                selector,
                value_ref,
                values,
            } = parse_args(args)?;
            let value = {
                let mut secrets = state.secrets.lock().await;
                secrets.resolve_spec(&value_ref, &values).await?
            };
            page.locator(&selector)
                .await
                .fill(&value, None)
                .await
                .context("locator.fill")?;
            Ok(Value::Null)
        }
        "secret_discover" => {
            let SecretDiscoverArgs { query, limit } = parse_args(args)?;
            let search = query
                .search
                .as_deref()
                .map(str::trim)
                .filter(|search| !search.is_empty())
                .context("secret_discover requires a non-empty search query")?;
            if search.len() < 2 {
                bail!("secret_discover search query must be at least 2 characters");
            }
            let mut matches = {
                let secrets = state.secrets.lock().await;
                secrets.discover(&query).await?
            };
            matches.truncate(limit.clamp(1, 25));
            serde_json::to_value(matches).context("serialize secret_discover response")
        }
        "select_option" => {
            let FillArgs { selector, value } = parse_args(args)?;
            page.locator(&selector)
                .await
                .select_option(value.as_str(), None)
                .await
                .context("locator.select_option")?;
            Ok(Value::Null)
        }
        "wait_for" => {
            let WaitForArgs {
                selector,
                timeout_ms,
            } = parse_args(args)?;
            wait_for_any_match(page, &selector, timeout_ms).await?;
            Ok(Value::Null)
        }
        "url" => Ok(json!(page.url())),
        "hide_window" => {
            window::hide(page).await.context("hide browser window")?;
            Ok(Value::Null)
        }
        "surface_window" => {
            window::surface(page)
                .await
                .context("surface browser window")?;
            Ok(Value::Null)
        }
        "locator_count" => {
            let SelectorArgs { selector } = parse_args(args)?;
            let n = page
                .locator(&selector)
                .await
                .count()
                .await
                .context("locator.count")?;
            Ok(json!(n))
        }
        "locator_visible_count" => {
            let SelectorArgs { selector } = parse_args(args)?;
            let n = locator_visible_count(page, &selector).await?;
            Ok(json!(n))
        }
        "url_template" => {
            let ValuesArgs { values } = parse_args(args)?;
            let url = page.url();
            let templated = {
                let mut secrets = state.secrets.lock().await;
                secrets.template_url(&url, &values).await
            };
            Ok(json!(templated))
        }
        "content" => {
            let html = page.content().await.context("page.content")?;
            Ok(json!(html))
        }
        "dump_masked" => {
            let MaskedDumpArgs {
                mask_config,
                site_elements,
                place,
                values,
            } = parse_args(args)?;
            let html = page.content().await.context("page.content")?;
            let policy =
                MaskPolicy::build(&mask_config, &site_elements).context("building mask policy")?;
            let dummy = dummy_place();
            let place_ref = place.as_ref().unwrap_or(&dummy);
            let value_names = {
                let mut secrets = state.secrets.lock().await;
                secrets.value_name_map(&values).await
            };
            let effective = policy.for_place(place_ref);
            let masked = effective
                .apply(&html, &value_names)
                .context("masking captured DOM")?;
            Ok(json!(masked.0))
        }
        "selector_text_raw" => {
            let SelectorArgs { selector } = parse_args(args)?;
            let snippets = selector_text(page, &selector).await?;
            serde_json::to_value(snippets).context("serialize selector_text_raw response")
        }
        "selector_attr_raw" => {
            let SelectorAttrArgs { selector, attr } = parse_args(args)?;
            let locator = page.locator(&selector).await;
            let count = locator.count().await.context("locator.count")?;
            let mut out: Vec<String> = Vec::with_capacity(count);
            for i in 0..count {
                let v = locator
                    .nth(i as i32)
                    .get_attribute(&attr)
                    .await
                    .context("locator.get_attribute")?;
                out.push(v.unwrap_or_default());
            }
            serde_json::to_value(out).context("serialize selector_attr_raw response")
        }
        "request_unmask_approval" => {
            let ApprovalArgs {
                mut context,
                selector,
                proposed_name,
            } = parse_args(args)?;
            if context.current_url.is_none() {
                context.current_url = Some(page.url());
            }
            let snippets = selector_text(page, &selector).await?;
            let out = UnmaskedSnippets::new(context, selector, proposed_name, snippets);
            serde_json::to_value(out).context("serialize unmask approval response")
        }
        "aria_snapshot" => {
            let yaml = page
                .locator("body")
                .await
                .aria_snapshot()
                .await
                .context("locator(body).aria_snapshot")?;
            Ok(json!(yaml))
        }
        "evaluate" => {
            let EvaluateArgs { js } = parse_args(args)?;
            let v: Value = page
                .evaluate(&js, None::<&()>)
                .await
                .context("page.evaluate")?;
            Ok(v)
        }
        other => bail!("unknown op: {other}"),
    }
}

async fn selector_text(page: &PwPage, selector: &str) -> Result<Vec<RawSnippet>> {
    let texts = page
        .locator(selector)
        .await
        .all_text_contents()
        .await
        .context("locator.all_text_contents")?;
    Ok(texts
        .into_iter()
        .enumerate()
        .map(|(i, text)| RawSnippet::new(i, text))
        .collect())
}

async fn locator_visible_count(page: &PwPage, selector: &str) -> Result<usize> {
    let locator = page.locator(selector).await;
    let count = locator.count().await.context("locator.count")?;
    let mut visible = 0;
    for idx in 0..count {
        if locator
            .nth(idx as i32)
            .is_visible()
            .await
            .context("locator.nth(...).is_visible")?
        {
            visible += 1;
        }
    }
    Ok(visible)
}

async fn wait_for_any_match(page: &PwPage, selector: &str, timeout_ms: u64) -> Result<()> {
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let count = page
            .locator(selector)
            .await
            .count()
            .await
            .with_context(|| format!("locator.count [selector: {selector}]"))?;
        if count > 0 {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("timed out after {timeout_ms}ms waiting for selector: {selector}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T> {
    serde_json::from_value(args).context("parsing rpc args")
}

#[derive(Deserialize)]
struct GotoArgs {
    url: String,
}

#[derive(Deserialize)]
struct GotoTemplateArgs {
    url: String,
    values: ValuesConfig,
}

#[derive(Deserialize)]
struct SelectorArgs {
    selector: String,
}

#[derive(Deserialize)]
struct SelectorAttrArgs {
    selector: String,
    attr: String,
}

#[derive(Deserialize)]
struct ApprovalArgs {
    context: UnmaskApprovalContext,
    selector: String,
    proposed_name: Option<String>,
}

#[derive(Deserialize)]
struct FillArgs {
    selector: String,
    value: String,
}

#[derive(Deserialize)]
struct ClickArgs {
    selector: String,
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize)]
struct PressArgs {
    selector: String,
    key: String,
}

#[derive(Deserialize)]
struct TypeArgs {
    selector: String,
    text: String,
}

#[derive(Deserialize)]
struct KeyArgs {
    key: String,
}

#[derive(Deserialize)]
struct EvaluateArgs {
    js: String,
}

#[derive(Deserialize)]
struct FillRefArgs {
    selector: String,
    value_ref: String,
    values: ValuesConfig,
}

#[derive(Deserialize)]
struct SecretDiscoverArgs {
    query: stencil_secrets::SecretDiscoveryQuery,
    #[serde(default = "default_secret_discover_limit")]
    limit: usize,
}

fn default_secret_discover_limit() -> usize {
    10
}

#[derive(Deserialize)]
struct ValuesArgs {
    values: ValuesConfig,
}

#[derive(Deserialize)]
struct MaskedDumpArgs {
    mask_config: MaskConfig,
    site_elements: Vec<Element>,
    place: Option<Place>,
    values: ValuesConfig,
}

#[derive(Deserialize)]
struct WaitForArgs {
    selector: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn dummy_place() -> Place {
    Place {
        name: "_page".into(),
        url: None,
        from: None,
        via: None,
        interactive: false,
        submit: None,
        signature: Signature::default(),
        completion: None,
        redirect: None,
        elements: vec![],
    }
}
