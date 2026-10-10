#!/usr/bin/env python3
import sys
import struct
sys.path.insert(0, '/root/VelocityRL/src-tauri/tools')
from validate_tagame import validate_function_bytecode, sim_serialize_expr

def build_test_conditional_paint_bytecode(r=1.5, g=1.5, b=1.5, max_sz=636):
    """
    Constructs conditional paint bytecode for CarMeshComponentBase_TA::ApplyPaintSettings:
    if (bLocalPlayer)
    {
        CustomColorOverride.R = r;
        CustomColorOverride.G = g;
        CustomColorOverride.B = b;
        CustomColorOverride.A = 1.0f;
    }
    """
    bc = bytearray()
    
    # 1. EX_JUMP_IF_NOT: if (bLocalPlayer)
    bc.append(0x07)
    jump_pos = len(bc)
    bc.extend([0x00, 0x00])
    
    # Condition: bLocalPlayer (Export #15648)
    # UE3 bool variable read: 0x2D 0x01 <4 bytes var_id>
    bc.append(0x2D) # EX_BOOL_VARIABLE
    bc.extend(int(15648).to_bytes(4, 'little'))
    bc.append(0x00) # bit mask
    
    # 2. Assignments for R, G, B, A
    members = [
        (-1580, r),    # R
        (-1579, g),    # G
        (-1578, b),    # B
        (-1577, 1.0),  # A
    ]
    struct_id = -4233  # LinearColor
    target_var_id = 15655 # CustomColorOverride
    
    for prop_id, val in members:
        bc.append(0x0F) # EX_LET
        bc.append(0x35) # EX_STRUCT_MEMBER
        bc.extend(int(prop_id).to_bytes(4, 'little', signed=True))
        bc.extend(int(struct_id).to_bytes(4, 'little', signed=True))
        bc.extend([0x00, 0x01]) # flags
        bc.append(0x2B) # EX_INSTANCE_VARIABLE
        bc.extend(int(target_var_id).to_bytes(4, 'little'))
        bc.append(0x1E) # EX_FLOAT_CONST
        bc.extend(struct.pack('<f', float(val)))
    
    # Patch jump target
    jump_target = len(bc)
    bc[jump_pos:jump_pos+2] = jump_target.to_bytes(2, 'little')
    
    # Return statement
    bc.append(0x04) # EX_RETURN
    bc.append(0x0B) # EX_NOTHING
    bc.append(0x4C) # EX_END_OF_SCRIPT
    
    while len(bc) < max_sz:
        bc.append(0x0B) # EX_NOTHING
        
    return bytes(bc)


def build_test_explosion_setproduct_bytecode(owned=2044, target=4001, max_sz=273):
    """
    Constructs bytecode for ExplosionPreviewer_TA::SetProduct:
    if (ProductID == owned)
    {
        ProductID = target;
    }
    """
    bc = bytearray()
    product_id_var = 23230 # ProductID local/param
    
    # if (ProductID == owned) ProductID = target;
    bc.append(0x07) # EX_JUMP_IF_NOT
    jump_pos = len(bc)
    bc.extend([0x00, 0x00])
    
    # 0x9A ==
    bc.append(0x9A)
    bc.append(0x2B) # EX_INSTANCE_VARIABLE / LOCAL
    bc.extend(int(product_id_var).to_bytes(4, 'little'))
    bc.append(0x1D) # EX_INT_CONST
    bc.extend(int(owned).to_bytes(4, 'little'))
    bc.append(0x16) # EX_END_FUNCTION_PARMS
    
    # ProductID = target
    bc.append(0x0F) # EX_LET
    bc.append(0x2B) # EX_INSTANCE_VARIABLE / LOCAL
    bc.extend(int(product_id_var).to_bytes(4, 'little'))
    bc.append(0x1D) # EX_INT_CONST
    bc.extend(int(target).to_bytes(4, 'little'))
    
    jump_target = len(bc)
    bc[jump_pos:jump_pos+2] = jump_target.to_bytes(2, 'little')
    
    bc.append(0x04) # EX_RETURN
    bc.append(0x0B) # EX_NOTHING
    bc.append(0x4C) # EX_END_OF_SCRIPT
    
    while len(bc) < max_sz:
        bc.append(0x0B)
        
    return bytes(bc)

if __name__ == '__main__':
    print("Testing conditional paint bytecode (CarMeshComponentBase_TA::ApplyPaintSettings)...")
    paint_bc = build_test_conditional_paint_bytecode(r=2.5, g=0.1, b=0.8, max_sz=636)
    print(f"Paint bytecode generated: {len(paint_bc)} bytes")
    validate_function_bytecode("CarMeshComponentBase_TA::ApplyPaintSettings", paint_bc)

    print("\nTesting goal explosion previewer bytecode (ExplosionPreviewer_TA::SetProduct)...")
    exp_bc = build_test_explosion_setproduct_bytecode(owned=2044, target=4001, max_sz=273)
    print(f"Explosion preview bytecode generated: {len(exp_bc)} bytes")
    validate_function_bytecode("ExplosionPreviewer_TA::SetProduct", exp_bc)
    print("\n[+] All conditional test bytecodes successfully verified!")
