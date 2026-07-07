//! GUI e2e smoke over WebDriver (tauri-driver + WebKitWebDriver). Drives the
//! REAL desktop app's DOM — no xdotool, no coordinates, no focus games.
//!
//! Run: `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture`
//! Needs: a display, `tauri-driver` + `WebKitWebDriver` on PATH, npm (Vite is
//! auto-spawned if :5173 is down). `turn_smoke` additionally needs llama-server
//! on :8080 (qwen3.6-35b-a3b).

mod e2e_harness;

use e2e_harness::Gui;
use std::time::Duration;
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
