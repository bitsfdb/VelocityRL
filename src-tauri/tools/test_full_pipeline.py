#!/usr/bin/env python3
import sys
import os
import shutil
import struct
sys.path.insert(0, '/root/VelocityRL/src-tauri/tools')
from test_conditional_features import build_test_conditional_paint_bytecode, build_test_explosion_setproduct_bytecode
from validate_tagame import validate_tagame, decompress_chunk, TAGAME_KEY
from Crypto.Cipher import AES
import zlib

def test_full_pipeline(src_upk="/root/velrlapi/.cache/TAGame.upk", dst_upk="/tmp/TAGame_test_pipeline.upk"):
    print(f"[*] Copying {src_upk} -> {dst_upk}...")
    shutil.copyfile(src_upk, dst_upk)

    with open(dst_upk, "rb") as f:
        data = bytearray(f.read())

    tot_header_size = struct.unpack("<I", data[8:12])[0]
    p = 12
    flen = struct.unpack("<i", data[p:p+4])[0]
    p += 4 + (flen if flen > 0 else -flen*2)
    pkg_flags, name_count, name_offset = struct.unpack("<III", data[p:p+12])
    p += 12
    export_count, export_offset, import_count, import_offset, depends_offset = struct.unpack("<IIIII", data[p:p+20])
    enc_size = (tot_header_size - name_offset + 15) & ~15
    enc_end = name_offset + enc_size
    plain_header = bytearray(AES.new(TAGAME_KEY, AES.MODE_ECB).decrypt(data[name_offset:enc_end]))

    chunks_rel = depends_offset - name_offset
    num_chunks = struct.unpack("<I", plain_header[chunks_rel:chunks_rel+4])[0]
    chunks = []
    c_p = chunks_rel + 4
    for i in range(num_chunks):
        u_off, u_sz, c_off, c_sz = struct.unpack("<qiqi", plain_header[c_p:c_p+24])
        chunks.append({
            "idx": i,
            "uncomp_offset": u_off,
            "uncomp_size": u_sz,
            "comp_offset": c_off,
            "comp_size": c_sz
        })
        c_p += 36

    pos = export_offset - name_offset
    names = []
    # read names
    n_p = 0
    for _ in range(name_count):
        l = struct.unpack("<i", plain_header[n_p:n_p+4])[0]
        n_p += 4
        if l > 0:
            names.append(plain_header[n_p:n_p+l].split(b"\x00")[0].decode("latin1"))
            n_p += l
        else:
            names.append(plain_header[n_p:n_p+-l*2].decode("utf-16le").split("\x00")[0])
            n_p += -l*2
        n_p += 8

    exports = []
    while pos + 72 <= chunks_rel and len(exports) < export_count:
        class_idx = struct.unpack("<i", plain_header[pos:pos+4])[0]
        outer_idx = struct.unpack("<i", plain_header[pos+8:pos+12])[0]
        name_idx = struct.unpack("<i", plain_header[pos+12:pos+16])[0]
        serial_size = struct.unpack("<i", plain_header[pos+32:pos+36])[0]
        serial_offset = struct.unpack("<q", plain_header[pos+36:pos+44])[0]
        noc = struct.unpack("<i", plain_header[pos+48:pos+52])[0]
        nm = names[name_idx] if 0 <= name_idx < len(names) else ""
        exports.append({
            "idx": len(exports) + 1,
            "name": nm,
            "outer_idx": outer_idx,
            "outer_name": "",
            "serial_size": serial_size,
            "serial_offset": serial_offset
        })
        pos += 72 + max(0, noc) * 4

    for e in exports:
        if 0 < e["outer_idx"] <= len(exports):
            e["outer_name"] = exports[e["outer_idx"] - 1]["name"]

    # 1. Patch Chunk 0: CarMeshComponentBase_TA::ApplyPaintSettings
    print("[*] Patching Chunk 0 with conditional custom paint...")
    c0 = chunks[0]
    c0_bytes = data[c0["comp_offset"]:c0["comp_offset"]+c0["comp_size"]]
    decomp0 = bytearray(decompress_chunk(c0_bytes))

    cmc_aps = next(e for e in exports if e["name"] == "ApplyPaintSettings" and e["outer_name"] == "CarMeshComponentBase_TA")
    aps_off = cmc_aps["serial_offset"] - c0["uncomp_offset"]
    aps_disk = struct.unpack("<I", decomp0[aps_off+44:aps_off+48])[0]
    paint_payload = build_test_conditional_paint_bytecode(r=2.5, g=0.0, b=1.0, max_sz=aps_disk)
    decomp0[aps_off+48:aps_off+48+aps_disk] = paint_payload

    # Recompress Chunk 0
    def compress_chunk_ue3(decomp):
        block_sz = 131072
        num_blocks = (len(decomp) + block_sz - 1) // block_sz
        comp_blocks = []
        for i in range(num_blocks):
            start = i * block_sz
            end = min(start + block_sz, len(decomp))
            part = decomp[start:end]
            c_part = zlib.compress(part, level=9)
            comp_blocks.append((c_part, len(part)))
        tot_c = sum(len(cb) for cb, _ in comp_blocks)
        out = bytearray(struct.pack("<IIII", 0x9E2A83C1, block_sz, tot_c, len(decomp)))
        for cb, u_len in comp_blocks:
            out.extend(struct.pack("<ii", len(cb), u_len))
        for cb, _ in comp_blocks:
            out.extend(cb)
        return bytes(out)

    recomp0 = compress_chunk_ue3(bytes(decomp0))
    if len(recomp0) > c0["comp_size"]:
        raise ValueError(f"Recompressed Chunk 0 ({len(recomp0)}) exceeds allocation ({c0['comp_size']})")
    padded0 = bytearray(recomp0)
    padded0.extend(bytes(c0["comp_size"] - len(recomp0)))
    data[c0["comp_offset"]:c0["comp_offset"]+c0["comp_size"]] = padded0
    print(f"[+] Chunk 0 recompressed: {len(recomp0)}/{c0['comp_size']} bytes.")

    # 2. Patch Chunk 1: ExplosionPreviewer_TA::SetProduct
    print("[*] Patching Chunk 1 with goal explosion previewer hook...")
    c1 = chunks[1]
    c1_bytes = data[c1["comp_offset"]:c1["comp_offset"]+c1["comp_size"]]
    decomp1 = bytearray(decompress_chunk(c1_bytes))

    ep_sp = next(e for e in exports if e["name"] == "SetProduct" and e["outer_name"] == "ExplosionPreviewer_TA")
    ep_off = ep_sp["serial_offset"] - c1["uncomp_offset"]
    ep_disk = struct.unpack("<I", decomp1[ep_off+44:ep_off+48])[0]
    exp_payload = build_test_explosion_setproduct_bytecode(owned=2044, target=4001, max_sz=ep_disk)
    decomp1[ep_off+48:ep_off+48+ep_disk] = exp_payload

    recomp1 = compress_chunk_ue3(bytes(decomp1))
    if len(recomp1) > c1["comp_size"]:
        raise ValueError(f"Recompressed Chunk 1 ({len(recomp1)}) exceeds allocation ({c1['comp_size']})")
    padded1 = bytearray(recomp1)
    padded1.extend(bytes(c1["comp_size"] - len(recomp1)))
    data[c1["comp_offset"]:c1["comp_offset"]+c1["comp_size"]] = padded1
    print(f"[+] Chunk 1 recompressed: {len(recomp1)}/{c1['comp_size']} bytes.")

    # Write patched UPK
    with open(dst_upk, "wb") as f:
        f.write(data)

    print("\n[*] Validating patched UPK with validate_tagame...")
    success = validate_tagame(dst_upk)
    if success:
        print("\n[✓] FULL PIPELINE TEST PASSED! The patched UPK is 100% valid and verified.")
    else:
        print("\n[✗] Validation failed!")
    return success

if __name__ == '__main__':
    res = test_full_pipeline()
    sys.exit(0 if res else 1)
