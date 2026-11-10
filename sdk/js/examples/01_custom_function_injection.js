/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

import { rlim, BytecodeBuilder, FunctionFlags } from "../index.js";

console.log("[Plugin] Initializing Custom Stance Function Injector...");

// 1. Build the bytecode for our new custom function:
// simulated function RLIM_ApplyCustomStance()
// {
//     FrontWheelRoll = -10.5;
//     RearWheelRoll = -8.0;
//     return;
// }
const customFuncBytecode = new BytecodeBuilder()
    // FrontWheelRoll = -10.5
    .let(
        b => b.instanceVar(0x00000000), // Front wheel rotation variable index
        b => b.constFloat(-10.5)
    )
    // RearWheelRoll = -8.0
    .let(
        b => b.instanceVar(0x00000004), // Rear wheel rotation variable index
        b => b.constFloat(-8.0)
    )
    // return;
    .return()
    .padToSize(48);

// 2. Inject the new function export directly into TAGame.upk
const injected = rlim.patcher.createFunction(
    "TAGame.upk",
    "CarMeshComponent_TA",
    "RLIM_ApplyCustomStance",
    {
        flags: FunctionFlags.FUNC_Public | FunctionFlags.FUNC_Simulated,
        bytecode: customFuncBytecode
    }
);

if (injected) {
    console.log("[Plugin] Custom function RLIM_ApplyCustomStance successfully created in TAGame.upk!");

    // 3. Now patch CarMeshComponent_TA::InitWheelControls to invoke our new function!
    const hookBytecode = new BytecodeBuilder()
        // Call our new custom function
        .call(0x0000ABCD) // Index of our newly registered function
        .return(b => b.constBool(true))
        .padToSize(120);

    rlim.patcher.patchFunction(
        "TAGame.upk",
        "CarMeshComponent_TA",
        "InitWheelControls",
        hookBytecode
    );

    console.log("[Plugin] Patched InitWheelControls to call RLIM_ApplyCustomStance.");
} else {
    console.error("[Plugin] Failed to create custom function.");
}
