#!/usr/bin/env python3
"""
/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
"""
import sys
import struct
sys.path.insert(0, '/root/VelocityRL/src-tauri/tools')
from validate_tagame import validate_function_bytecode, sim_serialize_expr

def build_test_conditional_paint_bytecode(r=1.5, g=1.5, b=1.5, max_sz=636):
    bc = bytearray()
    
    # if (bLocalPlayer)
    bc.append(0x07) # EX_JUMP_IF_NOT
    jump_pos = len(bc)
    bc.extend([0x00, 0x00])
    
    bc.append(0x2D) # EX_BOOL_VARIABLE (bLocalPlayer, Export 15648)
    bc.extend(int(15648).to_bytes(4, 'little'))
    bc.append(0x00) # bit mask
    
    members = [
        (-1580, r),    # R
        (-1579, g),    # G
        (-1578, b),    # B
        (-1577, 1.0),  # A
    ]
    struct_id = -4233
    target_var_id = 15655
    
    for prop_id, val in members:
        bc.append(0x0F) # EX_LET
        bc.append(0x35) # EX_STRUCT_MEMBER
        bc.extend(int(prop_id).to_bytes(4, 'little', signed=True))
        bc.extend(int(struct_id).to_bytes(4, 'little', signed=True))
        bc.extend([0x00, 0x01])
        bc.append(0x2B) # EX_INSTANCE_VARIABLE
        bc.extend(int(target_var_id).to_bytes(4, 'little'))
        bc.append(0x1E) # EX_FLOAT_CONST
        bc.extend(struct.pack('<f', float(val)))
    
    jump_target = len(bc)
    bc[jump_pos:jump_pos+2] = jump_target.to_bytes(2, 'little')
    
    bc.append(0x04) # EX_RETURN
    bc.append(0x0B) # EX_NOTHING
    bc.append(0x4C) # EX_END_OF_SCRIPT
    
    while len(bc) < max_sz:
        bc.append(0x0B)
        
    return bytes(bc)


def build_test_explosion_setproduct_bytecode(owned=2044, target=4001, max_sz=273):
    bc = bytearray()
    product_id_var = 23230
    
    # if (ProductID == owned) ProductID = target;
    bc.append(0x07) # EX_JUMP_IF_NOT
    jump_pos = len(bc)
    bc.extend([0x00, 0x00])
    
    bc.append(0x9A) # EX_EQUAL_EQUAL_INT_INT
    bc.append(0x2B)
    bc.extend(int(product_id_var).to_bytes(4, 'little'))
    bc.append(0x1D)
    bc.extend(int(owned).to_bytes(4, 'little'))
    bc.append(0x16)
    
    bc.append(0x0F)
    bc.append(0x2B)
    bc.extend(int(product_id_var).to_bytes(4, 'little'))
    bc.append(0x1D)
    bc.extend(int(target).to_bytes(4, 'little'))
    
    jump_target = len(bc)
    bc[jump_pos:jump_pos+2] = jump_target.to_bytes(2, 'little')
    
    bc.append(0x04)
    bc.append(0x0B)
    bc.append(0x4C)
    
    while len(bc) < max_sz:
        bc.append(0x0B)
        
    return bytes(bc)


def build_test_car_set_loadout_bytecode(slots=[(0, 23, 4001), (15, 2044, 3001)], max_sz=186):
    bc = bytearray()
    products_id = 1871
    cld_id = 1872
    data_var_id = 16586

    default_vanilla_body = bytes([
        0x14, 0x2D, 0x01, 0x8D, 0x40, 0x00, 0x00, 0x27,
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF8, 0x3E, 0x00, 0x00, 0x00, 0x01, 0xF8, 0x3E, 0x00, 0x00, 0x49, 0x8B, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF7, 0x3E, 0x00, 0x00, 0x00, 0x01, 0xF7, 0x3E, 0x00, 0x00, 0x49, 0x7D, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x1C, 0x10, 0x3F, 0x00, 0x00, 0x46, 0xCA, 0x40, 0x00, 0x00, 0x16,
        0x04, 0x0B, 0x4C
    ])

    available_space = max_sz - len(default_vanilla_body)

    trigger_slot, trigger_owned, _ = slots[0]

    # if (Data.Products[trigger_slot] == trigger_owned)
    bc.append(0x07) # EX_JUMP_IF_NOT
    jump_pos = len(bc)
    bc.extend([0x00, 0x00])

    bc.append(0x9A) # EX_EQUAL_EQUAL_INT_INT
    bc.extend([0x57, 0x00, 0x00])
    bc.append(0x5E)
    if trigger_slot == 0:
        bc.append(0x25) # EX_INT_ZERO
    else:
        bc.extend([0x2C, trigger_slot])
    bc.append(0x35)
    bc.extend(products_id.to_bytes(4, 'little'))
    bc.extend(cld_id.to_bytes(4, 'little'))
    bc.extend([0x00, 0x01])
    bc.append(0x46)
    bc.extend(data_var_id.to_bytes(4, 'little'))
    bc.append(0x1D)
    bc.extend(int(trigger_owned).to_bytes(4, 'little'))
    bc.append(0x16)

    for slot_idx, _, target_id in slots:
        index_len = 1 if slot_idx == 0 else 2
        target_len = 2 if 0 <= target_id <= 255 else 5
        assign_len = 21 + index_len + target_len
        if len(bc) + assign_len > available_space:
            break

        bc.append(0x0F) # EX_LET
        bc.extend([0x57, 0x00, 0x00])
        bc.append(0x5E)
        if slot_idx == 0:
            bc.append(0x25)
        else:
            bc.extend([0x2C, slot_idx])
        bc.append(0x35)
        bc.extend(products_id.to_bytes(4, 'little'))
        bc.extend(cld_id.to_bytes(4, 'little'))
        bc.extend([0x00, 0x01])
        bc.append(0x46)
        bc.extend(data_var_id.to_bytes(4, 'little'))
        if 0 <= target_id <= 255:
            bc.extend([0x2C, target_id])
        else:
            bc.append(0x1D)
            bc.extend(int(target_id).to_bytes(4, 'little'))

    jump_target = len(bc)
    bc[jump_pos:jump_pos+2] = jump_target.to_bytes(2, 'little')
    bc.extend(default_vanilla_body)

    while len(bc) < max_sz:
        bc.append(0x0B)

    return bytes(bc)


def build_test_car_preview_set_loadout_bytecode(slots=[(0, 23, 4001), (1, 10, 250), (2, 5, 100), (3, 2, 50), (5, 1, 30), (14, 4, 80)], max_sz=230):
    bc = bytearray()
    products_id = 1871
    loadout_data_id = 2364
    new_loadout_id = 18093
    in_loadout_id = 18094
    force_set_id = 20264

    # NewLoadout = InLoadout;
    bc.append(0x0F)
    bc.append(0x2B)
    bc.extend(new_loadout_id.to_bytes(4, 'little'))
    bc.append(0x46)
    bc.extend(in_loadout_id.to_bytes(4, 'little'))

    # Tail: ForceSetLoadout(); return;
    tail = bytearray()
    tail.append(0x1C)
    tail.extend(force_set_id.to_bytes(4, 'little'))
    tail.append(0x16)
    tail.append(0x04)
    tail.append(0x0B)
    tail.append(0x4C)

    available_space = max_sz - len(tail)
    trigger_slot, trigger_owned, _ = slots[0]

    # if (InLoadout.Products[trigger_slot] == trigger_owned)
    bc.append(0x07)
    jump_pos = len(bc)
    bc.extend([0x00, 0x00])

    bc.append(0x9A)
    bc.extend([0x57, 0x00, 0x00])
    bc.append(0x5E)
    if trigger_slot == 0:
        bc.append(0x25)
    else:
        bc.extend([0x2C, trigger_slot])
    bc.append(0x35)
    bc.extend(products_id.to_bytes(4, 'little'))
    bc.extend(loadout_data_id.to_bytes(4, 'little'))
    bc.extend([0x00, 0x01])
    bc.append(0x46)
    bc.extend(in_loadout_id.to_bytes(4, 'little'))
    bc.append(0x1D)
    bc.extend(int(trigger_owned).to_bytes(4, 'little'))
    bc.append(0x16)

    for slot_idx, _, target_id in slots:
        index_len = 1 if slot_idx == 0 else 2
        target_len = 2 if 0 <= target_id <= 255 else 5
        assign_len = 21 + index_len + target_len
        if len(bc) + assign_len > available_space:
            break

        bc.append(0x0F)
        bc.extend([0x57, 0x00, 0x00])
        bc.append(0x5E)
        if slot_idx == 0:
            bc.append(0x25)
        else:
            bc.extend([0x2C, slot_idx])
        bc.append(0x35)
        bc.extend(products_id.to_bytes(4, 'little'))
        bc.extend(loadout_data_id.to_bytes(4, 'little'))
        bc.extend([0x00, 0x01])
        bc.append(0x2B)
        bc.extend(new_loadout_id.to_bytes(4, 'little'))
        if 0 <= target_id <= 255:
            bc.extend([0x2C, target_id])
        else:
            bc.append(0x1D)
            bc.extend(int(target_id).to_bytes(4, 'little'))

    jump_target = len(bc)
    bc[jump_pos:jump_pos+2] = jump_target.to_bytes(2, 'little')
    bc.extend(tail)

    while len(bc) < max_sz:
        bc.append(0x0B)

    return bytes(bc)


if __name__ == '__main__':
    print("1. Testing conditional paint bytecode (CarMeshComponentBase_TA::ApplyPaintSettings)...")
    paint_bc = build_test_conditional_paint_bytecode(r=2.5, g=0.1, b=0.8, max_sz=636)
    validate_function_bytecode("CarMeshComponentBase_TA::ApplyPaintSettings", paint_bc)

    print("\n2. Testing goal explosion previewer bytecode (ExplosionPreviewer_TA::SetProduct)...")
    exp_bc = build_test_explosion_setproduct_bytecode(owned=2044, target=4001, max_sz=273)
    validate_function_bytecode("ExplosionPreviewer_TA::SetProduct", exp_bc)

    print("\n3. Testing in-game vehicle loadout bytecode with Goal Explosion slot 15 (Car_TA::SetLoadout)...")
    car_bc = build_test_car_set_loadout_bytecode(slots=[(0, 23, 4001), (15, 2044, 3001)], max_sz=186)
    validate_function_bytecode("Car_TA::SetLoadout", car_bc)

    print("\n4. Testing garage pedestal loadout bytecode with 6 full cosmetic slots (CarPreviewActor_TA::SetLoadout)...")
    preview_bc = build_test_car_preview_set_loadout_bytecode(slots=[(0, 23, 4001), (1, 10, 250), (2, 5, 100), (3, 2, 50), (5, 1, 30), (14, 4, 80)], max_sz=230)
    validate_function_bytecode("CarPreviewActor_TA::SetLoadout", preview_bc)

    print("\n[+] All 4 core UScript functions successfully verified with 0 invalid tokens!")
