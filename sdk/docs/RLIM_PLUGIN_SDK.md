# RLIM Plugin SDK for VelocityRL

Welcome to the **RLIM Plugin SDK** — the native C/C++ extension system for **VelocityRL** and Rocket League.

RLIM (Rocket League Injection & Modding) allows developers to write native DLL plugins that interact directly with the game engine, hook gameplay events, modify vehicle stance (camber), control dynamic RGB shaders (underglow, rim lights, rainbow paint), spawn items client-side, and synchronize custom visuals across peer-to-peer matches via the RLIM Wire Protocol.

---

## 1. The `RLIM-xx-xx-xx` Namespace Standard

All configuration keys, shader variables, and wire protocol headers in RLIM strictly follow a hyphenated hierarchy:

```
RLIM-<Category>-<Version>-<Property>
```

### Reference Table

| Category | Key | Type | Description |
|---|---|---|---|
| **Underglow** | `RLIM-Underglow-v1-Enabled` | `bool` | Enable/disable car underglow |
| | `RLIM-Underglow-v1-Color` | `ColorRGBA` | Primary underglow neon color |
| | `RLIM-Underglow-v1-Mode` | `int` | 0=Static, 1=Rainbow, 2=Pulse, 3=Strobe |
| | `RLIM-Underglow-v1-Brightness` | `float` | HDR intensity multiplier (0.0 – 5.0) |
| | `RLIM-Underglow-v1-Speed` | `float` | Animation frequency speed |
| **Lighting** | `RLIM-Lights-v1-HeadlightsColor` | `ColorRGBA` | Headlamp beam & projector color |
| | `RLIM-Lights-v1-TaillightsColor` | `ColorRGBA` | Rear tail and brake light color |
| | `RLIM-Wheel-v1-Color1` | `ColorRGBA` | Tire tread & rim illumination |
| | `RLIM-Wheel-v1-RimColor` | `ColorRGBA` | Metallic rim alloy tint |
| **Camber / Stance** | `RLIM-Camber-v1-Enabled` | `bool` | Enable wheel suspension camber |
| | `RLIM-Camber-v1-Front` | `float` | Front wheel tilt in degrees (-25° to +25°) |
| | `RLIM-Camber-v1-Rear` | `float` | Rear wheel tilt in degrees (-25° to +25°) |
| | `RLIM-Camber-v1-TrackFront` | `float` | Front axle spacing offset |
| | `RLIM-Camber-v1-TrackRear` | `float` | Rear axle spacing offset |
| | `RLIM-Camber-v1-Height` | `float` | Ride height offset |
| **HUD & Metrics** | `RLIM-HUD-v1-FlipReset` | `bool` | On-screen airborne flip reset counter |
| | `RLIM-HUD-v1-BoostCounter` | `bool` | Accurate boost consumption tracker |
| | `RLIM-HUD-v1-MMRTracker` | `bool` | Live tracker.gg peak MMR next to name |
| **Automation** | `RLIM-Instant-v1-Queue` | `bool` | Instantly queue match on game end |
| | `RLIM-Instant-v1-Training` | `bool` | Instantly launch freeplay on game end |
| **Freeplay** | `RLIM-Freeplay-v1-InfiniteFlips`| `bool` | Disables dodge timer limit |
| | `RLIM-Freeplay-v1-BallGravity` | `float` | Custom ball gravity multiplier |
| | `RLIM-Freeplay-v1-BallBounciness`| `float` | Custom ball restitution multiplier |
| **Wire Protocol** | `RLIM-Wire-v1-Enabled` | `bool` | P2P cosmetic & stance replication |
| | `RLIM-Packet-v1:<payload>` | `string` | Replicated loadout packet prefix |

---

## 2. Plugin Architecture

A plugin is a standard 64-bit Windows dynamic link library (`.dll`) placed into VelocityRL's `plugins/` directory.

### Required Exported Functions

```c
#include <rlim/rlim.h>

// 1. Expose plugin information to VelocityRL
RLIM_EXPORT const RLIM_PluginInfo* RLIM_GetInfo(void);

// 2. Initialize host interface and subscribe to events
RLIM_EXPORT bool RLIM_Init(const RLIM_HostInterface* host);

// 3. Clean up on unload
RLIM_EXPORT void RLIM_Shutdown(void);
```

---

## 3. Host Function Table

When VelocityRL loads your plugin, it passes a `const RLIM_HostInterface*` containing:

### Configuration API
* `host->GetConfigBool(key, default_val)`
* `host->GetConfigFloat(key, default_val)`
* `host->GetConfigString(key, out_buf, buf_size)`
* `host->SetConfigFloat(key, val)`
* `host->SetConfigColor(key, &color)`

### Event Hooks
* `host->SubscribeEvent(type, callback, user_data)`
* `host->UnsubscribeEvent(type, callback)`

Available events:
* `RLIM_EVENT_ENGINE_INIT`: Engine ready.
* `RLIM_EVENT_TICK`: In-game tick delta update.
* `RLIM_EVENT_MATCH_STARTED`: Local player entered a match or freeplay.
* `RLIM_EVENT_MATCH_ENDED`: Match ended with final scores.
* `RLIM_EVENT_CAR_SPAWNED`: Player car spawned onto the pitch.
* `RLIM_EVENT_FLIP_RESET_GAINED`: Vehicle four wheels made contact with surface/ball.
* `RLIM_EVENT_BALL_HIT`: Ball struck with impact force and direction.
* `RLIM_EVENT_WIRE_PACKET_RECEIVED`: Incoming P2P loadout from another RLIM user.

### Live In-Engine Modification API
* `host->SpawnItem(slot, product_id, paint_id)`: Spawn any body, decal, wheel, or explosion directly into the local player's active loadout.
* `host->SetCamber(&camber_settings)`: Apply live wheel stance.
* `host->SetUnderglow(&underglow_settings)`: Set RGB underglow mode and color.
* `host->SetLighting(&lighting_settings)`: Set headlights, taillights, and wheel rim lights.
* `host->SetCustomTitle("VELOCITY", &color, true)`: Display client-side and peer-replicated glowing player title.
* `host->BroadcastWirePacket(&payload)`: Broadcast custom visuals to other VelocityRL players in the match.

---

## 4. Building Plugins

Using CMake:

```cmake
cmake_minimum_required(VERSION 3.16)
project(my_rlim_plugin C)

include_directories("${VELOCITYRL_SDK_DIR}/include")

add_library(my_rlim_plugin SHARED
    main.c
)
```

Compile with MSVC or Clang:
```bash
mkdir build && cd build
cmake .. -A x64
cmake --build . --config Release
```

Copy `my_rlim_plugin.dll` to your VelocityRL `plugins/` directory.
