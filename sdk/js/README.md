# VelocityRL — RLIM JavaScript Plugin SDK (`rquickjs`)

The **RLIM JavaScript SDK** allows developers to write plugins in standard JavaScript (ES2020) that execute inside an isolated **QuickJS (`rquickjs`)** virtual machine in the Rust backend.

User scripts do **not** run inside the Tauri WebView DOM. If a script throws a syntax error, creates an infinite loop (`while(true){}`), or leaks memory, the QuickJS runtime enforces:
* **Max Memory Limit:** 64 MB (configurable)
* **Call Stack Limit:** 1 MB
* **Interrupt Timeout:** 5.0 seconds maximum execution

---

## 1. Core Capabilities

Unlike conventional modding frameworks that only allow hooking pre-existing events, RLIM exposes low-level Unreal Engine 3 package manipulation:

1. **Function Patching (`rlim.patcher.patchFunction`)**:
   Directly rewrite the UnrealScript bytecode of any existing function in `TAGame.upk`, `ProjectX.upk`, or `Engine.upk`.
2. **Custom Function Creation (`rlim.patcher.createFunction`)**:
   Inject brand-new UnrealScript functions into package export tables and call them from patched game functions.
3. **Function Neutralization (`rlim.patcher.neutralize`)**:
   Stub out telemetry, DRM, EULA checks, or validation routines with a single `return` opcode.
4. **Fluent Bytecode Builder (`BytecodeBuilder`)**:
   Construct UnrealScript AST bytecodes in JavaScript using chainable methods (`.let()`, `.instanceVar()`, `.constFloat()`, `.call()`, `.return()`).
5. **Shader & Stance Controls (`rlim.shaders`)**:
   Modify underglow, wheel rim lights, and camber angles.
6. **Cosmetic Spawning (`rlim.spawner`)**:
   Inject items and decals into the player's local loadout.

---

## 2. Quick Example: Creating a Custom Function

```javascript
import { rlim, BytecodeBuilder, FunctionFlags } from "@velocityrl/rlim";

// 1. Define custom function bytecode
const customCode = new BytecodeBuilder()
    .let(b => b.instanceVar(0x00), b => b.constFloat(-12.0))
    .return()
    .padToSize(48);

// 2. Create the function export on CarMeshComponent_TA
rlim.patcher.createFunction(
    "TAGame.upk",
    "CarMeshComponent_TA",
    "RLIM_CustomStance",
    {
        flags: FunctionFlags.FUNC_Public | FunctionFlags.FUNC_Simulated,
        bytecode: customCode
    }
);
```

---

## 3. Directory Layout

Place your `.js` plugins into VelocityRL's `plugins/` directory:
```
VelocityRL/
├── plugins/
│   ├── custom_stance.js
│   └── reactive_underglow.js
└── ...
```
VelocityRL loads and evaluates them inside the backend QuickJS runtime upon game launch or patch application.
