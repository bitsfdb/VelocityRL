/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

#include "../../include/rlim/rlim.h"
#include <stdio.h>
#include <string.h>
#include <math.h>

#define RLIM_BUILD_DLL

static const RLIM_HostInterface* g_Host = NULL;
static bool g_ReactiveUnderglow = true;
static float g_CamberAngle = -8.5f;

/* ── Plugin Metadata ──────────────────────────────────────────────────────── */

static const RLIM_PluginInfo g_PluginInfo = {
    "Dynamic Stance & Underglow",
    "1.0.0",
    "VelocityRL Team",
    "Reactive vehicle stance and underglow lighting synchronized via RLIM-Packet-v1",
    RLIM_API_VERSION
};

RLIM_EXPORT const RLIM_PluginInfo* RLIM_GetInfo(void)
{
    return &g_PluginInfo;
}

/* ── Event Callbacks ──────────────────────────────────────────────────────── */

static void OnFlipResetGained(RLIM_EventType type, const void* data, void* user_data)
{
    const RLIM_EventFlipResetGained* ev = (const RLIM_EventFlipResetGained*)data;
    if (!g_Host) return;

    char msg[128];
    snprintf(msg, sizeof(msg), "[RLIM-Stance] Flip reset detected! (Count: %d) Triggering flash.", ev->total_resets_in_air);
    g_Host->Log(0, msg);

    if (g_ReactiveUnderglow) {
        /* Pulse underglow to hyper-bright cyan upon gaining flip reset */
        RLIM_UnderglowSettings pulse = {0};
        pulse.enabled = true;
        pulse.mode = RLIM_LIGHTING_PULSE;
        pulse.primary_color = (RLIM_ColorRGBA){ 0.0f, 1.0f, 1.0f, 1.0f };   /* Neon Cyan */
        pulse.secondary_color = (RLIM_ColorRGBA){ 0.0f, 0.2f, 0.8f, 1.0f };
        pulse.brightness = 4.0f;                                              /* Flash brightness */
        pulse.animation_speed = 3.5f;

        g_Host->SetUnderglow(&pulse);
    }
}

static void OnMatchStarted(RLIM_EventType type, const void* data, void* user_data)
{
    const RLIM_EventMatchStarted* ev = (const RLIM_EventMatchStarted*)data;
    if (!g_Host) return;

    char msg[128];
    snprintf(msg, sizeof(msg), "[RLIM-Stance] Match started: %s (Ranked: %d)", ev->playlist_name, ev->is_ranked);
    g_Host->Log(0, msg);

    /* Read stance setting configured in VelocityRL UI */
    float camber_front = g_Host->GetConfigFloat(RLIM_CFG_CAMBER_FRONT, g_CamberAngle);
    float camber_rear  = g_Host->GetConfigFloat(RLIM_CFG_CAMBER_REAR,  g_CamberAngle * 0.75f);

    /* Apply vehicle camber */
    RLIM_CamberSettings stance = {0};
    stance.enabled = true;
    stance.front_camber_deg = camber_front;
    stance.rear_camber_deg = camber_rear;
    stance.front_track_width = 2.0f;
    stance.rear_track_width = 2.0f;
    stance.suspension_height = -1.5f; /* Lowered stance */

    g_Host->SetCamber(&stance);

    /* Apply baseline underglow */
    RLIM_UnderglowSettings underglow = {0};
    underglow.enabled = true;
    underglow.mode = RLIM_LIGHTING_RAINBOW;
    underglow.brightness = 2.5f;
    underglow.animation_speed = 1.2f;

    g_Host->SetUnderglow(&underglow);

    /* Broadcast configuration over wire protocol so peer VelocityRL clients see our car */
    RLIM_WirePayload wire = {0};
    strncpy(wire.sender_name, "LocalPlayer", sizeof(wire.sender_name) - 1);
    wire.camber = stance;
    wire.underglow = underglow;

    g_Host->BroadcastWirePacket(&wire);
}

/* ── Plugin Lifecycle ─────────────────────────────────────────────────────── */

RLIM_EXPORT bool RLIM_Init(const RLIM_HostInterface* host)
{
    if (!host) return false;
    if (host->api_version != RLIM_API_VERSION) {
        return false; /* ABI incompatibility */
    }

    g_Host = host;
    g_Host->Log(0, "[RLIM-Stance] Initializing Dynamic Stance & Underglow Plugin v1.0.0");

    /* Read initial configuration from hyphenated keys */
    g_ReactiveUnderglow = g_Host->GetConfigBool(RLIM_CFG_UNDERGLOW_ENABLED, true);
    g_CamberAngle = g_Host->GetConfigFloat(RLIM_CFG_CAMBER_FRONT, -8.5f);

    /* Subscribe to game events */
    g_Host->SubscribeEvent(RLIM_EVENT_MATCH_STARTED, OnMatchStarted, NULL);
    g_Host->SubscribeEvent(RLIM_EVENT_FLIP_RESET_GAINED, OnFlipResetGained, NULL);

    g_Host->Log(0, "[RLIM-Stance] Plugin initialized successfully.");
    return true;
}

RLIM_EXPORT void RLIM_Shutdown(void)
{
    if (!g_Host) return;

    g_Host->Log(0, "[RLIM-Stance] Shutting down plugin.");
    g_Host->UnsubscribeEvent(RLIM_EVENT_MATCH_STARTED, OnMatchStarted);
    g_Host->UnsubscribeEvent(RLIM_EVENT_FLIP_RESET_GAINED, OnFlipResetGained);

    g_Host = NULL;
}
