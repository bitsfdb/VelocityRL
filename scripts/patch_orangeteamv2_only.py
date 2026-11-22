#!/usr/bin/env python3
"""
patch_orangeteamv2_only.py
Patches ONLY the 'OrangeTeamV2' export in TAGame.upk with the expanded 10x21
rich color palette containing BOTH primary colors (BlueTeam + OrangeTeam + Accent).

All other exports (BlueTeamV3, OrangeTeamV3, Accent, etc.) remain 100% vanilla.
When PsyNet class override 'CarColorSet -> OrangeTeamV2' is turned ON in VelocityRL,
the game loads this expanded palette. When turned OFF, it uses the stock team colors.
"""

import os
import sys
import struct
from pathlib import Path

sys.path.append("/root")
import ue3_package as up
try:
    from Crypto.Cipher import AES
except ImportError:
    from Cryptodome.Cipher import AES

TAGAME_KEY = up.TAGAME_KEY

TEAM_HUE_COUNT = 10
ACCENT_HUE_COUNT = 15
SRC_VALUES = 7
STACK_HUE_COUNT = 10
STACK_VALUE_COUNT = 21 # 7 * 3
TOTAL_COLORS = STACK_HUE_COUNT * STACK_VALUE_COUNT # 210

TARGET_EXPORT_NAME = "OrangeTeamV2"

def get_export_raw(pkg, exp):
    so = exp["serial_offset"]
    sz = exp["serial_size"]
    ch = next(c for c in pkg.chunks if c["uncomp_offset"] <= so < c["uncomp_offset"] + c["uncomp_size"])
    rel = so - ch["uncomp_offset"]
    decomp = pkg.get_chunk_decompressed(ch["idx"])
    return decomp[rel:rel + sz]

def parse_props(pkg, data):
    net_idx = struct.unpack_from("<i", data, 0)[0]
    p = 4
    props = []
    while p < len(data):
        n_idx, n_num = struct.unpack_from("<ii", data, p)
        pname = pkg.names[n_idx]
        p += 8
        if pname == "None":
            props.append({"name": "None", "name_idx": n_idx, "name_num": n_num})
            break
        t_idx, t_num = struct.unpack_from("<ii", data, p)
        tname = pkg.names[t_idx]
        p += 8
        size, arr_idx = struct.unpack_from("<ii", data, p)
        p += 8
        inner = None
        if tname in ["StructProperty", "ByteProperty"]:
            in_idx, in_num = struct.unpack_from("<ii", data, p)
            inner = (in_idx, in_num)
            p += 8
        if tname == "BoolProperty":
            bval = data[p:p+1]
            p += 1
            props.append({
                "name": pname, "name_idx": n_idx, "name_num": n_num,
                "type": tname, "type_idx": t_idx, "type_num": t_num,
                "size": size, "arr_idx": arr_idx, "inner": inner,
                "bool": bval
            })
        else:
            body = data[p:p+size]
            p += size
            props.append({
                "name": pname, "name_idx": n_idx, "name_num": n_num,
                "type": tname, "type_idx": t_idx, "type_num": t_num,
                "size": size, "arr_idx": arr_idx, "inner": inner,
                "body": bytearray(body)
            })
    return net_idx, props

def serialize_props(net_idx, props):
    out = bytearray(struct.pack("<i", net_idx))
    for pr in props:
        if pr["name"] == "None":
            out.extend(struct.pack("<ii", pr["name_idx"], pr["name_num"]))
            break
        out.extend(struct.pack("<ii", pr["name_idx"], pr["name_num"]))
        out.extend(struct.pack("<ii", pr["type_idx"], pr["type_num"]))
        if pr["type"] == "BoolProperty":
            out.extend(struct.pack("<ii", 0, pr["arr_idx"]))
            out.extend(pr["bool"])
        else:
            out.extend(struct.pack("<ii", len(pr["body"]), pr["arr_idx"]))
            if pr["inner"]:
                out.extend(struct.pack("<ii", pr["inner"][0], pr["inner"][1]))
            out.extend(pr["body"])
    return bytes(out)

def build_combined_swatches(pkg):
    b3_exp = next(e for e in pkg.exports if e["name"] == "BlueTeamV3")
    o3_exp = next(e for e in pkg.exports if e["name"] == "OrangeTeamV3")
    acc_exp = next(e for e in pkg.exports if e["name"] == "Accent")

    _, b3_p = parse_props(pkg, get_export_raw(pkg, b3_exp))
    _, o3_p = parse_props(pkg, get_export_raw(pkg, o3_exp))
    _, acc_p = parse_props(pkg, get_export_raw(pkg, acc_exp))

    def get_body(props, name):
        return next(p["body"] for p in props if p["name"] == name)

    b3_c = get_body(b3_p, "Colors")[4:]
    b3_d = get_body(b3_p, "DebugColors")[4:]
    o3_c = get_body(o3_p, "Colors")[4:]
    o3_d = get_body(o3_p, "DebugColors")[4:]
    acc_c = get_body(acc_p, "Colors")[4:]
    acc_d = get_body(acc_p, "DebugColors")[4:]

    colors_out = bytearray()
    debug_out = bytearray()

    # Stack layout (10 hues x 21 values = 210 colors):
    # Band 0 (v=0..6): BlueTeam swatches (both primary colors: Blue)
    # Band 1 (v=7..13): OrangeTeam swatches (both primary colors: Orange)
    # Band 2 (v=14..20): Accent swatches (grayscale, white, dark shades)
    for v in range(STACK_VALUE_COUNT):
        band = v // SRC_VALUES
        local_v = v % SRC_VALUES
        for h in range(STACK_HUE_COUNT):
            if band == 0:
                off_c = (local_v * TEAM_HUE_COUNT + h) * 16
                off_d = (local_v * TEAM_HUE_COUNT + h) * 81
                colors_out.extend(b3_c[off_c:off_c + 16])
                debug_out.extend(b3_d[off_d:off_d + 81])
            elif band == 1:
                off_c = (local_v * TEAM_HUE_COUNT + h) * 16
                off_d = (local_v * TEAM_HUE_COUNT + h) * 81
                colors_out.extend(o3_c[off_c:off_c + 16])
                debug_out.extend(o3_d[off_d:off_d + 81])
            else:
                off_c = (local_v * ACCENT_HUE_COUNT + h) * 16
                off_d = (local_v * ACCENT_HUE_COUNT + h) * 81
                colors_out.extend(acc_c[off_c:off_c + 16])
                debug_out.extend(acc_d[off_d:off_d + 81])

    return colors_out, debug_out

def build_patched_orangeteamv2(pkg, colors_payload, debug_payload):
    exp = next(e for e in pkg.exports if e["name"] == TARGET_EXPORT_NAME)
    raw = get_export_raw(pkg, exp)
    net_idx, props = parse_props(pkg, raw)

    for pr in props:
        if pr["name"] == "HueCount":
            pr["body"] = bytearray(struct.pack("<i", STACK_HUE_COUNT))
        elif pr["name"] == "ValueCount":
            pr["body"] = bytearray(struct.pack("<i", STACK_VALUE_COUNT))
        elif pr["name"] == "DefaultId":
            pr["body"] = bytearray(struct.pack("<i", 0))
        elif pr["name"] == "Colors":
            pr["body"] = bytearray(struct.pack("<i", TOTAL_COLORS) + colors_payload)
        elif pr["name"] == "DebugColors":
            pr["body"] = bytearray(struct.pack("<i", TOTAL_COLORS) + debug_payload)

    new_serial = serialize_props(net_idx, props)
    return exp, new_serial

def patch_tagame_orangeteamv2(upk_path: Path):
    print(f"[*] Reading {upk_path}...")
    pkg = up.PackageReader(str(upk_path))
    
    print("[*] Extracting BlueTeamV3 + OrangeTeamV3 + Accent swatches...")
    colors_payload, debug_payload = build_combined_swatches(pkg)
    print(f"[+] Combined swatches built: {TOTAL_COLORS} colors (Colors: {len(colors_payload)} bytes, DebugColors: {len(debug_payload)} bytes)")

    target_exp, new_serial = build_patched_orangeteamv2(pkg, colors_payload, debug_payload)
    print(f"[+] Rebuilt {TARGET_EXPORT_NAME}: {target_exp['serial_size']} -> {len(new_serial)} bytes")

    # Read original file bytes
    with open(upk_path, "rb") as f:
        file_bytes = bytearray(f.read())

    # Get last chunk
    last_idx = len(pkg.chunks) - 1
    last = pkg.chunks[last_idx]
    c_off = last["comp_offset"]
    c_sz = last["comp_size"]
    u_off = last["uncomp_offset"]
    u_sz = last["uncomp_size"]

    print(f"[*] Decompressing last chunk {last_idx} (offset={c_off}, comp_sz={c_sz}, uncomp_sz={u_sz})...")
    last_chunk_bytes = file_bytes[c_off:c_off + c_sz]
    last_uncomp = bytearray(up.decompress_chunk_data(last_chunk_bytes))
    
    new_export_offset = u_off + len(last_uncomp)
    last_uncomp.extend(new_serial)
    new_uncomp_sz = len(last_uncomp)

    print(f"[*] Recompressing last chunk ({u_sz} -> {new_uncomp_sz} bytes)...")
    new_compressed_chunk = up.compress_chunk_data(bytes(last_uncomp))
    new_comp_sz = len(new_compressed_chunk)

    # Replace last chunk in file_bytes
    file_bytes[c_off:c_off + c_sz] = new_compressed_chunk

    # Update plain header:
    # 1. Update ONLY target export (OrangeTeamV2) serial_size and serial_offset
    plain_header = bytearray(pkg.plain_header)
    exp_pos = target_exp["header_pos"]
    struct.pack_into("<i", plain_header, exp_pos + 32, len(new_serial))
    struct.pack_into("<q", plain_header, exp_pos + 36, new_export_offset)

    # 2. Update chunk table entry for last chunk
    chunks_rel = pkg.depends_offset - pkg.name_offset
    last_chunk_pos = chunks_rel + 4 + (last_idx * 36)
    struct.pack_into("<qiqi", plain_header, last_chunk_pos,
                     u_off, new_uncomp_sz,
                     c_off, new_comp_sz)

    # Encrypt plain_header from name_offset
    print("[*] Encrypting package header with AES-256 ECB...")
    enc_size = (pkg.total_hdr - pkg.name_offset + 15) & ~15
    enc_plain = bytes(plain_header)
    if len(enc_plain) < enc_size:
        enc_plain += b"\x00" * (enc_size - len(enc_plain))
    else:
        enc_plain = enc_plain[:enc_size]

    enc_header = AES.new(TAGAME_KEY, AES.MODE_ECB).encrypt(enc_plain)
    file_bytes[pkg.name_offset:pkg.name_offset + enc_size] = enc_header

    # Write output atomically
    tmp_out = upk_path.with_suffix(".upk.tmp")
    with open(tmp_out, "wb") as f:
        f.write(file_bytes)
    os.replace(tmp_out, upk_path)
    print(f"[SUCCESS] Successfully patched OrangeTeamV2 in {upk_path} (new size: {len(file_bytes)} bytes)")

if __name__ == "__main__":
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/root/velrlapi/prepatched/TAGame.upk")
    patch_tagame_orangeteamv2(target)
