use anyhow::{Context, Result, bail};
use playwright_rs::protocol::page::Page as PwPage;
use serde_json::{Value, json};
use tracing::info;

const OFFSCREEN_LEFT: i64 = -32_000;
const OFFSCREEN_TOP: i64 = 0;
const SURFACE_LEFT: i64 = 80;
const SURFACE_TOP: i64 = 80;
const WINDOW_WIDTH: i64 = 1280;
const WINDOW_HEIGHT: i64 = 900;

pub(super) fn offscreen_launch_args() -> Vec<String> {
    vec![
        format!("--window-position={OFFSCREEN_LEFT},{OFFSCREEN_TOP}"),
        format!("--window-size={WINDOW_WIDTH},{WINDOW_HEIGHT}"),
    ]
}

pub(super) async fn hide(page: &PwPage) -> Result<()> {
    let cdp = page
        .context()
        .context("getting page browser context")?
        .new_cdp_session(page)
        .await
        .context("opening Chrome DevTools Protocol session")?;
    let window_id = window_id_for_target(&cdp).await?;

    let offscreen = cdp
        .send(
            "Browser.setWindowBounds",
            Some(json!({
                "windowId": window_id,
                "bounds": offscreen_bounds(),
            })),
        )
        .await;

    match offscreen {
        Ok(_) => {
            let actual = cdp
                .send("Browser.getWindowForTarget", None)
                .await
                .context("verifying off-screen Chrome window bounds")?;
            match window_bounds_from_response(&actual) {
                Ok(bounds) if bounds.is_far_left_offscreen() => {}
                Ok(bounds) => {
                    info!(
                        ?bounds,
                        "Chrome clamped off-screen bounds; minimizing browser window"
                    );
                    minimize(&cdp, window_id)
                        .await
                        .context("minimizing Chrome window after off-screen bounds were clamped")?;
                }
                Err(err) => {
                    info!(
                        error = %err,
                        "could not verify off-screen bounds; minimizing browser window"
                    );
                    minimize(&cdp, window_id)
                        .await
                        .context("minimizing Chrome window after off-screen verification failed")?;
                }
            }
        }
        Err(offscreen_err) => {
            minimize(&cdp, window_id).await.with_context(|| {
                format!(
                    "moving Chrome window off-screen failed before minimize fallback: {offscreen_err}"
                )
            })?;
        }
    }

    let _ = cdp.detach().await;
    Ok(())
}

pub(super) async fn surface(page: &PwPage) -> Result<()> {
    let cdp = page
        .context()
        .context("getting page browser context")?
        .new_cdp_session(page)
        .await
        .context("opening Chrome DevTools Protocol session")?;
    let window_id = window_id_for_target(&cdp).await?;

    cdp.send(
        "Browser.setWindowBounds",
        Some(json!({
            "windowId": window_id,
            "bounds": { "windowState": "normal" },
        })),
    )
    .await
    .context("restoring Chrome window state")?;

    cdp.send(
        "Browser.setWindowBounds",
        Some(json!({
            "windowId": window_id,
            "bounds": surface_bounds(),
        })),
    )
    .await
    .context("moving Chrome window on-screen")?;

    let _ = cdp.detach().await;
    page.bring_to_front()
        .await
        .context("bringing Chrome page to front")?;
    Ok(())
}

async fn window_id_for_target(cdp: &playwright_rs::protocol::CDPSession) -> Result<i64> {
    let result = cdp
        .send("Browser.getWindowForTarget", None)
        .await
        .context("Browser.getWindowForTarget")?;
    window_id_from_response(&result)
}

async fn minimize(cdp: &playwright_rs::protocol::CDPSession, window_id: i64) -> Result<()> {
    cdp.send(
        "Browser.setWindowBounds",
        Some(json!({
            "windowId": window_id,
            "bounds": { "windowState": "minimized" },
        })),
    )
    .await?;
    Ok(())
}

fn window_id_from_response(result: &Value) -> Result<i64> {
    let window_id = response_body(result)
        .get("windowId")
        .and_then(Value::as_i64);
    let Some(window_id) = window_id else {
        bail!("Browser.getWindowForTarget returned no integer windowId: {result}");
    };
    Ok(window_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowBounds {
    left: i64,
    width: i64,
}

impl WindowBounds {
    fn is_far_left_offscreen(&self) -> bool {
        self.left <= OFFSCREEN_LEFT / 2
    }
}

fn window_bounds_from_response(result: &Value) -> Result<WindowBounds> {
    let bounds = response_body(result)
        .get("bounds")
        .and_then(Value::as_object)
        .with_context(|| format!("Browser.getWindowForTarget returned no bounds: {result}"))?;
    let left = bounds
        .get("left")
        .and_then(Value::as_i64)
        .with_context(|| format!("Browser.getWindowForTarget returned no bounds.left: {result}"))?;
    let width = bounds
        .get("width")
        .and_then(Value::as_i64)
        .with_context(|| {
            format!("Browser.getWindowForTarget returned no bounds.width: {result}")
        })?;
    Ok(WindowBounds { left, width })
}

fn response_body(result: &Value) -> &Value {
    result.get("result").unwrap_or(result)
}

fn offscreen_bounds() -> Value {
    json!({
        "windowState": "normal",
        "left": OFFSCREEN_LEFT,
        "top": OFFSCREEN_TOP,
        "width": WINDOW_WIDTH,
        "height": WINDOW_HEIGHT,
    })
}

fn surface_bounds() -> Value {
    json!({
        "left": SURFACE_LEFT,
        "top": SURFACE_TOP,
        "width": WINDOW_WIDTH,
        "height": WINDOW_HEIGHT,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offscreen_launch_args_start_chrome_outside_the_visible_desktop() {
        let args = offscreen_launch_args();
        assert_eq!(
            args,
            vec![
                "--window-position=-32000,0".to_string(),
                "--window-size=1280,900".to_string(),
            ]
        );
    }

    #[test]
    fn window_id_from_response_requires_integer_window_id() {
        assert_eq!(
            window_id_from_response(&json!({ "windowId": 42 })).unwrap(),
            42
        );
        assert_eq!(
            window_id_from_response(&json!({ "result": { "windowId": 42 } })).unwrap(),
            42
        );
        assert!(window_id_from_response(&json!({ "windowId": "42" })).is_err());
        assert!(window_id_from_response(&json!({})).is_err());
    }

    #[test]
    fn window_bounds_from_response_accepts_wrapped_cdp_result() {
        let bounds = window_bounds_from_response(&json!({
            "result": {
                "windowId": 42,
                "bounds": {
                    "left": -1200,
                    "top": 0,
                    "width": 1282,
                    "height": 846,
                    "windowState": "normal"
                }
            }
        }))
        .unwrap();
        assert_eq!(
            bounds,
            WindowBounds {
                left: -1200,
                width: 1282,
            }
        );
        assert!(!bounds.is_far_left_offscreen());
    }

    #[test]
    fn window_bounds_from_response_accepts_truly_offscreen_position() {
        let bounds = window_bounds_from_response(&json!({
            "bounds": {
                "left": -32000,
                "width": 1280,
            }
        }))
        .unwrap();
        assert!(bounds.is_far_left_offscreen());
    }
}
