/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

#ifndef RLIM_H
#define RLIM_H

#define RLIM_API_VERSION_MAJOR 1
#define RLIM_API_VERSION_MINOR 0
#define RLIM_API_VERSION_PATCH 0
#define RLIM_API_VERSION ((RLIM_API_VERSION_MAJOR << 16) | (RLIM_API_VERSION_MINOR << 8) | RLIM_API_VERSION_PATCH)

#if defined(_WIN32) || defined(__CYGWIN__)
    #ifdef RLIM_BUILD_DLL
        #define RLIM_EXPORT __declspec(dllexport)
    #else
        #define RLIM_EXPORT __declspec(dllimport)
    #endif
#else
    #define RLIM_EXPORT __attribute__((visibility("default")))
#endif

#include "rlim_types.h"
#include "rlim_config.h"
#include "rlim_events.h"
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Core RLIM Host Function Table ────────────────────────────────────────── */

typedef struct RLIM_HostInterface {
    /* ABI verification */
    uint32_t struct_size;
    uint32_t api_version;

    /* Logging */
    void (*Log)(int level, const char* message);

    /* Configuration Getters (strictly hyphenated RLIM-xx-xx-xx keys) */
    bool        (*GetConfigBool)(const char* key, bool default_val);
    int32_t     (*GetConfigInt)(const char* key, int32_t default_val);
    float       (*GetConfigFloat)(const char* key, float default_val);
    bool        (*GetConfigString)(const char* key, char* out_buf, size_t buf_size);
    bool        (*GetConfigColor)(const char* key, RLIM_ColorRGBA* out_color);

    /* Configuration Setters */
    bool        (*SetConfigBool)(const char* key, bool val);
    bool        (*SetConfigInt)(const char* key, int32_t val);
    bool        (*SetConfigFloat)(const char* key, float val);
    bool        (*SetConfigString)(const char* key, const char* val);
    bool        (*SetConfigColor)(const char* key, const RLIM_ColorRGBA* color);

    /* Event Subscriptions */
    bool        (*SubscribeEvent)(RLIM_EventType type, RLIM_EventCallback callback, void* user_data);
    bool        (*UnsubscribeEvent)(RLIM_EventType type, RLIM_EventCallback callback);

    /* Live In-Engine Spawning & Customization */
    bool        (*SpawnItem)(RLIM_Slot slot, int32_t product_id, int32_t paint_id);
    bool        (*ApplyCustomLoadout)(const RLIM_Loadout* loadout);
    bool        (*SetCamber)(const RLIM_CamberSettings* camber);
    bool        (*SetUnderglow)(const RLIM_UnderglowSettings* underglow);
    bool        (*SetLighting)(const RLIM_LightingSettings* lighting);
    bool        (*SetCustomTitle)(const char* title, const RLIM_ColorRGBA* color, bool glowing);

    /* Wire Protocol (P2P Mesh Sync) */
    bool        (*BroadcastWirePacket)(const RLIM_WirePayload* payload);

    /* Game State Inspection */
    bool        (*GetHUDMetrics)(RLIM_HUDMetrics* out_metrics);
    bool        (*GetLocalPlayerCar)(RLIM_Vector3* out_pos, RLIM_Rotator* out_rot, RLIM_Vector3* out_velocity);
    bool        (*GetBallState)(RLIM_Vector3* out_pos, RLIM_Vector3* out_velocity);

} RLIM_HostInterface;

/* ── Plugin Exported Function Prototypes ──────────────────────────────────── */

/**
 * Called by VelocityRL host when querying plugin metadata.
 */
RLIM_EXPORT const RLIM_PluginInfo* RLIM_GetInfo(void);

/**
 * Called when VelocityRL loads the plugin.
 * @param host Pointer to host API function table.
 * @return true if initialization succeeded, false to abort.
 */
RLIM_EXPORT bool RLIM_Init(const RLIM_HostInterface* host);

/**
 * Called when VelocityRL unloads the plugin or shuts down.
 */
RLIM_EXPORT void RLIM_Shutdown(void);

#ifdef __cplusplus
}
#endif

#endif /* RLIM_H */
