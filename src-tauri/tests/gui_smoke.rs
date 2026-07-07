//! GUI e2e smoke over WebDriver (tauri-driver + WebKitWebDriver). Drives the
//! REAL desktop app's DOM — no xdotool, no coordinates, no focus games.
//!
//! Run: `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture`
//! Needs: a display, `tauri-driver` + `WebKitWebDriver` on PATH, npm (Vite is
//! auto-spawned if :5173 is down). `turn_smoke` additionally needs llama-server
//! on :8080 (qwen3.6-35b-a3b).

mod e2e_harness;

use e2e_harness::Gui;
use std::time::{Duration, Instant};
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite (auto-spawned)"]
async fn boot_smoke() {
    let gui = Gui::launch().await;

    // Composer rendered → frontend loaded from Vite into the webview.
    gui.driver
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer textarea should render");

    // Architecture/Config tabs only render when isTauri() sees real IPC —
    // this is the assertion that we drove the app, not a plain browser.
    let tabs = gui
        .driver
        .query(By::Css("button[role='tab']"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .all_from_selector()
        .await
        .expect("tab buttons");
    let mut labels = Vec::new();
    for t in &tabs {
        labels.push(t.text().await.unwrap_or_default());
    }
    assert!(
        labels.contains(&"Architecture".to_string()) && labels.contains(&"Config".to_string()),
        "expected Tauri-only tabs in {labels:?}"
    );

    gui.shutdown().await;
}

#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite + llama-server on :8080 (qwen3.6-35b-a3b)"]
async fn turn_smoke() {
    // Gate on the model server exactly like smoke_context_explorer.
    let health = reqwest::get("http://localhost:8080/health").await;
    assert!(
        health.map(|r| r.status().is_success()).unwrap_or(false),
        "llama-server not healthy on :8080 — start it before this test"
    );

    let gui = Gui::launch().await;
    let d = &gui.driver;

    let marker = "SQUIRREL7";
    let ta = d
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer");
    ta.click().await.expect("focus composer");
    ta.send_keys(format!("Reply with exactly: {marker}")).await.expect("type prompt");
    // \u{E007} is the W3C WebDriver Enter keycode; the composer submits on Enter.
    ta.send_keys("\u{E007}").await.expect("send Enter");

    // The user bubble echoes the marker once; the assistant reply makes it >= 2.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let ret = d
            .execute("return document.body.innerText;", vec![])
            .await
            .expect("read innerText");
        let text = ret.json().as_str().unwrap_or_default().to_string();
        if text.matches(marker).count() >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "no assistant reply containing {marker} within 120s");
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }

    // Context tab shows a populated breakdown: the total line and the system chip.
    d.query(By::XPath("//button[@role='tab' and normalize-space()='Context']"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .first()
        .await
        .expect("Context tab")
        .click()
        .await
        .expect("click Context tab");
    d.query(By::XPath("//div[contains(., 'tokens')]"))
        .wait(Duration::from_secs(15), Duration::from_millis(500))
        .first()
        .await
        .expect("breakdown total line ('N / M tokens')");
    d.query(By::XPath("//button[contains(., 'system')]"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .first()
        .await
        .expect("system legend chip");

    // Evidence screenshot (webview-exact, focus-independent).
    let _ = d
        .screenshot(std::path::Path::new("target/gui_smoke_turn.png"))
        .await;

    gui.shutdown().await;
}
