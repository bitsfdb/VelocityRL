/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

import { Opcodes, FunctionFlags } from "./opcodes.js";
import { BytecodeBuilder } from "./builder.js";

const host = typeof globalThis.rlim !== "undefined" ? globalThis.rlim : {
    patcher: {
        patchFunction: () => false,
        createFunction: () => false,
        neutralizeFunction: () => false,
    },
    shaders: {
        setCamber: () => false,
        setUnderglow: () => false,
    },
    spawner: {
        spawnItem: () => false,
    },
    config: {
        get: (_k, def) => def,
        set: () => false,
    }
};

export const rlim = {
    /**
     * Low-level package and function patching
     */
    patcher: {
        /**
         * Patches bytecode of an existing engine function
         * @param {string} packageName e.g. "TAGame.upk"
         * @param {string} className e.g. "CarMeshComponent_TA"
         * @param {string} funcName e.g. "InitWheelControls"
         * @param {number[]|Uint8Array|BytecodeBuilder} bytecode
         */
        patchFunction(packageName, className, funcName, bytecode) {
            const bytes = bytecode instanceof BytecodeBuilder ? bytecode.toArray() : Array.from(bytecode);
            return host.patcher.patchFunction(packageName, className, funcName, bytes);
        },

        /**
         * Injects a brand-new custom UnrealScript function export into a package
         * @param {string} packageName e.g. "TAGame.upk"
         * @param {string} className Outer class e.g. "CarMeshComponent_TA"
         * @param {string} funcName Name of new function e.g. "RLIM_SetUnderglow"
         * @param {object} options
         */
        createFunction(packageName, className, funcName, options = {}) {
            const flags = options.flags || (FunctionFlags.FUNC_Public | FunctionFlags.FUNC_Simulated);
            const bytecode = options.bytecode || [];
            const bytes = bytecode instanceof BytecodeBuilder ? bytecode.toArray() : Array.from(bytecode);
            return host.patcher.createFunction(packageName, className, funcName, flags, bytes);
        },

        /**
         * Neutralizes a function by returning a constant value
         */
        neutralize(packageName, className, funcName, returnBool = true) {
            return host.patcher.neutralizeFunction(packageName, className, funcName, returnBool);
        }
    },

    /**
     * Stance, camber, and RGB shader controls
     */
    shaders: {
        /**
         * Adjust vehicle camber / stance
         */
        setCamber(frontDegrees, rearDegrees) {
            return host.shaders.setCamber(frontDegrees, rearDegrees);
        },

        /**
         * Adjust neon underglow
         */
        setUnderglow(rgb = [0, 1, 1], brightness = 2.0, mode = 0) {
            const [r, g, b] = rgb;
            return host.shaders.setUnderglow(r, g, b, brightness, mode);
        }
    },

    /**
     * Spawning cosmetics into local loadout
     */
    spawner: {
        spawnItem(productId, paintId = 0) {
            return host.spawner.spawnItem(productId, paintId);
        }
    },

    /**
     * Configuration registry using hyphenated RLIM-xx-xx-xx keys
     */
    config: {
        get(key, defaultValue = "") {
            return host.config.get(key, defaultValue);
        },
        set(key, value) {
            return host.config.set(key, String(value));
        }
    },

    log(msg) {
        console.log(msg);
    }
};

export { Opcodes, FunctionFlags, BytecodeBuilder };
export default rlim;
