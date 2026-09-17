pub mod models;
pub mod session_tracker;
pub mod stats_api;
pub mod commands;

pub use commands::*;
pub use models::*;
pub use session_tracker::SessionTracker;

use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

pub fn init(app: &AppHandle) -> Arc<Mutex<SessionTracker>> {
    let config_dir = app.path().app_config_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tracker = Arc::new(Mutex::new(SessionTracker::new(config_dir)));

    let tracker_clone = Arc::clone(&tracker);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        stats_api::run_stats_api_loop(app_handle, tracker_clone).await;
    });

    tracker
}

pub fn create_overlay_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let url = tauri::WebviewUrl::App("tracker_overlay.html".into());
    match tauri::WebviewWindowBuilder::new(app, "tracker_overlay", url)
        .title("VelocityRL Overlay")
        .inner_size(400.0, 270.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()
    {
        Ok(win) => {
            let _ = win.set_always_on_top(true);
            let _ = commands::tracker_position_overlay(app.handle().clone(), "bottom-right".into());
            crate::applog::event("tracker: overlay window created (hidden) in setup");
        }
        Err(e) => {
            crate::applog::event(&format!("tracker: creating overlay in setup returned: {e}"));
        }
    }
    Ok(())
}
