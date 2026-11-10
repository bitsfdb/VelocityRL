/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

import { rlim, BytecodeBuilder } from "../index.js";

console.log("[Plugin] Starting Low-Level Engine Patcher...");

// 1. Completely neutralize telemetry upload (BinaryUploader_TA::Upload => return true)
rlim.patcher.neutralize("TAGame.upk", "BinaryUploader_TA", "Upload", true);
console.log("[Plugin] BinaryUploader_TA::Upload neutralized.");

// 2. Patch PRI_TA::GetPing to always return 42ms
// simulated function int GetPing() { return 42; }
const pingBytecode = new BytecodeBuilder()
    .return(b => b.constInt(42))
    .padToSize(32);

rlim.patcher.patchFunction("TAGame.upk", "PRI_TA", "GetPing", pingBytecode);
console.log("[Plugin] PRI_TA::GetPing patched with spoofed 42ms ping.");

// 3. Apply neon cyan underglow with strobe mode
rlim.shaders.setUnderglow([0.0, 1.0, 1.0], 3.5, 3); // Cyan, 3.5 brightness, strobe

// 4. Set vehicle camber
rlim.shaders.setCamber(-9.0, -7.5);
console.log("[Plugin] Shaders and stance configured.");
