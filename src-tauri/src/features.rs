use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

pub const VRL_BUILD_ID: i64 = -659612010;
const FEATURES_URL: &str = "https://api.velocityrl.tech/v2/features.json";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MaintenanceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub message: String,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            message: "VelocityRL servers undergoing routine maintenance.".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FeatureFlags {
    #[serde(default = "default_true")]
    pub psynet_proxy: bool,
    #[serde(default = "default_true")]
    pub item_swapper: bool,
    #[serde(default = "default_true")]
    pub custom_titles: bool,
    #[serde(default = "default_true")]
    pub fake_ranks: bool,
    #[serde(default = "default_true")]
    pub camera_spoof: bool,
    #[serde(default = "default_true")]
    pub rich_palette: bool,
    #[serde(default = "default_true")]
    pub dynamic_logos: bool,
    #[serde(default = "default_true")]
    pub blog_motd: bool,
    #[serde(default = "default_true")]
    pub workshop_browser: bool,
    #[serde(default = "default_false")]
    pub workshop_upload_enabled: bool,
    #[serde(default = "default_true")]
    pub live_tracker_overlay: bool,
    #[serde(default = "default_true")]
    pub name_spoof: bool,
    #[serde(default = "default_true")]
    pub leaderboard_spoof: bool,
    #[serde(default = "default_true")]
    pub credit_spoof: bool,
    #[serde(default = "default_true")]
    pub tagame_swapper: bool,
    #[serde(default = "default_true")]
    pub custom_avatar: bool,
    #[serde(default = "default_true")]
    pub auto_updater: bool,
    #[serde(default = "default_false")]
    pub replay_analysis_v2: bool,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            psynet_proxy: true,
            item_swapper: true,
            custom_titles: true,
            fake_ranks: true,
            camera_spoof: true,
            rich_palette: true,
            dynamic_logos: true,
            blog_motd: true,
            workshop_browser: true,
            workshop_upload_enabled: false,
            live_tracker_overlay: true,
            name_spoof: true,
            leaderboard_spoof: true,
            credit_spoof: true,
            tagame_swapper: true,
            custom_avatar: true,
            auto_updater: true,
            replay_analysis_v2: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AnnouncementConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub text: String,
}

pub fn get_client_build_id() -> i64 {
    option_env!("VRL_BUILD_ID")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(VRL_BUILD_ID)
}

fn deserialize_build_ids<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntOrVec {
        Int(i64),
        Vec(Vec<i64>),
    }
    match Option::<IntOrVec>::deserialize(deserializer)? {
        Some(IntOrVec::Int(n)) => Ok(vec![n]),
        Some(IntOrVec::Vec(v)) => Ok(v),
        None => Ok(Vec::new()),
    }
}

pub fn is_build_supported() -> bool {
    let feat = get_cached_features();
    let client_build = get_client_build_id();
    !feat.blacklisted_builds.contains(&client_build)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FeaturesConfig {
    #[serde(default = "default_version")]
    pub version: i32,
    #[serde(
        default,
        deserialize_with = "deserialize_build_ids",
        alias = "blacklisted_build_nums",
        alias = "blacklisted_build_num",
        alias = "blacklisted_build"
    )]
    pub blacklisted_builds: Vec<i64>,
    #[serde(default)]
    pub maintenance: MaintenanceConfig,
    #[serde(default)]
    pub flags: FeatureFlags,
    #[serde(default)]
    pub announcement: AnnouncementConfig,
    #[serde(default)]
    pub build_outdated: bool,
    #[serde(default)]
    pub client_build_num: i64,
}

fn default_version() -> i32 {
    1
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            version: 1,
            blacklisted_builds: Vec::new(),
            maintenance: MaintenanceConfig::default(),
            flags: FeatureFlags::default(),
            announcement: AnnouncementConfig::default(),
            build_outdated: false,
            client_build_num: get_client_build_id(),
        }
    }
}

static CACHED_FEATURES: std::sync::Mutex<Option<FeaturesConfig>> = std::sync::Mutex::new(None);

fn process_features(mut feat: FeaturesConfig) -> FeaturesConfig {
    let client_build = get_client_build_id();
    feat.client_build_num = client_build;
    if feat.blacklisted_builds.contains(&client_build) {
        feat.build_outdated = true;
        crate::applog::event(&format!(
            "features: CRITICAL: Build is blacklisted! Installed: {}",
            client_build
        ));
        feat.flags = FeatureFlags {
            psynet_proxy: false,
            item_swapper: false,
            custom_titles: false,
            fake_ranks: false,
            camera_spoof: false,
            rich_palette: false,
            dynamic_logos: false,
            blog_motd: false,
            workshop_browser: false,
            workshop_upload_enabled: false,
            live_tracker_overlay: false,
            name_spoof: false,
            leaderboard_spoof: false,
            credit_spoof: false,
            tagame_swapper: false,
            custom_avatar: false,
            auto_updater: true,
            replay_analysis_v2: false,
        };
    } else {
        feat.build_outdated = false;
    }
    feat
}

#[tauri::command]
pub async fn get_features(app: tauri::AppHandle) -> Result<FeaturesConfig, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    fs::create_dir_all(&config_dir).ok();

    let ver_path = config_dir.join("features.ver");
    let json_path = config_dir.join("features.json");

    let local_config = fs::read_to_string(&json_path)
        .ok()
        .and_then(|s| serde_json::from_str::<FeaturesConfig>(&s).ok());

    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    crate::applog::event(&format!("features: requesting remote config from {FEATURES_URL}"));

    match client.get(FEATURES_URL).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(feat) = resp.json::<FeaturesConfig>().await {
                crate::applog::event(&format!(
                    "features: received remote config v{} (blacklisted_builds={:?}, maintenance={})",
                    feat.version, feat.blacklisted_builds, feat.maintenance.enabled
                ));
                let processed = process_features(feat);
                if let Ok(serialized) = serde_json::to_string_pretty(&processed) {
                    let _ = fs::write(&json_path, serialized);
                    let _ = fs::write(&ver_path, processed.version.to_string());
                    crate::applog::event(&format!(
                        "features: synced features config v{} (fake_ranks={})",
                        processed.version, processed.flags.fake_ranks
                    ));
                }
                *CACHED_FEATURES.lock().unwrap() = Some(processed.clone());
                return Ok(processed);
            }
        }
        Ok(resp) => {
            crate::applog::event(&format!("features: remote returned HTTP {}", resp.status()));
        }
        Err(e) => {
            crate::applog::event(&format!("features: request error (using cached): {e}"));
        }
    }

    if let Some(cfg) = local_config {
        let processed = process_features(cfg);
        *CACHED_FEATURES.lock().unwrap() = Some(processed.clone());
        return Ok(processed);
    }

    if let Some(cached) = CACHED_FEATURES.lock().unwrap().clone() {
        let processed = process_features(cached);
        return Ok(processed);
    }

    let def = process_features(FeaturesConfig::default());
    Ok(def)
}

pub fn get_cached_features() -> FeaturesConfig {
    CACHED_FEATURES
        .lock()
        .unwrap()
        .clone()
        .map(process_features)
        .unwrap_or_else(|| process_features(FeaturesConfig::default()))
}
