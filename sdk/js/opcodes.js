/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

export const Opcodes = Object.freeze({
    EX_LOCAL_VARIABLE:        0x46,
    EX_INSTANCE_VARIABLE:     0x2B,
    EX_STRUCT_MEMBER:         0x35,
    EX_DYN_ARRAY_ELEMENT:     0x5E,
    EX_INT_ZERO:              0x25,
    EX_INT_ONE:               0x26,
    EX_INT_CONST_BYTE:        0x2C,
    EX_INT_CONST:             0x1D,
    EX_FLOAT_CONST:           0x1E,
    EX_STRING_CONST:          0x1F,
    EX_TRUE_CONST:            0x27,
    EX_FALSE_CONST:           0x28,
    EX_LET:                   0x0F,
    EX_RETURN:                0x04,
    EX_RETURN_VALUE:          0x3A,
    EX_END_OF_SCRIPT:         0x4C,
    EX_NOTHING:               0x0B,
    EX_JUMP:                  0x06,
    EX_JUMP_IF_NOT:           0x07,
    EX_FINAL_FUNCTION:        0x1C,
    EX_VIRTUAL_FUNCTION:      0x1B,
    EX_GLOBAL_FUNCTION:       0x38,
    EX_END_FUNCTION_PARMS:    0x16,
    EX_PRIMITIVE_CAST:        0x38,
    EX_EQUAL_EQUAL_INT_INT:   0x9A,
    EX_NOT_EQUAL_INT_INT:     0x9B,
    EX_BOOL_VARIABLE:         0x2D,
});

export const FunctionFlags = Object.freeze({
    FUNC_None:          0x00000000,
    FUNC_Final:         0x00000001,
    FUNC_Defined:       0x00000002,
    FUNC_Public:        0x00020000,
    FUNC_Private:       0x00040000,
    FUNC_Protected:     0x00080000,
    FUNC_Native:        0x00000400,
    FUNC_Event:         0x00000800,
    FUNC_Static:        0x00002000,
    FUNC_Simulated:     0x00000100,
    FUNC_Exec:          0x00000200,
});
