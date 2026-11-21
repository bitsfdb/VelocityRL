//! Game-owned files must not be repaired while Rocket League has them open.

pub const DEFERRED_MESSAGE: &str =
    "Game-file maintenance is deferred until Rocket League closes.";

pub fn ensure_idle() -> Result<(), String> {
    run_if_idle(|| Ok(()))
}

fn run_with_state<T>(game_running: bool, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if game_running {
        Err(DEFERRED_MESSAGE.into())
    } else {
        operation()
    }
}

pub fn run_if_idle<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    run_with_state(crate::psynet::rocket_league_process().is_some(), operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_game_prevents_file_access() {
        let path = std::env::temp_dir().join(format!("velocityrl-running-game-{}.tmp", std::process::id()));
        let result = run_with_state(true, || {
            std::fs::write(&path, b"must not be written").map_err(|e| e.to_string())
        });
        assert_eq!(result.unwrap_err(), DEFERRED_MESSAGE);
        assert!(!path.exists());
    }

    #[test]
    fn closed_game_allows_work_and_preserves_errors() {
        let mut attempts = 0;
        assert!(run_with_state(true, || { attempts += 1; Ok(()) }).is_err());
        let result: Result<(), String> = run_with_state(false, || {
            attempts += 1;
            Err("disk unavailable".into())
        });
        assert_eq!(result.unwrap_err(), "disk unavailable");
        assert_eq!(attempts, 1);
    }
}
