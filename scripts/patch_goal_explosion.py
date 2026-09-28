#!/usr/bin/env python3
"""
TAGame.upk Standalone Goal Explosion & Item Bytecode Patcher
Patches ConvertToClientLoadout & CorrectOnlineData in TAGame.upk to swap Goal Explosions and Items.
"""

import os
import sys
import struct
import zlib
import argparse
from typing import List, Dict, Tuple

# Official AES-256 Key for TAGame.upk Header
TAGAME_KEY = bytes([
    0xc7, 0xdf, 0x6b, 0x13, 0x25, 0x2a, 0xcc, 0x71,
    0x47, 0xbb, 0x51, 0xc9, 0x8a, 0xd7, 0xe3, 0x4b,
    0x7f, 0xe5, 0x00, 0xb7, 0x7f, 0xa5, 0xfa, 0xb2,
    0x93, 0xe2, 0xf2, 0x4e, 0x6b, 0x17, 0xe7, 0x79,
])

# UE3 64-bit Opcodes
EX_LOCAL_VARIABLE        = 0x46
EX_INSTANCE_VARIABLE     = 0x2B
EX_STRUCT_MEMBER         = 0x35
EX_DYN_ARRAY_OP          = 0x57
EX_INT_ZERO              = 0x25
EX_INT_CONST_BYTE        = 0x2C
EX_INT_CONST             = 0x1D
EX_LET                   = 0x0F
EX_RETURN                = 0x04
EX_RETURN_VALUE          = 0x3A
EX_END_OF_SCRIPT         = 0x4C
EX_NOTHING               = 0x0B
EX_JUMP_IF_NOT           = 0x07
EX_EQUAL_EQUAL_INT_INT   = 0x9A
EX_END_FUNCTION_PARMS    = 0x16

POPULAR_GOAL_EXPLOSIONS = {
    "reaper": 2526,
    "electroshock": 1904,
    "hellfire": 1905,
    "fireworks": 1906,
    "subzero": 1907,
    "partytime": 2173,
    "popcorn": 2174,
    "duelingdragons": 2574,
    "atomizer": 2983,
    "juiced": 3037,
    "singularity": 3192,
    "supernova": 3283,
    "solarflare": 3448,
    "shattered": 4272,
    "bigsplash": 4441,
    "gravitybomb": 4902,
    "phoenixfire": 6259,
}

def decrypt_ecb(key: bytes, data: bytes) -> bytes:
    try:
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
        from cryptography.hazmat.backends import default_backend
        cipher = Cipher(algorithms.AES(key), modes.ECB(), backend=default_backend())
        decryptor = cipher.decryptor()
        return decryptor.update(data) + decryptor.finalize()
    except ImportError:
        pass
    try:
        from Crypto.Cipher import AES
        return AES.new(key, AES.MODE_ECB).decrypt(data)
    except ImportError:
        raise RuntimeError("Please install 'cryptography' (pip install cryptography)")

def encrypt_ecb(key: bytes, data: bytes) -> bytes:
    try:
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
        from cryptography.hazmat.backends import default_backend
        cipher = Cipher(algorithms.AES(key), modes.ECB(), backend=default_backend())
        encryptor = cipher.encryptor()
        return encryptor.update(data) + encryptor.finalize()
    except ImportError:
        pass
    try:
        from Crypto.Cipher import AES
        return AES.new(key, AES.MODE_ECB).encrypt(data)
    except ImportError:
        raise RuntimeError("Please install 'cryptography' (pip install cryptography)")

def emit_convert_to_client_loadout(slot_overrides: List[Tuple[int, int, int]], max_disk_size: int = 3000) -> Tuple[bytes, int]:
    """
    Emits bytecode for ConvertToClientLoadout:
    1. NewLoadout.Products = FromData.Products;
    2. Overrides: if (NewLoadout.Products[slot] == owned_id) NewLoadout.Products[slot] = target_id;
    3. return NewLoadout;
    4. NOP padding.
    """
    bc = bytearray()
    mem_sz = 0

    # 1. NewLoadout.Products = FromData.Products;
    bc.append(EX_LET)
    bc.append(EX_STRUCT_MEMBER)
    bc.extend(struct.pack("<i", 1871))
    bc.extend(struct.pack("<i", 1872))
    bc.extend(b"\x00\x01")
    bc.append(EX_INSTANCE_VARIABLE)
    bc.extend(struct.pack("<i", 75))
    bc.append(EX_STRUCT_MEMBER)
    bc.extend(struct.pack("<i", 1871))
    bc.extend(struct.pack("<i", 2364))
    bc.extend(b"\x00\x00")
    bc.append(EX_LOCAL_VARIABLE)
    bc.extend(struct.pack("<i", 77))

    mem_sz += 57

    # 2. Overrides
    for slot_idx, owned_id, target_id in slot_overrides:
        bc.append(EX_JUMP_IF_NOT)
        jump_pos = len(bc)
        bc.extend(b"\x00\x00")

        # EqualEqual_IntInt(NewLoadout.Products[slot], owned_id)
        bc.append(EX_EQUAL_EQUAL_INT_INT)
        bc.append(EX_DYN_ARRAY_OP)
        bc.extend(b"\x00\x00")
        if slot_idx == 0:
            bc.append(EX_INT_ZERO)
            index_mem = 1
        else:
            bc.append(EX_INT_CONST_BYTE)
            bc.append(slot_idx)
            index_mem = 2

        bc.append(EX_STRUCT_MEMBER)
        bc.extend(struct.pack("<i", 1871))
        bc.extend(struct.pack("<i", 1872))
        bc.extend(b"\x00\x01")
        bc.append(EX_INSTANCE_VARIABLE)
        bc.extend(struct.pack("<i", 75))

        bc.append(EX_INT_CONST)
        bc.extend(struct.pack("<i", owned_id))
        bc.append(EX_END_FUNCTION_PARMS)

        # Body: NewLoadout.Products[slot] = target_id
        bc.append(EX_LET)
        bc.append(EX_DYN_ARRAY_OP)
        bc.extend(b"\x00\x00")
        if slot_idx == 0:
            bc.append(EX_INT_ZERO)
        else:
            bc.append(EX_INT_CONST_BYTE)
            bc.append(slot_idx)

        bc.append(EX_STRUCT_MEMBER)
        bc.extend(struct.pack("<i", 1871))
        bc.extend(struct.pack("<i", 1872))
        bc.extend(b"\x00\x01")
        bc.append(EX_INSTANCE_VARIABLE)
        bc.extend(struct.pack("<i", 75))
        bc.append(EX_INT_CONST)
        bc.extend(struct.pack("<i", target_id))

        cond_mem = 1 + 3 + index_mem + 28 + 5 + 1
        body_mem = 1 + 3 + index_mem + 28 + 5
        total_rule_mem = 3 + cond_mem + body_mem
        jump_target = mem_sz + total_rule_mem
        struct.pack_into("<H", bc, jump_pos, jump_target)
        mem_sz += total_rule_mem

    # 3. return NewLoadout;
    bc.append(EX_RETURN_VALUE)
    bc.append(EX_INSTANCE_VARIABLE)
    bc.extend(struct.pack("<i", 75))
    bc.append(EX_END_OF_SCRIPT)
    mem_sz += 1 + 9 + 1

    if len(bc) > max_disk_size:
        raise ValueError(f"Bytecode size {len(bc)} exceeds max size {max_disk_size}")

    nop_count = max_disk_size - len(bc)
    bc.extend(b"\x0b" * nop_count)
    mem_sz += nop_count
    return bytes(bc), mem_sz

def patch_tagame(tagame_path: str, target_explosion_id: int):
    if not os.path.isfile(tagame_path):
        print(f"[-] File not found: {tagame_path}")
        return False

    bak_path = tagame_path + ".bak"
    if not os.path.isfile(bak_path):
        import shutil
        shutil.copy2(tagame_path, bak_path)
        print(f"[+] Created backup: {bak_path}")

    with open(tagame_path, "rb") as f:
        file_bytes = bytearray(f.read())

    total_header_size = struct.unpack_from("<I", file_bytes, 8)[0]
    p = 12
    flen = struct.unpack_from("<i", file_bytes, p)[0]
    p += 4 + (flen if flen > 0 else -flen * 2)
    p += 4
    name_count = struct.unpack_from("<i", file_bytes, p)[0]
    p += 4
    name_offset = struct.unpack_from("<I", file_bytes, p)[0]
    export_count = struct.unpack_from("<i", file_bytes, p + 4)[0]
    export_offset = struct.unpack_from("<i", file_bytes, p + 8)[0]
    depends_offset = struct.unpack_from("<i", file_bytes, p + 20)[0]

    enc_size = (total_header_size - name_offset + 15) & ~15
    enc_end = name_offset + enc_size
    plain_header = bytearray(decrypt_ecb(TAGAME_KEY, bytes(file_bytes[name_offset:enc_end])))

    # Names
    names = []
    n_pos = 0
    for _ in range(name_count):
        if n_pos + 4 > len(plain_header):
            break
        slen = struct.unpack_from("<i", plain_header, n_pos)[0]
        n_pos += 4
        if slen > 0:
            s = plain_header[n_pos:n_pos + slen - 1].decode("latin-1", errors="replace")
            n_pos += slen + 8
        elif slen < 0:
            u_bytes = (-slen) * 2
            s = plain_header[n_pos:n_pos + u_bytes - 2].decode("utf-16le", errors="replace")
            n_pos += u_bytes + 8
        else:
            s = ""
            n_pos += 8
        names.append(s)

    # Exports
    exports = []
    exp_rel = export_offset - name_offset
    dep_rel = depends_offset - name_offset
    pos = exp_rel
    target_exp_idx = None
    while pos + 72 <= dep_rel and pos + 72 <= len(plain_header) and len(exports) < export_count:
        name_idx = struct.unpack_from("<i", plain_header, pos + 12)[0]
        serial_size = struct.unpack_from("<i", plain_header, pos + 32)[0]
        serial_offset = struct.unpack_from("<q", plain_header, pos + 36)[0]
        noc = struct.unpack_from("<i", plain_header, pos + 48)[0]

        name_str = names[name_idx] if 0 <= name_idx < len(names) else ""
        if name_str == "ConvertToClientLoadout":
            target_exp_idx = len(exports)

        exports.append({
            "pos": pos,
            "name": name_str,
            "serial_size": serial_size,
            "serial_offset": serial_offset,
        })
        pos += 72 + max(0, noc) * 4

    if target_exp_idx is None:
        print("[-] Function ConvertToClientLoadout not found in Export Table.")
        return False

    target_exp = exports[target_exp_idx]

    # Chunk 0
    c_pos = dep_rel
    chunk_count = struct.unpack_from("<i", plain_header, c_pos)[0]
    c_pos += 4
    chunks = []
    for _ in range(chunk_count):
        u_off, u_sz, c_off, c_sz = struct.unpack_from("<qiqq", plain_header, c_pos)
        chunks.append({
            "pos": c_pos,
            "uncomp_offset": u_off,
            "uncomp_size": u_sz,
            "comp_offset": c_off,
            "comp_size": c_sz,
        })
        c_pos += 36

    c0 = chunks[0]
    b_csz = struct.unpack_from("<i", file_bytes, c0["comp_offset"] + 16)[0]
    decomp0 = bytearray(zlib.decompress(bytes(file_bytes[c0["comp_offset"] + 24:c0["comp_offset"] + 24 + b_csz])))

    func_off0 = target_exp["serial_offset"] - c0["uncomp_offset"]
    orig_disk_sz = struct.unpack_from("<I", decomp0, func_off0 + 44)[0]

    # Goal Explosion slot overrides (Slot 10 and Slot 15: 1903=Classic, 0=Default)
    rules = [
        (10, 1903, target_explosion_id), # Slot 10 Classic
        (10, 0, target_explosion_id),    # Slot 10 Default/0
        (15, 1903, target_explosion_id), # Slot 15 Classic
        (15, 0, target_explosion_id),    # Slot 15 Default/0
    ]

    EXPANDED_SIZE = 3000
    if orig_disk_sz < EXPANDED_SIZE:
        delta = EXPANDED_SIZE - orig_disk_sz
        insert_pos = func_off0 + 48 + orig_disk_sz
        decomp0[insert_pos:insert_pos] = b"\x0b" * delta

        payload0, mem_sz0 = emit_convert_to_client_loadout(rules, EXPANDED_SIZE)
        struct.pack_into("<I", decomp0, func_off0 + 40, mem_sz0)
        struct.pack_into("<I", decomp0, func_off0 + 44, EXPANDED_SIZE)
        decomp0[func_off0 + 48:func_off0 + 48 + EXPANDED_SIZE] = payload0

        # Update export table
        new_serial_sz = target_exp["serial_size"] + delta
        struct.pack_into("<i", plain_header, target_exp["pos"] + 32, new_serial_sz)
        for exp in exports:
            if exp["serial_offset"] > target_exp["serial_offset"]:
                struct.pack_into("<q", plain_header, exp["pos"] + 36, exp["serial_offset"] + delta)

        # Update chunk table
        new_c0_uncomp = c0["uncomp_size"] + delta
        struct.pack_into("<i", plain_header, c0["pos"] + 8, new_c0_uncomp)
        for ch in chunks[1:]:
            struct.pack_into("<q", plain_header, ch["pos"], ch["uncomp_offset"] + delta)
    else:
        payload0, mem_sz0 = emit_convert_to_client_loadout(rules, orig_disk_sz)
        struct.pack_into("<I", decomp0, func_off0 + 40, mem_sz0)
        decomp0[func_off0 + 48:func_off0 + 48 + orig_disk_sz] = payload0

    # Recompress Chunk 0
    recomp0 = bytearray(zlib.compress(bytes(decomp0), 9))
    orig_c0_sz = c0["comp_size"]
    if len(recomp0) > orig_c0_sz:
        print(f"[-] Recompressed Chunk 0 ({len(recomp0)}) exceeds buffer ({orig_c0_sz})")
        return False

    recomp0.extend(b"\x00" * (orig_c0_sz - len(recomp0)))
    file_bytes[c0["comp_offset"] + 24:c0["comp_offset"] + 24 + orig_c0_sz] = recomp0

    # Re-encrypt header
    re_enc = encrypt_ecb(TAGAME_KEY, bytes(plain_header))
    file_bytes[name_offset:enc_end] = re_enc

    with open(tagame_path, "wb") as f:
        f.write(file_bytes)

    print(f"[+] Successfully patched TAGame.upk with Goal Explosion ID {target_explosion_id}!")
    return True

def main():
    parser = argparse.ArgumentParser(description="TAGame.upk Goal Explosion Patcher")
    parser.add_argument("tagame_path", nargs="?", default="", help="Path to TAGame.upk")
    parser.add_argument("--explosion", "-e", default="reaper", help="Goal Explosion name (e.g. reaper, electroshock, duelingdragons, hellfire, bigsplash) or ID number")
    args = parser.parse_args()

    tagame_path = args.tagame_path
    if not tagame_path:
        candidates = [
            r"E:\games\rocketleague\TAGame\CookedPCConsole\TAGame.upk",
            r"C:\Program Files\Epic Games\rocketleague\TAGame\CookedPCConsole\TAGame.upk",
            r"C:\Program Files (x86)\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole\TAGame.upk",
            "TAGame.upk",
        ]
        for c in candidates:
            if os.path.isfile(c):
                tagame_path = c
                break

    if not tagame_path or not os.path.isfile(tagame_path):
        print("Error: Could not locate TAGame.upk. Please provide the path.")
        sys.exit(1)

    exp_arg = args.explosion.lower().replace(" ", "").replace("_", "")
    if exp_arg.isdigit():
        explosion_id = int(exp_arg)
    elif exp_arg in POPULAR_GOAL_EXPLOSIONS:
        explosion_id = POPULAR_GOAL_EXPLOSIONS[exp_arg]
    else:
        print(f"Unknown explosion '{args.explosion}'. Available names:")
        for name in POPULAR_GOAL_EXPLOSIONS:
            print(f"  - {name}")
        sys.exit(1)

    print(f"[*] Patching TAGame.upk at: {tagame_path}")
    print(f"[*] Target Goal Explosion: {args.explosion} (ID: {explosion_id})")
    patch_tagame(tagame_path, explosion_id)

if __name__ == "__main__":
    main()
