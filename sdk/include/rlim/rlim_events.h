/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

#ifndef RLIM_EVENTS_H
#define RLIM_EVENTS_H

#include "rlim_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ── Event Identifiers ────────────────────────────────────────────────────── */

typedef enum RLIM_EventType {
    RLIM_EVENT_ENGINE_INIT          = 0,
    RLIM_EVENT_TICK                 = 1,
    RLIM_EVENT_MATCH_STARTED        = 2,
    RLIM_EVENT_MATCH_ENDED          = 3,
    RLIM_EVENT_CAR_SPAWNED          = 4,
    RLIM_EVENT_CAR_DEMOLISHED       = 5,
    RLIM_EVENT_FLIP_RESET_GAINED    = 6,
    RLIM_EVENT_BALL_HIT             = 7,
    RLIM_EVENT_GOAL_SCORED          = 8,
    RLIM_EVENT_LOADOUT_APPLIED      = 9,
    RLIM_EVENT_WIRE_PACKET_RECEIVED = 10,
    RLIM_EVENT_CONFIG_CHANGED       = 11,
    RLIM_EVENT_MAX                  = 12
} RLIM_EventType;

/* ── Event Payload Data ───────────────────────────────────────────────────── */

typedef struct RLIM_EventMatchStarted {
    const char* playlist_name;
    int32_t playlist_id;
    bool is_ranked;
    bool is_freeplay;
} RLIM_EventMatchStarted;

typedef struct RLIM_EventMatchEnded {
    int32_t blue_score;
    int32_t orange_score;
    int32_t winner_team;      /* 0 = Blue, 1 = Orange */
} RLIM_EventMatchEnded;

typedef struct RLIM_EventCarSpawned {
    uint64_t player_id;
    const char* player_name;
    bool is_local_player;
    int32_t team_index;       /* 0 = Blue, 1 = Orange */
    RLIM_Vector3 spawn_location;
} RLIM_EventCarSpawned;

typedef struct RLIM_EventFlipResetGained {
    uint64_t player_id;
    int32_t total_resets_in_air;
} RLIM_EventFlipResetGained;

typedef struct RLIM_EventBallHit {
    uint64_t player_id;
    RLIM_Vector3 hit_location;
    RLIM_Vector3 ball_velocity_after;
    float hit_force;
} RLIM_EventBallHit;

typedef struct RLIM_EventWirePacketReceived {
    uint64_t sender_player_id;
    const char* sender_name;
    const RLIM_WirePayload* payload;
} RLIM_EventWirePacketReceived;

typedef struct RLIM_EventConfigChanged {
    const char* key;          /* Hyphenated key, e.g., RLIM-Underglow-v1-Color */
    const char* new_value;
} RLIM_EventConfigChanged;

/* ── Callback Function Signatures ─────────────────────────────────────────── */

typedef void (*RLIM_EventCallback)(RLIM_EventType type, const void* event_data, void* user_data);

#ifdef __cplusplus
}
#endif

#endif /* RLIM_EVENTS_H */
