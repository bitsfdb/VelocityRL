/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

#ifndef RLIM_TYPES_H
#define RLIM_TYPES_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Base Primitives ──────────────────────────────────────────────────────── */

typedef int32_t     RLIM_Int;
typedef uint32_t    RLIM_UInt;
typedef float       RLIM_Float;
typedef bool        RLIM_Bool;
typedef uint64_t    RLIM_Handle;

typedef struct RLIM_ColorRGBA {
    float r;
    float g;
    float b;
    float a;
} RLIM_ColorRGBA;

typedef struct RLIM_Vector3 {
    float x;
    float y;
    float z;
} RLIM_Vector3;

typedef struct RLIM_Rotator {
    int32_t pitch;
    int32_t yaw;
    int32_t roll;
} RLIM_Rotator;

/* ── Product & Loadout Types ──────────────────────────────────────────────── */

typedef enum RLIM_Slot {
    RLIM_SLOT_BODY           = 0,
    RLIM_SLOT_SKIN           = 1,
    RLIM_SLOT_WHEELS         = 2,
    RLIM_SLOT_BOOST          = 3,
    RLIM_SLOT_ANTENNA        = 4,
    RLIM_SLOT_TOPPER         = 5,
    RLIM_SLOT_PAINT_FINISH   = 6,
    RLIM_SLOT_UNDERGLOW      = 7,
    RLIM_SLOT_HEADLIGHTS     = 8,
    RLIM_SLOT_TAILLIGHTS     = 9,
    RLIM_SLOT_CAMBER         = 10,
    RLIM_SLOT_ENGINE_AUDIO   = 11,
    RLIM_SLOT_TRAIL          = 12,
    RLIM_SLOT_GOAL_EXPLOSION = 13,
    RLIM_SLOT_BANNER         = 14,
    RLIM_SLOT_AVATAR_BORDER  = 15,
    RLIM_SLOT_MAX            = 16
} RLIM_Slot;

typedef enum RLIM_Paint {
    RLIM_PAINT_NONE           = 0,
    RLIM_PAINT_CRIMSON        = 1,
    RLIM_PAINT_LIME           = 2,
    RLIM_PAINT_BLACK          = 3,
    RLIM_PAINT_COBALT         = 4,
    RLIM_PAINT_TITANIUM_WHITE = 5,
    RLIM_PAINT_SIENNA         = 6,
    RLIM_PAINT_GREY           = 7,
    RLIM_PAINT_ORANGE         = 8,
    RLIM_PAINT_PURPLE         = 9,
    RLIM_PAINT_SAFFRON        = 10,
    RLIM_PAINT_SKY_BLUE       = 11,
    RLIM_PAINT_FOREST_GREEN   = 12,
    RLIM_PAINT_PINK           = 13,
    RLIM_PAINT_GOLD           = 14,
    RLIM_PAINT_ROSE_GOLD      = 15
} RLIM_Paint;

typedef struct RLIM_SlotItem {
    int32_t product_id;
    int32_t paint_id;
    int32_t custom_color_enabled;
    RLIM_ColorRGBA custom_color;
} RLIM_SlotItem;

typedef struct RLIM_Loadout {
    RLIM_SlotItem slots[RLIM_SLOT_MAX];
    char custom_title[64];
    RLIM_ColorRGBA title_color;
    bool title_glowing;
} RLIM_Loadout;

/* ── Stance / Camber Types ────────────────────────────────────────────────── */

typedef struct RLIM_CamberSettings {
    bool enabled;
    float front_camber_deg;   /* Range: -25.0 to +25.0 degrees */
    float rear_camber_deg;    /* Range: -25.0 to +25.0 degrees */
    float front_track_width;  /* Offset in unreal units */
    float rear_track_width;   /* Offset in unreal units */
    float suspension_height;  /* Ride height offset */
} RLIM_CamberSettings;

/* ── Underglow & Lighting Types ───────────────────────────────────────────── */

typedef enum RLIM_LightingMode {
    RLIM_LIGHTING_STATIC  = 0,
    RLIM_LIGHTING_RAINBOW = 1,
    RLIM_LIGHTING_PULSE   = 2,
    RLIM_LIGHTING_STROBE  = 3,
    RLIM_LIGHTING_BREATHE = 4
} RLIM_LightingMode;

typedef struct RLIM_UnderglowSettings {
    bool enabled;
    RLIM_LightingMode mode;
    RLIM_ColorRGBA primary_color;
    RLIM_ColorRGBA secondary_color;
    float brightness;         /* 0.0 to 5.0 (HDR emission multiplier) */
    float animation_speed;    /* Speed multiplier for rainbow/pulse/strobe */
} RLIM_UnderglowSettings;

typedef struct RLIM_LightingSettings {
    bool headlights_enabled;
    RLIM_ColorRGBA headlights_color;
    float headlights_brightness;

    bool taillights_enabled;
    RLIM_ColorRGBA taillights_color;
    float taillights_brightness;

    bool rim_lighting_enabled;
    RLIM_ColorRGBA rim_color;
    float rim_brightness;
} RLIM_LightingSettings;

/* ── In-Match HUD & Gameplay Metrics ──────────────────────────────────────── */

typedef struct RLIM_HUDMetrics {
    int32_t flip_resets_current;
    int32_t flip_resets_best;
    float   boost_amount;
    int32_t tracker_peak_mmr;
    int32_t current_playlist_id;
} RLIM_HUDMetrics;

/* ── Wire Protocol Data ───────────────────────────────────────────────────── */

#define RLIM_WIRE_PREFIX "RLIM-Packet-v1:"
#define RLIM_MAX_WIRE_PAYLOAD 512

typedef struct RLIM_WirePayload {
    char sender_name[32];
    uint64_t sender_player_id;
    RLIM_Loadout loadout;
    RLIM_CamberSettings camber;
    RLIM_UnderglowSettings underglow;
    uint32_t crc32;
} RLIM_WirePayload;

/* ── Plugin Metadata ──────────────────────────────────────────────────────── */

typedef struct RLIM_PluginInfo {
    const char* name;
    const char* version;
    const char* author;
    const char* description;
    uint32_t api_version;     /* Must match RLIM_API_VERSION */
} RLIM_PluginInfo;

#ifdef __cplusplus
}
#endif

#endif /* RLIM_TYPES_H */
