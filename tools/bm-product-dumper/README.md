# VelocityRL Product Dumper — BakkesMod Plugin

BakkesMod plugin that dumps **all Rocket League products** from the game's internal database with:
- **Slot** (Body, Wheels, Decal, etc.)
- **Quality** (Common, Uncommon, Import, etc.)
- **UnlockMethod** (Default, Online, DLC, Special)
- **Paintable** (yes/no from the game engine)
- **Paints** array — which paint colors each item can have (IDs 1–13)

The output JSON matches VelocityRL's `items.json` schema so you can upload it directly to `api.velocityrl.tech/items.json`.

## Why

The current `items.json` has **no `Paintable` field**. Items like TW Octane fail because the swap engine can't find a painted UPK on disk — Rocket League stores paint as an inventory attribute, not a separate file. This plugin extracts the ground truth directly from the running game.

## Output Format

Dumps to your **Downloads** folder. See [`example_items.json`](example_items.json) for the full format.

```json
{
  "Items": [
    {
      "ID": 23,
      "Product": "Octane",
      "Slot": "Body",
      "Quality": "Common",
      "UnlockMethod": "UnlockMethod_Default",
      "Paintable": true,
      "Paints": [
        { "id": 1, "label": "Crimson" },
        { "id": 12, "label": "Titanium White" }
      ],
      "AssetPackage": "Body_Octane_SF.upk",
      "AssetPath": "Body_Octane.Body_Octane"
    }
  ]
}
```

**Key fields the plugin adds** (vs the old items.json):
| Field | Type | Description |
|-------|------|-------------|
| `Paintable` | `boolean` | `true` if the game engine says this item supports paint |
| `Paints` | `array` | List of `{ id, label }` for each paint color (only present when Paintable=true) |
| `UnlockMethod` | `string` | How the item is obtained (Default, Online, DLC, Special) |

## Build

### Prerequisites
- **Visual Studio 2019/2022** (C++ desktop workload)
- **BakkesMod** installed (`%appdata%\bakkesmod\bakkesmod\bakkesmodsdk`)

### Steps
1. Create a new VS DLL project (x86, Release) or use the [BakkesMod Plugin Template](https://github.com/Martinii89/BakkesmodPluginTemplate)
2. Configure:
   - **Include Dirs**: `$(APPDATA)\bakkesmod\bakkesmod\bakkesmodsdk\include`
   - **Lib Dirs**: `$(APPDATA)\bakkesmod\bakkesmod\bakkesmodsdk\lib`
   - **Dependencies**: `pluginsdk.lib`
   - **Precompiled Header**: `pch.h`
   - **Standard**: C++17
   - **Platform**: x86
3. Add the source files (`pch.h`, `pch.cpp`, `VelocityProductDumper.h`, `VelocityProductDumper.cpp`)
4. Build → `VelocityProductDumper.dll`
5. Copy DLL to `%appdata%\bakkesmod\bakkesmod\plugins\`

## Usage

1. Launch Rocket League with BakkesMod
2. Open BakkesMod console (F6)
3. Run:

```
velocity_dump_all          → Downloads/velocity_products_all.json     (ALL items)
velocity_dump_paintable    → Downloads/velocity_products_paintable.json (paintable only)
```

4. Upload the JSON to your API

## Paint ID Reference

| ID | Color | ID | Color |
|----|-------|----|-------|
| 0 | None | 7 | Saffron |
| 1 | Crimson | 8 | Grey |
| 2 | Lime | 9 | Pink |
| 3 | Black | 10 | Forest Green |
| 4 | Orange | 11 | Purple |
| 5 | Sky Blue | 12 | Titanium White |
| 6 | Cobalt | 13 | Burnt Sienna |
