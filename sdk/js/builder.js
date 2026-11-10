/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

import { Opcodes } from "./opcodes.js";

export class BytecodeBuilder {
    constructor() {
        this.bytes = [];
    }

    byte(b) {
        this.bytes.push(b & 0xFF);
        return this;
    }

    /** Emits little-endian 32-bit integer */
    int32(val) {
        this.bytes.push(val & 0xFF);
        this.bytes.push((val >> 8) & 0xFF);
        this.bytes.push((val >> 16) & 0xFF);
        this.bytes.push((val >> 24) & 0xFF);
        return this;
    }

    /** Emits little-endian 32-bit IEEE-754 float */
    float32(val) {
        const buf = new ArrayBuffer(4);
        new DataView(buf).setFloat32(0, val, true);
        const u8 = new Uint8Array(buf);
        for (let i = 0; i < 4; i++) this.bytes.push(u8[i]);
        return this;
    }

    /** Assign expression: Target = Value */
    let(targetBuilder, valueBuilder) {
        this.byte(Opcodes.EX_LET);
        targetBuilder(this);
        valueBuilder(this);
        return this;
    }

    /** References an instance variable by name table / export index */
    instanceVar(varIndex) {
        this.byte(Opcodes.EX_INSTANCE_VARIABLE);
        this.int32(varIndex);
        return this;
    }

    /** References a local variable by export index */
    localVar(varIndex) {
        this.byte(Opcodes.EX_LOCAL_VARIABLE);
        this.int32(varIndex);
        return this;
    }

    /** Float constant */
    constFloat(val) {
        this.byte(Opcodes.EX_FLOAT_CONST);
        this.float32(val);
        return this;
    }

    /** Integer constant */
    constInt(val) {
        if (val === 0) {
            this.byte(Opcodes.EX_INT_ZERO);
        } else if (val === 1) {
            this.byte(Opcodes.EX_INT_ONE);
        } else if (val >= 0 && val <= 255) {
            this.byte(Opcodes.EX_INT_CONST_BYTE);
            this.byte(val);
        } else {
            this.byte(Opcodes.EX_INT_CONST);
            this.int32(val);
        }
        return this;
    }

    /** Boolean constant */
    constBool(val) {
        this.byte(val ? Opcodes.EX_TRUE_CONST : Opcodes.EX_FALSE_CONST);
        return this;
    }

    /** Function call: virtual or final */
    call(funcIndex, ...argBuilders) {
        this.byte(Opcodes.EX_FINAL_FUNCTION);
        this.int32(funcIndex);
        for (const arg of argBuilders) {
            arg(this);
        }
        this.byte(Opcodes.EX_END_FUNCTION_PARMS);
        return this;
    }

    /** Return opcode */
    return(exprBuilder = null) {
        this.byte(Opcodes.EX_RETURN);
        if (exprBuilder) {
            exprBuilder(this);
        } else {
            this.byte(Opcodes.EX_NOTHING);
        }
        return this;
    }

    /** Pads with EX_NOTHING up to targetSize - 1, then ends with EX_END_OF_SCRIPT */
    padToSize(targetSize) {
        while (this.bytes.length + 1 < targetSize) {
            this.bytes.push(Opcodes.EX_NOTHING);
        }
        this.bytes.push(Opcodes.EX_END_OF_SCRIPT);
        return this;
    }

    /** Builds to array of byte values */
    toArray() {
        return [...this.bytes];
    }

    /** Builds to Uint8Array */
    toUint8Array() {
        return new Uint8Array(this.bytes);
    }
}
