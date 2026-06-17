use anyhow::{Context, Result, bail};
use playwright_rs::protocol::page::Page as PwPage;
use serde_json::{Value, json};

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

    if let Err(offscreen_err) = offscreen {
        cdp.send(
            "Browser.setWindowBounds",
            Some(json!({
                "windowId": window_id,
                "bounds": { "windowState": "minimized" },
            })),
        )
        .await
        .with_context(|| {
            format!(
                "moving Chrome window off-screen failed before minimize fallback: {offscreen_err}"
            )
        })?;
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

fn window_id_from_response(result: &Value) -> Result<i64> {
    let Some(window_id) = result.get("windowId").and_then(Value::as_i64) else {
        bail!("Browser.getWindowForTarget returned no integer windowId: {result}");
    };
    Ok(window_id)
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
        assert!(window_id_from_response(&json!({ "windowId": "42" })).is_err());
        assert!(window_id_from_response(&json!({})).is_err());
    }
}
