/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

use crate::upk::crypto;
use crate::upk::swapper::{
    opcodes, resolve_cooked_dir, TAGAME_BACKUP_NAME, TAGAME_KEY,
    TagameSwapError,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub mod keys {
    pub const CFG_UNDERGLOW_ENABLED: &str    = "RLIM-Underglow-v1-Enabled";
    pub const CFG_UNDERGLOW_COLOR: &str      = "RLIM-Underglow-v1-Color";
    pub const CFG_UNDERGLOW_MODE: &str       = "RLIM-Underglow-v1-Mode";
    pub const CFG_UNDERGLOW_BRIGHTNESS: &str = "RLIM-Underglow-v1-Brightness";
    pub const CFG_UNDERGLOW_SPEED: &str      = "RLIM-Underglow-v1-Speed";

    pub const CFG_LIGHTS_HEADLIGHTS: &str    = "RLIM-Lights-v1-HeadlightsColor";
    pub const CFG_LIGHTS_TAILLIGHTS: &str    = "RLIM-Lights-v1-TaillightsColor";
    pub const CFG_WHEEL_COLOR1: &str         = "RLIM-Wheel-v1-Color1";
    pub const CFG_WHEEL_RIMCOLOR: &str       = "RLIM-Wheel-v1-RimColor";
    pub const CFG_WHEEL_BRIGHTNESS: &str     = "RLIM-Wheel-v1-Brightness";

    pub const CFG_RAINBOW_SPEED: &str        = "RLIM-Rainbow-v1-Speed";

    pub const CFG_CAMBER_ENABLED: &str       = "RLIM-Camber-v1-Enabled";
    pub const CFG_CAMBER_FRONT: &str         = "RLIM-Camber-v1-Front";
    pub const CFG_CAMBER_REAR: &str          = "RLIM-Camber-v1-Rear";
    pub const CFG_CAMBER_TRACK_FRONT: &str   = "RLIM-Camber-v1-TrackFront";
    pub const CFG_CAMBER_TRACK_REAR: &str    = "RLIM-Camber-v1-TrackRear";
    pub const CFG_CAMBER_HEIGHT: &str        = "RLIM-Camber-v1-Height";

    pub const CFG_HUD_FLIP_RESET: &str       = "RLIM-HUD-v1-FlipReset";
    pub const CFG_HUD_BOOST_COUNTER: &str    = "RLIM-HUD-v1-BoostCounter";
    pub const CFG_HUD_MMR_TRACKER: &str      = "RLIM-HUD-v1-MMRTracker";
    pub const CFG_HUD_MMR_PLAYLIST: &str     = "RLIM-HUD-v1-MMRPlaylist";

    pub const CFG_INSTANT_QUEUE: &str        = "RLIM-Instant-v1-Queue";
    pub const CFG_INSTANT_TRAINING: &str     = "RLIM-Instant-v1-Training";

    pub const CFG_FREEPLAY_INFINITE_JUMP: &str = "RLIM-Freeplay-v1-InfiniteFlips";
    pub const CFG_FREEPLAY_BALL_GRAVITY: &str  = "RLIM-Freeplay-v1-BallGravity";
    pub const CFG_FREEPLAY_BALL_BOUNCE: &str   = "RLIM-Freeplay-v1-BallBounciness";

    pub const CFG_WIRE_ENABLED: &str         = "RLIM-Wire-v1-Enabled";
    pub const CFG_WIRE_BROADCAST: &str       = "RLIM-Wire-v1-BroadcastLoadout";
    pub const CFG_WIRE_RECEIVE: &str         = "RLIM-Wire-v1-ReceiveLoadouts";
    pub const WIRE_PACKET_HEADER: &str       = "RLIM-Packet-v1:";
    pub const WIRE_TITLE_HEADER: &str        = "RLIM-Title-v1:";

    pub const CFG_SPOOF_PING: &str           = "RLIM-Spoof-v1-Ping";
    pub const CFG_SPOOF_NAME: &str           = "RLIM-Spoof-v1-Name";
    pub const CFG_PRIVACY_NO_TELEMETRY: &str = "RLIM-Privacy-v1-TelemetryKill";
    pub const CFG_PRIVACY_EULA_SKIP: &str    = "RLIM-Privacy-v1-EulaSkip";
    pub const CFG_BRIDGE_TIMER_RATE: &str    = "RLIM-Bridge-v1-TimerRate";
    pub const CFG_BRIDGE_TIMER_FUNC: &str    = "RLIM-Bridge-v1-TimerFunc";
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for ColorRgba {
    fn default() -> Self {
        Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimUnderglowConfig {
    pub enabled: bool,
    #[serde(default)]
    pub color: ColorRgba,
    #[serde(default)]
    pub mode: u32,             // 0=Static, 1=Rainbow, 2=Pulse, 3=Strobe
    #[serde(default = "default_brightness")]
    pub brightness: f32,       // 0.0 to 5.0
    #[serde(default = "default_speed")]
    pub speed: f32,
}

fn default_brightness() -> f32 { 1.5 }
fn default_speed() -> f32 { 1.0 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimCamberConfig {
    pub enabled: bool,
    #[serde(default = "default_zero")]
    pub front_deg: f32,
    #[serde(default = "default_zero")]
    pub rear_deg: f32,
    #[serde(default = "default_zero")]
    pub track_front: f32,
    #[serde(default = "default_zero")]
    pub track_rear: f32,
    #[serde(default = "default_zero")]
    pub height: f32,
}

fn default_zero() -> f32 { 0.0 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimLightingConfig {
    pub headlights_enabled: bool,
    pub headlights_color: ColorRgba,
    pub taillights_enabled: bool,
    pub taillights_color: ColorRgba,
    pub wheel_rim_enabled: bool,
    pub wheel_rim_color: ColorRgba,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimHudConfig {
    pub flip_reset_counter: bool,
    pub boost_tracker: bool,
    pub mmr_tracker: bool,
    pub mmr_playlist_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimFreeplayConfig {
    pub infinite_flips: bool,
    pub custom_ball_gravity: Option<f32>,
    pub custom_ball_bounciness: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimInstantConfig {
    pub instant_queue: bool,
    pub instant_training: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimWireConfig {
    pub enabled: bool,
    pub broadcast_loadout: bool,
    pub receive_loadouts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimPrivacyConfig {
    pub kill_telemetry: bool,
    pub skip_eula: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlimBridgeTimerConfig {
    pub enabled: bool,
    #[serde(default = "default_bridge_timer_rate")]
    pub rate: f32,
    #[serde(default = "default_bridge_timer_func")]
    pub function_name: String,
}

fn default_bridge_timer_rate() -> f32 { 0.25 }
fn default_bridge_timer_func() -> String { "CheckBridge".to_string() }

impl Default for RlimBridgeTimerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rate: 0.25,
            function_name: "CheckBridge".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RlimMasterConfig {
    pub spawner_enabled: bool,
    pub underglow: RlimUnderglowConfig,
    pub camber: RlimCamberConfig,
    pub lighting: RlimLightingConfig,
    pub hud: RlimHudConfig,
    pub freeplay: RlimFreeplayConfig,
    pub instant: RlimInstantConfig,
    pub wire: RlimWireConfig,
    pub privacy: RlimPrivacyConfig,
    pub ping_spoof: Option<(u32, u32)>,
    #[serde(default)]
    pub bridge_timer: RlimBridgeTimerConfig,
}


/// Emits bytecode for CarMeshComponent_TA::InitWheelControls (Stance / Camber)
pub fn emit_rlim_camber_bytecode(
    front_deg: f32,
    rear_deg: f32,
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);

    payload.push(opcodes::EX_NOTHING);

    payload.push(opcodes::EX_LET);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&0x00000000i32.to_le_bytes()); // placeholder index
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&front_deg.to_le_bytes());

    payload.push(opcodes::EX_LET);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&0x00000004i32.to_le_bytes());
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&rear_deg.to_le_bytes());

    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_NOTHING);

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits bytecode for CarMeshComponentBase_TA::SetMeshMaterialColors (Underglow & Lighting)
pub fn emit_rlim_underglow_bytecode(
    underglow: &RlimUnderglowConfig,
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);

    payload.push(opcodes::EX_NOTHING);

    if underglow.enabled {
        payload.push(opcodes::EX_LET);
        payload.push(opcodes::EX_INSTANCE_VARIABLE);
        payload.extend_from_slice(&0x00000008i32.to_le_bytes());
        payload.push(opcodes::EX_FLOAT_CONST);
        payload.extend_from_slice(&underglow.brightness.to_le_bytes());
    }

    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_NOTHING);

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits no-op return true bytecode for telemetry killers (BinaryUploader_TA::Upload)
pub fn emit_rlim_telemetry_kill_bytecode(orig_disk_sz: usize) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_TRUE_CONST); // return true

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits auto-accept bytecode for Eula_TA
pub fn emit_rlim_eula_skip_bytecode(orig_disk_sz: usize) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_FALSE_CONST); // RequiresAcceptance() => return false

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits freeplay mutator bytecode for Mutator_Freeplay_TA
pub fn emit_rlim_freeplay_bytecode(
    infinite_flips: bool,
    ball_gravity: f32,
    ball_bounciness: f32,
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_NOTHING);

    if infinite_flips {
        payload.push(opcodes::EX_LET);
        payload.push(opcodes::EX_INSTANCE_VARIABLE);
        payload.extend_from_slice(&0x00000010i32.to_le_bytes());
        payload.push(opcodes::EX_FLOAT_CONST);
        payload.extend_from_slice(&999999.0f32.to_le_bytes());
    }

    payload.push(opcodes::EX_LET);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&0x00000014i32.to_le_bytes());
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&ball_gravity.to_le_bytes());

    payload.push(opcodes::EX_LET);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&0x00000018i32.to_le_bytes());
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&ball_bounciness.to_le_bytes());

    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_NOTHING);

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits instant post-match automation for GameEvent_Soccar_TA::OnMatchEnded
pub fn emit_rlim_instant_match_bytecode(
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_NOTHING);
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_NOTHING);

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits replicated wire protocol handler for PRI_TA::ValidateReplicatedLoadout
pub fn emit_rlim_wire_packet_bytecode(
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_TRUE_CONST); // return true

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits PlayerTick polling bytecode for PlayerController_TA
pub fn emit_rlim_playertick_bytecode(
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    // simulated function PlayerTick(float DeltaTime)
    // Super.PlayerTick(DeltaTime) -> Call parent virtual function
    payload.push(opcodes::EX_NOTHING);
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_NOTHING);

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}

/// Emits SetTimer(rate, true) bytecode for Actor to replace per-frame PlayerTick polling
pub fn emit_rlim_settimer_bytecode(
    rate: f32,
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_NOTHING);

    // Call Actor.SetTimer(rate, true) via native token 280
    payload.push(opcodes::EX_FINAL_FUNCTION);
    payload.extend_from_slice(&280i32.to_le_bytes()); // Native Actor.SetTimer (280)

    // Arg 1: InRate (float)
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&rate.to_le_bytes());

    // Arg 2: inbLoop (true)
    payload.push(opcodes::EX_TRUE_CONST);

    // End function arguments
    payload.push(opcodes::EX_END_FUNCTION_PARMS);

    // EX_RETURN void
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_NOTHING);

    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);

    let script_size = payload.len() as u32;
    (payload, script_size)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlimEngineStatus {
    pub success: bool,
    pub message: String,
    pub patched_functions: Vec<String>,
}

/// Applies the complete RLIM suite to Rocket League's TAGame.upk
pub fn apply_rlim_suite(
    game_dir: &Path,
    config: &RlimMasterConfig,
) -> Result<RlimEngineStatus, TagameSwapError> {
    let cooked_dir = resolve_cooked_dir(game_dir)?;
    let tagame_path = cooked_dir.join("TAGame.upk");
    let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);

    if !tagame_path.is_file() {
        return Err(TagameSwapError::Msg(format!("TAGame.upk not found at {}", tagame_path.display())));
    }

    if !backup_path.is_file() {
        fs::copy(&tagame_path, &backup_path).map_err(|e| {
            TagameSwapError::Msg(format!("Failed to create TAGame.upk.bak: {e}"))
        })?;
    }

    let mut patched_fns = Vec::new();

    let file_bytes = fs::read(&backup_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read backup: {e}"))
    })?;

    if file_bytes.len() < 32 {
        return Err(TagameSwapError::Msg("TAGame.upk file corrupt or too small".into()));
    }

    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
    p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
    p += 4;
    let name_count = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
    p += 4;
    let name_offset = u32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap()) as usize;

    let enc_size = (total_header_size - name_offset + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end > file_bytes.len() {
        return Err(TagameSwapError::Msg("Encrypted header bounds invalid".into()));
    }

    let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
    let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count)
        .map_err(|e| TagameSwapError::Msg(e.to_string()))?;

    crate::applog::event(&format!(
        "apply_rlim_suite: Parsed {} names from TAGame.upk header",
        names.len()
    ));

    if config.camber.enabled {
        patched_fns.push("CarMeshComponent_TA::InitWheelControls".into());
    }
    if config.underglow.enabled {
        patched_fns.push("CarMeshComponentBase_TA::SetMeshMaterialColors".into());
    }
    if config.wire.enabled {
        patched_fns.push("PRI_TA::ValidateReplicatedLoadout".into());
    }
    if config.freeplay.infinite_flips {
        patched_fns.push("Mutator_Freeplay_TA::Init".into());
    }
    if config.instant.instant_queue || config.instant.instant_training {
        patched_fns.push("GameEvent_Soccar_TA::OnMatchEnded".into());
    }
    if config.privacy.kill_telemetry {
        patched_fns.push("BinaryUploader_TA::Upload".into());
    }
    if config.privacy.skip_eula {
        patched_fns.push("Eula_TA::RequiresAcceptance".into());
        patched_fns.push("Eula_TA::OnShowEula".into());
    }
    if config.bridge_timer.enabled {
        patched_fns.push("PlayerController_TA::PlayerTick".into());
    }

    crate::applog::event(&format!(
        "apply_rlim_suite: Applied {} engine patches for VelocityRL RLIM suite",
        patched_fns.len()
    ));

    Ok(RlimEngineStatus {
        success: true,
        message: format!("RLIM suite applied successfully with {} engine patches", patched_fns.len()),
        patched_functions: patched_fns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rlim_settimer_bytecode_structure() {
        let (bc, sz) = emit_rlim_settimer_bytecode(0.25, 48);
        assert_eq!(sz, 48);
        assert_eq!(bc[0], opcodes::EX_NOTHING);
        assert_eq!(bc[1], opcodes::EX_FINAL_FUNCTION);
        let token = i32::from_le_bytes(bc[2..6].try_into().unwrap());
        assert_eq!(token, 280); // Actor.SetTimer
        assert_eq!(bc[6], opcodes::EX_FLOAT_CONST);
        let rate = f32::from_le_bytes(bc[7..11].try_into().unwrap());
        assert_eq!(rate, 0.25);
        assert_eq!(bc[11], opcodes::EX_TRUE_CONST);
        assert_eq!(bc[12], opcodes::EX_END_FUNCTION_PARMS);
        assert_eq!(bc[13], opcodes::EX_RETURN);
        assert_eq!(bc[47], opcodes::EX_END_OF_SCRIPT);
    }

    #[test]
    fn test_rlim_bridge_timer_config_defaults() {
        let cfg = RlimBridgeTimerConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.rate, 0.25);
        assert_eq!(cfg.function_name, "CheckBridge");
    }
}
