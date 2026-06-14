//! `place_goto`: recognize, navigate on miss, recover, mask. The
//! single workhorse operation behind `stencilwright <site> place <name> goto`.
//!
//! Behavior per spec §5, with concrete refinements:
//!
//! - First recognize the live page. If it already matches the target,
//!   dump immediately without navigation.
//! - Otherwise navigate to target.url; SETTLE_DELAY for SPA hydration;
//!   `recognize()` over the whole graph.
//! - Match on the recognized place:
//!   - Target: capture and return.
//!   - Interactive: `auto_fill` what we have, run any configured
//!     submit click if all fills succeeded, then poll for the
//!     place's `completion` signature (or, if absent, poll until
//!     `recognize()` returns a *different* place). After that, do
//!     NOT re-navigate — the user may already be at target after
//!     the flow, and re-navigating can re-trigger captchas. Just
//!     re-recognize on the next iteration.
//!   - Other non-interactive: bail (transitions are a v2 concern).
//!   - None (unrecognized): poll `recognize()` over the whole graph
//!     until a known place is reached, then re-loop. No stdin
//!     involvement; this works headless from any spawning shell.
//! - Cap at MAX_ATTEMPTS iterations; HUMAN_TIMEOUT per poll loop.
//! - Every LOG_INTERVAL inside a poll, print a progress tick so the
//!   user knows we're alive.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use stencil_browser::Page;
use stencil_core::{Element, Place, Signature, Submit};
use stencil_mask::MaskedHtml;
use tokio::io::{AsyncBufReadExt, BufReader, stdin};
use tracing::{info, warn};

use crate::PlaceGraph;
use crate::recognize::{PlaceMatch, recognize, signature_matches};

const MAX_ATTEMPTS: usize = 5;
const SETTLE_DELAY: Duration = Duration::from_millis(1500);
const HUMAN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const LOG_INTERVAL: Duration = Duration::from_secs(15);

pub async fn place_goto(graph: &PlaceGraph, page: &Page, target_name: &str) -> Result<MaskedHtml> {
    let target = graph
        .place(target_name)
        .ok_or_else(|| anyhow!("unknown place: '{target_name}'"))?;

    tokio::time::sleep(SETTLE_DELAY).await;
    if let Some(m) = recognize(graph, page).await? {
        if m.place_name == target_name {
            return complete_target(graph, page, target, target_name, true).await;
        }
    }

    let target_url_template = target.url.as_deref().ok_or_else(|| {
        anyhow!(
            "place '{target_name}' has no `url` field (transition-only places not supported in v1)"
        )
    })?;
    let mut needs_navigation = true;

    for attempt in 1..=MAX_ATTEMPTS {
        if needs_navigation {
            info!(
                target = target_name,
                attempt,
                url = target_url_template,
                "navigating"
            );
            ui(format!(
                "→ [{attempt}/{MAX_ATTEMPTS}] navigating to {target_url_template}"
            ));
            page.goto_template(target_url_template, &graph.values)
                .await
                .with_context(|| format!("goto {target_url_template}"))?;
        }
        tokio::time::sleep(SETTLE_DELAY).await;

        let here = recognize(graph, page).await?;
        match here {
            Some(m) if m.place_name == target_name => {
                return complete_target(graph, page, target, target_name, false).await;
            }
            Some(m) => {
                let place = graph
                    .place(&m.place_name)
                    .expect("recognize returned a place name from the graph");
                if let Some(redirect_url) = &place.redirect {
                    ui(format!(
                        "→ at '{}'; redirecting to {}",
                        place.name, redirect_url
                    ));
                    page.goto_template(redirect_url, &graph.values)
                        .await
                        .with_context(|| {
                            format!("goto {redirect_url} (redirect from {})", place.name)
                        })?;
                    needs_navigation = false;
                    continue;
                }
                if place.interactive {
                    handle_interactive(graph, page, place).await?;
                    needs_navigation = false;
                    continue;
                } else {
                    bail!(
                        "landed at non-interactive place '{}', no transition path to '{}' \
                         (transitions not supported in v1)",
                        place.name,
                        target_name
                    );
                }
            }
            None => {
                ui(format!(
                    "→ unrecognized; waiting for any known place to appear (timeout {}min)…",
                    HUMAN_TIMEOUT.as_secs() / 60,
                ));
                match wait_for_recognition(graph, page, HUMAN_TIMEOUT, "any place").await? {
                    Some(_) => {
                        // Re-loop without navigating: the next iteration's
                        // `recognize()` dispatches based on whichever place
                        // we landed in.
                        needs_navigation = false;
                        continue;
                    }
                    None => bail!(
                        "no known place appeared within {}min while unrecognized",
                        HUMAN_TIMEOUT.as_secs() / 60
                    ),
                }
            }
        }
    }
    bail!("exceeded {MAX_ATTEMPTS} attempts trying to reach '{target_name}'")
}

async fn complete_target(
    graph: &PlaceGraph,
    page: &Page,
    target: &Place,
    target_name: &str,
    already_here: bool,
) -> Result<MaskedHtml> {
    if target.interactive {
        if already_here {
            ui(format!(
                "→ already at interactive target '{target_name}' — handling flow"
            ));
        } else {
            ui(format!(
                "→ recognized interactive target '{target_name}' — handling flow"
            ));
        }
        handle_interactive(graph, page, target).await?;
        ui(format!(
            "→ interactive target '{target_name}' complete — dumping live DOM"
        ));
    } else if already_here {
        ui(format!("→ already at '{target_name}' — dumping live DOM"));
    } else {
        ui(format!("→ recognized target '{target_name}' — capturing"));
    }
    page.dump_masked(
        &graph.mask_config,
        &graph.site_elements,
        Some(target),
        &graph.values,
    )
    .await
}

/// Auto-fill what we can on an interactive place, then wait for the
/// flow to complete. Two completion strategies:
///
/// - If the place defines a `completion` signature, poll it.
/// - Otherwise, poll until `recognize()` returns a *different* place
///   (more robust for cases like CAPTCHA where the post-condition
///   isn't easily expressible as a single signature).
async fn handle_interactive(graph: &PlaceGraph, page: &Page, place: &Place) -> Result<()> {
    ui(format!(
        "→ at interactive place '{}'; auto-filling what we have…",
        place.name
    ));
    let report = auto_fill(page, &place.elements, graph).await?;
    if let Some(submit) = &place.submit {
        if report.failed == 0 {
            submit_interactive(page, submit, &place.name).await?;
        } else {
            warn!(
                place = %place.name,
                attempted = report.attempted,
                failed = report.failed,
                "skipping submit because at least one auto_fill failed",
            );
        }
    }
    if let Some(completion) = &place.completion {
        ui(format!(
            "→ complete the {} flow in Chrome; watching for completion signature (timeout {}min)…",
            place.name,
            HUMAN_TIMEOUT.as_secs() / 60,
        ));
        if !wait_for_signature(
            page,
            completion,
            HUMAN_TIMEOUT,
            &format!("{} completion", place.name),
        )
        .await?
        {
            bail!(
                "interactive completion for '{}' timed out after {}min",
                place.name,
                HUMAN_TIMEOUT.as_secs() / 60,
            );
        }
    } else {
        ui(format!(
            "→ complete the {} flow in Chrome; watching for any state change (timeout {}min)…",
            place.name,
            HUMAN_TIMEOUT.as_secs() / 60,
        ));
        if wait_until_left(graph, page, &place.name, HUMAN_TIMEOUT)
            .await?
            .is_none()
        {
            bail!(
                "no state change away from '{}' within {}min",
                place.name,
                HUMAN_TIMEOUT.as_secs() / 60,
            );
        }
    }
    ui(format!("→ left '{}'", place.name));
    Ok(())
}

/// For each element with `auto_fill`, ask the daemon to resolve the
/// reference and fill the field. Best-effort: a missing selector or
/// failed resolution logs a warning and we move on.
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoFillReport {
    pub attempted: usize,
    pub failed: usize,
}

pub async fn auto_fill(
    page: &Page,
    elements: &[Element],
    graph: &PlaceGraph,
) -> Result<AutoFillReport> {
    let mut report = AutoFillReport::default();
    for el in elements {
        let Some(uri) = &el.auto_fill else { continue };
        report.attempted += 1;
        if let Err(e) = page.fill_ref(&el.selector, uri, &graph.values).await {
            report.failed += 1;
            warn!(
                element = %el.name,
                selector = %el.selector,
                "auto_fill failed: {e:#}",
            );
        } else {
            info!(element = %el.name, "filled");
        }
    }
    Ok(report)
}

async fn submit_interactive(page: &Page, submit: &Submit, place_name: &str) -> Result<()> {
    if let Some(selector) = &submit.click {
        ui(format!("→ submitting '{place_name}'"));
        page.click(selector)
            .await
            .with_context(|| format!("click submit selector for '{place_name}': {selector}"))?;
    }
    Ok(())
}

/// Poll `signature_matches(sig, page)` every POLL_INTERVAL until it
/// returns true (Ok(true)) or `timeout` elapses (Ok(false)). Logs a
/// progress tick every LOG_INTERVAL.
async fn wait_for_signature(
    page: &Page,
    sig: &Signature,
    timeout: Duration,
    label: &str,
) -> Result<bool> {
    let start = Instant::now();
    let deadline = start + timeout;
    let mut last_log = start;
    loop {
        let url = page.url().await?;
        if signature_matches(sig, &url, page).await? {
            return Ok(true);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        if now.duration_since(last_log) >= LOG_INTERVAL {
            ui(format!(
                "→ still waiting for {label} ({}s elapsed, {}s left)…",
                now.duration_since(start).as_secs(),
                deadline.saturating_duration_since(now).as_secs(),
            ));
            last_log = now;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll `recognize()` over the whole graph until any place matches.
/// Returns the matched place name, or None on timeout.
async fn wait_for_recognition(
    graph: &PlaceGraph,
    page: &Page,
    timeout: Duration,
    label: &str,
) -> Result<Option<PlaceMatch>> {
    let start = Instant::now();
    let deadline = start + timeout;
    let mut last_log = start;
    loop {
        if let Some(m) = recognize(graph, page).await? {
            ui(format!("→ recognized '{}'", m.place_name));
            return Ok(Some(m));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        if now.duration_since(last_log) >= LOG_INTERVAL {
            ui(format!(
                "→ still waiting for {label} ({}s elapsed, {}s left)…",
                now.duration_since(start).as_secs(),
                deadline.saturating_duration_since(now).as_secs(),
            ));
            last_log = now;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Poll until `recognize()` returns a place whose name is not
/// `current_name`. Treats `None` (unrecognized) as "still waiting" —
/// transient unrecognized states during page transitions are normal.
async fn wait_until_left(
    graph: &PlaceGraph,
    page: &Page,
    current_name: &str,
    timeout: Duration,
) -> Result<Option<PlaceMatch>> {
    let start = Instant::now();
    let deadline = start + timeout;
    let mut last_log = start;
    loop {
        match recognize(graph, page).await? {
            Some(m) if m.place_name != current_name => return Ok(Some(m)),
            _ => {}
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        if now.duration_since(last_log) >= LOG_INTERVAL {
            ui(format!(
                "→ still on '{current_name}' ({}s elapsed, {}s left)…",
                now.duration_since(start).as_secs(),
                deadline.saturating_duration_since(now).as_secs(),
            ));
            last_log = now;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// User-facing line printed to stdout, flushed immediately so it
/// streams through pipes (`tail`, `tee`) without buffering surprise.
fn ui(line: String) {
    println!("{line}");
    let _ = io::stdout().flush();
}

/// Last-resort stdin Enter for places that lack a completion
/// signature AND can't be detected by recognize. Currently unused
/// — kept for future flows where polling can't express "done".
#[allow(dead_code)]
async fn wait_for_enter() -> Result<()> {
    let mut buf = String::new();
    BufReader::new(stdin())
        .read_line(&mut buf)
        .await
        .context("reading Enter from stdin")?;
    Ok(())
}
