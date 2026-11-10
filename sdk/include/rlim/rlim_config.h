/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

#ifndef RLIM_CONFIG_H
#define RLIM_CONFIG_H

#ifdef __cplusplus
extern "C" {
#endif

/* ── Underglow Keys ───────────────────────────────────────────────────────── */
#define RLIM_CFG_UNDERGLOW_ENABLED     "RLIM-Underglow-v1-Enabled"
#define RLIM_CFG_UNDERGLOW_COLOR       "RLIM-Underglow-v1-Color"
#define RLIM_CFG_UNDERGLOW_MODE        "RLIM-Underglow-v1-Mode"
#define RLIM_CFG_UNDERGLOW_BRIGHTNESS  "RLIM-Underglow-v1-Brightness"
#define RLIM_CFG_UNDERGLOW_SPEED       "RLIM-Underglow-v1-Speed"

/* ── Lighting & Rim Keys ──────────────────────────────────────────────────── */
#define RLIM_CFG_LIGHTS_HEADLIGHTS     "RLIM-Lights-v1-HeadlightsColor"
#define RLIM_CFG_LIGHTS_TAILLIGHTS     "RLIM-Lights-v1-TaillightsColor"
#define RLIM_CFG_WHEEL_COLOR1          "RLIM-Wheel-v1-Color1"
#define RLIM_CFG_WHEEL_RIMCOLOR        "RLIM-Wheel-v1-RimColor"
#define RLIM_CFG_WHEEL_BRIGHTNESS      "RLIM-Wheel-v1-Brightness"

/* ── Rainbow Engine Keys ──────────────────────────────────────────────────── */
#define RLIM_CFG_RAINBOW_SPEED         "RLIM-Rainbow-v1-Speed"

/* ── Camber & Stance Keys ─────────────────────────────────────────────────── */
#define RLIM_CFG_CAMBER_ENABLED        "RLIM-Camber-v1-Enabled"
#define RLIM_CFG_CAMBER_FRONT          "RLIM-Camber-v1-Front"
#define RLIM_CFG_CAMBER_REAR           "RLIM-Camber-v1-Rear"
#define RLIM_CFG_CAMBER_TRACK_FRONT    "RLIM-Camber-v1-TrackFront"
#define RLIM_CFG_CAMBER_TRACK_REAR     "RLIM-Camber-v1-TrackRear"
#define RLIM_CFG_CAMBER_HEIGHT         "RLIM-Camber-v1-Height"

/* ── HUD & Metrics Keys ───────────────────────────────────────────────────── */
#define RLIM_CFG_HUD_FLIP_RESET        "RLIM-HUD-v1-FlipReset"
#define RLIM_CFG_HUD_BOOST_COUNTER     "RLIM-HUD-v1-BoostCounter"
#define RLIM_CFG_HUD_MMR_TRACKER       "RLIM-HUD-v1-MMRTracker"
#define RLIM_CFG_HUD_MMR_PLAYLIST      "RLIM-HUD-v1-MMRPlaylist"

/* ── Instant Automation Keys ──────────────────────────────────────────────── */
#define RLIM_CFG_INSTANT_QUEUE         "RLIM-Instant-v1-Queue"
#define RLIM_CFG_INSTANT_TRAINING      "RLIM-Instant-v1-Training"

/* ── Freeplay Mutator Keys ────────────────────────────────────────────────── */
#define RLIM_CFG_FREEPLAY_INFINITE_JUMP "RLIM-Freeplay-v1-InfiniteFlips"
#define RLIM_CFG_FREEPLAY_BALL_GRAVITY  "RLIM-Freeplay-v1-BallGravity"
#define RLIM_CFG_FREEPLAY_BALL_BOUNCE   "RLIM-Freeplay-v1-BallBounciness"
#define RLIM_CFG_FREEPLAY_UNLIMITED_BOOST "RLIM-Freeplay-v1-UnlimitedBoost"

/* ── Wire Sync & Replicated Protocol Keys ─────────────────────────────────── */
#define RLIM_CFG_WIRE_ENABLED          "RLIM-Wire-v1-Enabled"
#define RLIM_CFG_WIRE_BROADCAST        "RLIM-Wire-v1-BroadcastLoadout"
#define RLIM_CFG_WIRE_RECEIVE          "RLIM-Wire-v1-ReceiveLoadouts"
#define RLIM_WIRE_PACKET_HEADER        "RLIM-Packet-v1:"
#define RLIM_WIRE_TITLE_HEADER         "RLIM-Title-v1:"

/* ── Spoofing & Privacy Keys ──────────────────────────────────────────────── */
#define RLIM_CFG_SPOOF_PING            "RLIM-Spoof-v1-Ping"
#define RLIM_CFG_SPOOF_NAME            "RLIM-Spoof-v1-Name"
#define RLIM_CFG_PRIVACY_NO_TELEMETRY  "RLIM-Privacy-v1-TelemetryKill"
#define RLIM_CFG_PRIVACY_EULA_SKIP     "RLIM-Privacy-v1-EulaSkip"

#ifdef __cplusplus
}
#endif

#endif /* RLIM_CONFIG_H */
