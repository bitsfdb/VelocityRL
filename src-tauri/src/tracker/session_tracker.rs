use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::tracker::models::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub session_rating: i32,
    pub wins: i32,
    pub losses: i32,
    pub streak_type: String,
    pub streak_count: i32,
    pub longest_win_streak: i32,
    pub finalized_matches: Vec<String>,
}

impl Default for PersistedSession {
    fn default() -> Self {
        Self {
            session_rating: 1000,
            wins: 0,
            losses: 0,
            streak_type: "none".into(),
            streak_count: 0,
            longest_win_streak: 0,
            finalized_matches: Vec::new(),
        }
    }
}

pub struct SessionTracker {
    pub config: OverlayConfig,
    pub session: PersistedSession,
    pub active_match_guid: Option<String>,
    pub is_match_active: bool,
    pub last_update: Option<UpdateStateData>,
    pub live_stats: LiveMatchStats,
    pub last_result: String,
    pub connection_status: String,
    pub status_detail: String,
    save_path: PathBuf,
}

impl SessionTracker {
    pub fn new(config_dir: PathBuf) -> Self {
        let save_path = config_dir.join("tracker_session.json");
        let session = PersistedSession::default();
        let config = OverlayConfig::default();

        let tracker = Self {
            config,
            session,
            active_match_guid: None,
            is_match_active: false,
            last_update: None,
            live_stats: LiveMatchStats::default(),
            last_result: "none".into(),
            connection_status: "disconnected".into(),
            status_detail: String::new(),
            save_path,
        };
        tracker.save();
        tracker
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.session) {
            let _ = fs::write(&self.save_path, json);
        }
    }

    pub fn reset_session(&mut self) {
        self.session = PersistedSession {
            session_rating: self.config.initial_rating,
            ..Default::default()
        };
        self.last_result = "none".into();
        self.save();
    }

    pub fn is_training_or_offline_state(data: &UpdateStateData) -> bool {
        // Online matches always have an authentic non-empty server match GUID.
        let has_valid_guid = data
            .match_guid
            .as_ref()
            .map(|g| !g.trim().is_empty() && !g.starts_with("session_match_"))
            .unwrap_or(false);

        if !has_valid_guid {
            return true;
        }

        if let Some(ref g) = data.game {
            if let Some(ref arena) = g.arena {
                let a = arena.to_lowercase();
                if a.contains("training") || a.contains("tutorial") || a.contains("editor") {
                    return true;
                }
            }
        }

        false
    }

    pub fn handle_update_state(&mut self, data: UpdateStateData) {
        if Self::is_training_or_offline_state(&data) {
            return;
        }

        if let Some(ref new_guid) = data.match_guid {
            if !new_guid.trim().is_empty() {
                if let Some(ref old_guid) = self.active_match_guid {
                    if old_guid != new_guid && self.is_match_active {
                        crate::applog::event(&format!("tracker: new match started ({new_guid}) while old match active ({old_guid}) -> auto-finalizing old match"));
                        self.handle_match_ended(None);
                    }
                }
                self.active_match_guid = Some(new_guid.clone());
            }
        }
        self.is_match_active = true;

        let mut parsed_players: Vec<PlayerData> = Vec::new();
        match data.players {
            serde_json::Value::Array(ref arr) => {
                for v in arr {
                    if let Ok(p) = serde_json::from_value(v.clone()) {
                        parsed_players.push(p);
                    }
                }
            }
            serde_json::Value::Object(ref obj) => {
                for (_, v) in obj {
                    if let Ok(p) = serde_json::from_value(v.clone()) {
                        parsed_players.push(p);
                    }
                }
            }
            _ => {}
        }

        let local_player = parsed_players.iter().find(|p| {
            (!self.config.player_primary_id.is_empty() && p.primary_id == self.config.player_primary_id)
                || (!self.config.player_name_fallback.is_empty() && p.name.eq_ignore_ascii_case(&self.config.player_name_fallback))
        }).or_else(|| parsed_players.first());

        if let Some(player) = local_player {
            let team_num = player.team_num;
            let mut team_score = 0;
            let mut opponent_score = 0;

            if let Some(ref game) = data.game {

                let mut parsed_teams: Vec<TeamData> = Vec::new();
                match &game.teams {
                    serde_json::Value::Array(arr) => {
                        for v in arr {
                            if let Ok(t) = serde_json::from_value(v.clone()) {
                                parsed_teams.push(t);
                            }
                        }
                    }
                    serde_json::Value::Object(obj) => {
                        for (_, v) in obj {
                            if let Ok(t) = serde_json::from_value(v.clone()) {
                                parsed_teams.push(t);
                            }
                        }
                    }
                    _ => {}
                }

                for team in parsed_teams {
                    if team.team_num == team_num {
                        team_score = team.score;
                    } else {
                        opponent_score = team.score;
                    }
                }
                self.live_stats.time_seconds = game.time_seconds.max(0.0) as i32;
            }

            self.live_stats.team_score = team_score;
            self.live_stats.opponent_score = opponent_score;
            self.live_stats.goals = player.goals;
            self.live_stats.assists = player.assists;
            self.live_stats.saves = player.saves;
            self.live_stats.shots = player.shots;
            self.live_stats.boost = player.boost.unwrap_or(0);
        }

        let mut final_data = data.clone();
        final_data.players = serde_json::to_value(parsed_players).unwrap_or(serde_json::Value::Null);
        self.last_update = Some(final_data);
    }

    pub fn handle_match_ended(&mut self, ended_data: Option<MatchEndedData>) {
        let guid = ended_data
            .as_ref()
            .and_then(|d| d.match_guid.clone())
            .or_else(|| self.active_match_guid.clone())
            .filter(|g| !g.trim().is_empty() && !g.starts_with("session_match_"));

        let Some(guid) = guid else {
            // No authentic match GUID was active; ignore menu transitions or offline events.
            self.is_match_active = false;
            self.active_match_guid = None;
            return;
        };

        let duplicate = self.session.finalized_matches.contains(&guid);
        if duplicate {
            crate::applog::event(&format!("tracker: match already finalized, skipping ({guid})"));
            return;
        }

        let mut outcome = "none";
        if let Some(ref data) = self.last_update {
            let parsed_players: Vec<PlayerData> = serde_json::from_value(data.players.clone()).unwrap_or_default();
            let local_player = parsed_players.iter().find(|p| {
                (!self.config.player_primary_id.is_empty() && p.primary_id == self.config.player_primary_id)
                    || (!self.config.player_name_fallback.is_empty() && p.name.eq_ignore_ascii_case(&self.config.player_name_fallback))
            }).or_else(|| parsed_players.first());

            if let Some(p) = local_player {
                let my_team = p.team_num;

                let mut team_score = self.live_stats.team_score;
                let mut opponent_score = self.live_stats.opponent_score;
                if let Some(ref game) = data.game {
                    let mut parsed_teams: Vec<TeamData> = Vec::new();
                    match &game.teams {
                        serde_json::Value::Array(arr) => {
                            for v in arr {
                                if let Ok(t) = serde_json::from_value(v.clone()) {
                                    parsed_teams.push(t);
                                }
                            }
                        }
                        serde_json::Value::Object(obj) => {
                            for (_, v) in obj {
                                if let Ok(t) = serde_json::from_value(v.clone()) {
                                    parsed_teams.push(t);
                                }
                            }
                        }
                        _ => {}
                    }
                    if !parsed_teams.is_empty() {
                        for team in parsed_teams {
                            if team.team_num == my_team {
                                team_score = team.score;
                            } else {
                                opponent_score = team.score;
                            }
                        }
                    }
                }

                let winner_team: Option<i32> = ended_data
                    .as_ref()
                    .and_then(|d| d.winner_team_num)
                    .or_else(|| {
                        data.game.as_ref().and_then(|g| {
                            g.winner.as_ref().and_then(|w| match w.to_ascii_lowercase().as_str() {
                                "blue" | "0" => Some(0),
                                "orange" | "1" => Some(1),
                                _ => None,
                            })
                        })
                    })
                    .or_else(|| {
                        if team_score > opponent_score {
                            Some(my_team)
                        } else if team_score < opponent_score {
                            Some(if my_team == 0 { 1 } else { 0 })
                        } else {
                            None
                        }
                    });

                if let Some(winner) = winner_team {
                    if winner == my_team {
                        outcome = "win";
                    } else {
                        outcome = "loss";
                    }
                }
            }
        }

        crate::applog::event(&format!(
            "tracker: match ended guid={guid} outcome={outcome} score={}/{} wins={} losses={}",
            self.live_stats.team_score, self.live_stats.opponent_score,
            self.session.wins, self.session.losses
        ));

        match outcome {
            "win" => {
                self.session.wins += 1;
                if self.session.streak_type == "win" {
                    self.session.streak_count += 1;
                } else {
                    self.session.streak_type = "win".into();
                    self.session.streak_count = 1;
                }
                if self.session.streak_count > self.session.longest_win_streak {
                    self.session.longest_win_streak = self.session.streak_count;
                }
                self.last_result = "win".into();
            }
            "loss" => {
                self.session.losses += 1;
                if self.session.streak_type == "loss" {
                    self.session.streak_count += 1;
                } else {
                    self.session.streak_type = "loss".into();
                    self.session.streak_count = 1;
                }
                self.last_result = "loss".into();
            }
            _ => {}
        }

        self.session.finalized_matches.push(guid);
        if self.session.finalized_matches.len() > 100 {
            self.session.finalized_matches.remove(0);
        }
        self.is_match_active = false;
        self.save();
    }

    pub fn to_overlay_payload(&self) -> OverlayStatePayload {
        let net_mmr = self.session.session_rating - self.config.initial_rating;
        OverlayStatePayload {
            connection: self.connection_status.clone(),
            status_detail: self.status_detail.clone(),
            match_active: self.is_match_active,
            session_rating: self.session.session_rating,
            net_mmr,
            rating_min: 0,
            rating_max: 3000,
            wins: self.session.wins,
            losses: self.session.losses,
            streak: StreakInfo {
                streak_type: self.session.streak_type.clone(),
                count: self.session.streak_count,
            },
            live: self.live_stats.clone(),
            last_result: self.last_result.clone(),
            playlist: 11,
            playlist_name: "2v2 Doubles".into(),
        }
    }
}
