use std::sync::{Arc, Mutex};
use std::time::Duration;
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use crate::tracker::models::*;
use crate::tracker::session_tracker::SessionTracker;

pub async fn run_stats_api_loop(app: AppHandle, tracker: Arc<Mutex<SessionTracker>>) {
    let mut last_connected = false;
    let mut last_fix_attempt: Option<std::time::Instant> = None;
    loop {
        let ws_url = {
            let mut t = tracker.lock().unwrap();
            let game_dir = crate::tracker::commands::stats_api_game_dir(&app);
            let binding =
                crate::tracker::commands::check_stats_api_binding(&game_dir);
            if t.status_detail != binding.detail {
                t.status_detail = binding.detail.clone();
                let _ = app.emit("overlay-state", t.to_overlay_payload());
            }
            let fix_due = last_fix_attempt
                .map(|t| t.elapsed() >= std::time::Duration::from_secs(60))
                .unwrap_or(true);
            if binding.needs_fix && fix_due {
                last_fix_attempt = Some(std::time::Instant::now());
                crate::applog::event(&format!(
                    "tracker: Stats API binding broken, rewriting configs ({})",
                    binding.detail
                ));
                let _ = crate::tracker::commands::ensure_stats_api_files(&game_dir);
            }
            t.config.ws_url.clone()
        };

        match connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
                crate::applog::event("tracker: connected to Stats API WebSocket");
                last_connected = true;
                {
                    let mut t = tracker.lock().unwrap();
                    t.connection_status = "connected".into();
                    t.status_detail = "Stats API connected".into();
                    let _ = app.emit("overlay-state", t.to_overlay_payload());
                }

                let (_, mut read) = ws_stream.split();

                while let Some(msg_res) = read.next().await {
                    match msg_res {
                        Ok(msg) if msg.is_text() => {
                            if let Ok(text) = msg.to_text() {
                                process_event(&app, &tracker, text);
                            }
                        }
                        Ok(msg) if msg.is_close() => {
                            crate::applog::event("tracker: Stats API connection closed");
                            break;
                        }
                        Err(e) => {
                            crate::applog::event(&format!("tracker: Stats API read error: {e}"));
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                let detail = if binding_ws_refused(&e) {
                    "Rocket League running but Stats API port closed — restart RL after VelocityRL to bind it".into()
                } else {
                    format!("Stats API unreachable: {e}")
                };
                let mut t = tracker.lock().unwrap();
                if t.status_detail != detail {
                    t.status_detail = detail.clone();
                    let _ = app.emit("overlay-state", t.to_overlay_payload());
                }
            }
        }

        if last_connected {
            last_connected = false;
            let mut t = tracker.lock().unwrap();
            t.connection_status = "disconnected".into();
            t.is_match_active = false;
            let _ = app.emit("overlay-state", t.to_overlay_payload());
        }

        sleep(Duration::from_secs(3)).await;
    }
}

fn binding_ws_refused(e: &tokio_tungstenite::tungstenite::Error) -> bool {
    matches!(
        e,
        tokio_tungstenite::tungstenite::Error::Io(io)
            if io.kind() == std::io::ErrorKind::ConnectionRefused
    )
}

fn process_event(app: &AppHandle, tracker: &Arc<Mutex<SessionTracker>>, raw: &str) {
    let mut envelope: StatsEnvelope = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            crate::applog::event(&format!("tracker: StatsEnvelope parse error: {} | raw: {}", e, raw));
            return;
        }
    };

    if envelope.data.is_string() {
        match serde_json::from_str::<serde_json::Value>(envelope.data.as_str().unwrap_or("")) {
            Ok(inner) => envelope.data = inner,
            Err(_) => {}
        }
    }

    let mut t = tracker.lock().unwrap();
    match envelope.event.as_str() {
        "MatchCreated" | "MatchInitialized" => {
            // Wait for UpdateState with valid online match GUID before activating match state.
        }
        "RoundStarted" => {
            if t.active_match_guid.is_some() {
                t.is_match_active = true;
            }
        }
        "UpdateState" => {
            match serde_json::from_value::<UpdateStateData>(envelope.data.clone()) {
                Ok(data) => t.handle_update_state(data),
                Err(e) => {
                    crate::applog::event(&format!("tracker: UpdateState error: {}", e));
                }
            }
        }
        "MatchEnded" => {
            match serde_json::from_value::<MatchEndedData>(envelope.data.clone()) {
                Ok(ended) => t.handle_match_ended(Some(ended)),
                Err(e) => {
                    crate::applog::event(&format!("tracker: MatchEnded error: {}", e));
                    if t.active_match_guid.is_some() {
                        t.handle_match_ended(None);
                    }
                }
            }
        }
        "MatchDestroyed" => {
            if t.is_match_active && t.active_match_guid.is_some() {
                t.handle_match_ended(None);
            }
            t.is_match_active = false;
            t.active_match_guid = None;
        }
        _ => {}
    }

    let payload = t.to_overlay_payload();
    let _ = app.emit("overlay-state", payload);
}

pub fn simulate_fake_match(app: &AppHandle, tracker: &Arc<Mutex<SessionTracker>>, win: bool) {
    let mut t = tracker.lock().unwrap();
    t.connection_status = "connected".into();
    t.is_match_active = true;
    t.live_stats = LiveMatchStats {
        team_score: if win { 3 } else { 1 },
        opponent_score: if win { 2 } else { 4 },
        goals: if win { 2 } else { 0 },
        assists: 1,
        saves: 2,
        shots: 4,
        boost: 82,
        time_seconds: 0,
    };

    let mock_ended = MatchEndedData {
        winner_team_num: Some(if win { 0 } else { 1 }),
        match_guid: Some(format!("sim_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis())),
        mmr_delta: None,
        new_rating: None,
    };
    t.handle_match_ended(Some(mock_ended));

    let payload = t.to_overlay_payload();
    let _ = app.emit("overlay-state", payload);
}
