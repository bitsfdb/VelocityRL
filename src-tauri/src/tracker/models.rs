use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatsEnvelope {
    #[serde(rename = "Event")]
    pub event: String,
    #[serde(rename = "Data", default)]
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateStateData {
    #[serde(rename = "MatchGuid", default)]
    pub match_guid: Option<String>,
    #[serde(rename = "Game", default)]
    pub game: Option<GameData>,
    #[serde(rename = "Players", default)]
    pub players: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GameData {
    #[serde(rename = "Arena", default)]
    pub arena: Option<String>,
    #[serde(rename = "bHasWinner", default)]
    pub has_winner: bool,
    #[serde(rename = "bOvertime", default)]
    pub overtime: bool,
    #[serde(rename = "TimeSeconds", default)]
    pub time_seconds: f64,

    #[serde(rename = "Winner", default)]
    pub winner: Option<String>,
    #[serde(rename = "Teams", default)]
    pub teams: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TeamData {
    #[serde(rename = "Score", default)]
    pub score: i32,
    #[serde(rename = "TeamNum", default)]
    pub team_num: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PlayerData {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "PrimaryId", default)]
    pub primary_id: String,
    #[serde(rename = "TeamNum", default)]
    pub team_num: i32,
    #[serde(rename = "Score", default)]
    pub score: i32,
    #[serde(rename = "Goals", default)]
    pub goals: i32,
    #[serde(rename = "Assists", default)]
    pub assists: i32,
    #[serde(rename = "Saves", default)]
    pub saves: i32,
    #[serde(rename = "Shots", default)]
    pub shots: i32,
    #[serde(rename = "Demos", default)]
    pub demos: i32,
    #[serde(rename = "Boost", default)]
    pub boost: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MatchEndedData {
    #[serde(rename = "WinnerTeamNum", alias = "winner_team_num", default)]
    pub winner_team_num: Option<i32>,
    #[serde(rename = "MatchGuid", alias = "match_guid", default)]
    pub match_guid: Option<String>,
    #[serde(rename = "MmrDelta", alias = "mmr_delta", alias = "delta", alias = "rating_change", default)]
    pub mmr_delta: Option<i32>,
    #[serde(rename = "NewRating", alias = "new_rating", alias = "rating", alias = "skill", alias = "player_skill", default)]
    pub new_rating: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreakInfo {
    #[serde(rename = "type")]
    pub streak_type: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiveMatchStats {
    pub team_score: i32,
    pub opponent_score: i32,
    pub goals: i32,
    pub assists: i32,
    pub saves: i32,
    pub shots: i32,
    pub boost: i32,
    pub time_seconds: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayStatePayload {
    pub connection: String,
    #[serde(default)]
    pub status_detail: String,
    pub match_active: bool,
    pub session_rating: i32,
    pub net_mmr: i32,
    pub rating_min: i32,
    pub rating_max: i32,
    pub wins: i32,
    pub losses: i32,
    pub streak: StreakInfo,
    pub live: LiveMatchStats,
    pub last_result: String,
    #[serde(default = "default_playlist")]
    pub playlist: i32,
    #[serde(default = "default_playlist_name")]
    pub playlist_name: String,
}

pub fn playlist_display_name(id: i32) -> &'static str {
    match id {
        10 => "1v1 Duel",
        11 => "2v2 Doubles",
        13 => "3v3 Standard",
        27 => "Hoops",
        28 => "Rumble",
        29 => "Dropshot",
        30 => "Snow Day",
        34 => "Tournament",
        _ => "Ranked",
    }
}

fn default_playlist() -> i32 { 11 }
fn default_playlist_name() -> String { "2v2 Doubles".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    pub ws_url: String,
    pub player_primary_id: String,
    pub player_name_fallback: String,
    pub initial_rating: i32,
    pub win_delta: i32,
    pub loss_delta: i32,
    pub rating_min: i32,
    pub rating_max: i32,
    pub is_locked: bool,
    pub position: String,
    pub playlist: i32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://127.0.0.1:49124".into(),
            player_primary_id: "".into(),
            player_name_fallback: "".into(),
            initial_rating: 0,
            win_delta: 9,
            loss_delta: 9,
            rating_min: 0,
            rating_max: 3000,
            is_locked: false,
            position: "bottom-right".into(),
            playlist: 11,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerUiSession {
    #[serde(default = "default_true")]
    pub master_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_launch_game: bool,
    #[serde(default = "default_position")]
    pub position: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_scale")]
    pub scale: i32,
    #[serde(default = "default_opacity")]
    pub opacity: i32,
    #[serde(default = "default_style")]
    pub overlay_style: String,
    #[serde(default = "default_true")]
    pub is_locked: bool,
    #[serde(default = "default_delta")]
    pub win_delta: i32,
    #[serde(default = "default_delta")]
    pub loss_delta: i32,
    #[serde(default = "default_playlist")]
    pub playlist: i32,
}

fn default_delta() -> i32 { 9 }

fn default_true() -> bool { true }
fn default_position() -> String { "bottom-right".into() }
fn default_scale() -> i32 { 100 }
fn default_opacity() -> i32 { 85 }
fn default_style() -> String { "circle".into() }

impl Default for TrackerUiSession {
    fn default() -> Self {
        Self {
            master_enabled: true,
            auto_launch_game: true,
            position: "bottom-right".into(),
            display_name: "".into(),
            scale: 100,
            opacity: 85,
            overlay_style: "circle".into(),
            is_locked: true,
            win_delta: 9,
            loss_delta: 9,
            playlist: 11,
        }
    }
}
