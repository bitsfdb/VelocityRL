#!/usr/bin/env python3
"""
TAGame.upk Paint & Bytecode Inspector, Slot Dumper, and Conditional Patcher
Searches TAGame.upk for paint-related functions and dumps authoritative slot enum mappings.
"""

import os
import sys
import struct
import zlib
import argparse
from typing import List, Dict, Tuple, Optional

# Official AES-256 Key for TAGame.upk Header
TAGAME_KEY = bytes([
    0xc7, 0xdf, 0x6b, 0x13, 0x25, 0x2a, 0xcc, 0x71,
    0x47, 0xbb, 0x51, 0xc9, 0x8a, 0xd7, 0xe3, 0x4b,
    0x7f, 0xe5, 0x00, 0xb7, 0x7f, 0xa5, 0xfa, 0xb2,
    0x93, 0xe2, 0xf2, 0x4e, 0x6b, 0x17, 0xe7, 0x79,
])

# UE3 64-bit UnrealScript Opcodes
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
        raise RuntimeError("Please install 'cryptography' (e.g. pip install cryptography)")

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
        raise RuntimeError("Please install 'cryptography'")

class ExportEntry:
    def __init__(self, idx: int, pos: int, class_idx: int, super_idx: int, outer_idx: int,
                 name_idx: int, name: str, serial_size: int, serial_offset: int, noc: int):
        self.idx = idx
        self.pos = pos
        self.class_idx = class_idx
        self.super_idx = super_idx
        self.outer_idx = outer_idx
        self.name_idx = name_idx
        self.name = name
        self.serial_size = serial_size
        self.serial_offset = serial_offset
        self.noc = noc

class TAGameInspector:
    def __init__(self, file_path: str):
        self.file_path = file_path
        with open(file_path, "rb") as f:
            self.file_bytes = bytearray(f.read())

        self.parse_header()

    def parse_header(self):
        fb = self.file_bytes
        self.total_header_size = struct.unpack_from("<I", fb, 8)[0]
        p = 12
        flen = struct.unpack_from("<i", fb, p)[0]
        p += 4 + (flen if flen > 0 else -flen * 2)
        p += 4 # package flags
        self.name_count = struct.unpack_from("<i", fb, p)[0]
        p += 4
        self.name_offset = struct.unpack_from("<I", fb, p)[0]
        self.export_count = struct.unpack_from("<i", fb, p + 4)[0]
        self.export_offset = struct.unpack_from("<i", fb, p + 8)[0]
        self.import_count = struct.unpack_from("<i", fb, p + 12)[0]
        self.import_offset = struct.unpack_from("<i", fb, p + 16)[0]
        self.depends_offset = struct.unpack_from("<i", fb, p + 20)[0]

        enc_size = (self.total_header_size - self.name_offset + 15) & ~15
        self.enc_end = self.name_offset + enc_size
        self.plain_header = bytearray(decrypt_ecb(TAGAME_KEY, bytes(fb[self.name_offset:self.enc_end])))

        # Parse Name Table
        self.names = []
        n_pos = 0
        for _ in range(self.name_count):
            if n_pos + 4 > len(self.plain_header):
                break
            slen = struct.unpack_from("<i", self.plain_header, n_pos)[0]
            n_pos += 4
            if slen > 0:
                s = self.plain_header[n_pos:n_pos + slen - 1].decode("latin-1", errors="replace")
                n_pos += slen + 8
            elif slen < 0:
                u_bytes = (-slen) * 2
                s = self.plain_header[n_pos:n_pos + u_bytes - 2].decode("utf-16le", errors="replace")
                n_pos += u_bytes + 8
            else:
                s = ""
                n_pos += 8
            self.names.append(s)

        # Parse Export Table
        self.exports: List[ExportEntry] = []
        exp_rel = self.export_offset - self.name_offset
        dep_rel = self.depends_offset - self.name_offset
        pos = exp_rel
        while pos + 72 <= dep_rel and pos + 72 <= len(self.plain_header) and len(self.exports) < self.export_count:
            class_idx, super_idx, outer_idx, name_idx, _ = struct.unpack_from("<iiiii", self.plain_header, pos)
            serial_size = struct.unpack_from("<i", self.plain_header, pos + 32)[0]
            serial_offset = struct.unpack_from("<q", self.plain_header, pos + 36)[0]
            noc = struct.unpack_from("<i", self.plain_header, pos + 48)[0]

            name_str = self.names[name_idx] if 0 <= name_idx < len(self.names) else f"Name_{name_idx}"
            self.exports.append(ExportEntry(
                idx=len(self.exports) + 1,
                pos=pos,
                class_idx=class_idx,
                super_idx=super_idx,
                outer_idx=outer_idx,
                name_idx=name_idx,
                name=name_str,
                serial_size=serial_size,
                serial_offset=serial_offset,
                noc=noc
            ))
            pos += 72 + max(0, noc) * 4

        # Parse Chunk Table
        c_pos = dep_rel
        chunk_count = struct.unpack_from("<i", self.plain_header, c_pos)[0]
        c_pos += 4
        self.chunks = []
        for _ in range(chunk_count):
            if c_pos + 36 > len(self.plain_header):
                break
            u_off, u_sz, c_off, c_sz = struct.unpack_from("<qiqq", self.plain_header, c_pos)
            self.chunks.append({
                "pos": c_pos,
                "uncomp_offset": u_off,
                "uncomp_size": u_sz,
                "comp_offset": c_off,
                "comp_size": c_sz,
            })
            c_pos += 36

    def decompress_chunk(self, chunk_idx: int = 0) -> bytearray:
        ch = self.chunks[chunk_idx]
        c_off = ch["comp_offset"]
        c_sz = ch["comp_size"]
        b_csz = struct.unpack_from("<i", self.file_bytes, c_off + 16)[0]
        compressed_data = bytes(self.file_bytes[c_off + 24:c_off + 24 + b_csz])
        return bytearray(zlib.decompress(compressed_data))

    def search_exports(self, pattern: str) -> List[ExportEntry]:
        pat = pattern.lower()
        return [e for e in self.exports if pat in e.name.lower()]

    def search_names(self, pattern: str) -> List[Tuple[int, str]]:
        pat = pattern.lower()
        return [(i, n) for i, n in enumerate(self.names) if pat in n.lower()]

    def dump_slots(self):
        print("\n=======================================================")
        print("          TAGAME.UPK AUTHORITATIVE SLOT MAPPING        ")
        print("=======================================================\n")

        # 1. Search Name Table for Slot patterns
        slot_names = self.search_names("Slot_")
        if not slot_names:
            slot_names = self.search_names("Slot")

        print(f"[+] Found {len(slot_names)} slot-related name entries in Name Table:")
        for idx, name in slot_names:
            print(f"  Name #{idx:05d}: {name}")

        # 2. Search for Enum exports
        enum_exports = [e for e in self.exports if "slot" in e.name.lower() or "eslot" in e.name.lower()]
        print(f"\n[+] Found {len(enum_exports)} slot-related Export entries:")
        decomp0 = None
        for e in enum_exports:
            print(f"\n  Export #{e.idx:05d}: '{e.name}' (Serial size: {e.serial_size}, Offset: 0x{e.serial_offset:X})")
            if e.serial_size > 0:
                ch0 = self.chunks[0]
                if ch0["uncomp_offset"] <= e.serial_offset < ch0["uncomp_offset"] + ch0["uncomp_size"]:
                    if decomp0 is None:
                        decomp0 = self.decompress_chunk(0)
                    off0 = e.serial_offset - ch0["uncomp_offset"]
                    payload = decomp0[off0:off0 + e.serial_size]
                    print(f"    Raw bytes: {payload[:32].hex()}...")
                    
                    # Try reading TArray<FName>
                    # In UE3 UEnum: offset 0 or after standard field header
                    for probe in range(0, min(len(payload) - 4, 32), 4):
                        cnt = struct.unpack_from("<i", payload, probe)[0]
                        if 5 <= cnt <= 30 and probe + 4 + cnt * 8 <= len(payload):
                            print(f"    [!] Decoded Enum Variant Array (Count = {cnt}):")
                            for v_idx in range(cnt):
                                n_idx = struct.unpack_from("<i", payload, probe + 4 + v_idx * 8)[0]
                                n_str = self.names[n_idx] if 0 <= n_idx < len(self.names) else f"Unknown_{n_idx}"
                                print(f"      Slot Index {v_idx:02d} (0x{v_idx:02X}) = '{n_str}'")
                            break

        # 3. Dump ClientLoadoutData struct members
        cld_exports = [e for e in self.exports if "clientloadoutdata" in e.name.lower()]
        print(f"\n[+] ClientLoadoutData_TA Struct Exports:")
        for e in cld_exports:
            print(f"  Export #{e.idx:05d}: '{e.name}'")

    def disassemble_bytecode(self, data: bytes) -> str:
        lines = []
        i = 0
        while i < len(data):
            op = data[i]
            if op == EX_LET:
                lines.append(f"  0x{i:04X}: EX_LET")
                i += 1
            elif op == EX_STRUCT_MEMBER:
                prop_idx, struct_idx = struct.unpack_from("<ii", data, i + 1)
                p_name = self.names[prop_idx] if 0 <= prop_idx < len(self.names) else str(prop_idx)
                s_name = self.names[struct_idx] if 0 <= struct_idx < len(self.names) else str(struct_idx)
                lines.append(f"  0x{i:04X}: EX_STRUCT_MEMBER {p_name} ({struct_idx})")
                i += 11
            elif op == EX_INSTANCE_VARIABLE:
                var_idx = struct.unpack_from("<i", data, i + 1)[0]
                v_name = self.names[var_idx] if 0 <= var_idx < len(self.names) else str(var_idx)
                lines.append(f"  0x{i:04X}: EX_INSTANCE_VARIABLE {v_name} ({var_idx})")
                i += 9
            elif op == EX_LOCAL_VARIABLE:
                var_idx = struct.unpack_from("<i", data, i + 1)[0]
                v_name = self.names[var_idx] if 0 <= var_idx < len(self.names) else str(var_idx)
                lines.append(f"  0x{i:04X}: EX_LOCAL_VARIABLE {v_name} ({var_idx})")
                i += 9
            elif op == EX_DYN_ARRAY_OP:
                lines.append(f"  0x{i:04X}: EX_DYN_ARRAY_OP")
                i += 3
            elif op == EX_INT_ZERO:
                lines.append(f"  0x{i:04X}: EX_INT_ZERO (0)")
                i += 1
            elif op == EX_INT_CONST_BYTE:
                b = data[i + 1]
                lines.append(f"  0x{i:04X}: EX_INT_CONST_BYTE {b}")
                i += 2
            elif op == EX_INT_CONST:
                val = struct.unpack_from("<i", data, i + 1)[0]
                lines.append(f"  0x{i:04X}: EX_INT_CONST {val}")
                i += 5
            elif op == EX_JUMP_IF_NOT:
                target = struct.unpack_from("<H", data, i + 1)[0]
                lines.append(f"  0x{i:04X}: EX_JUMP_IF_NOT -> mem:0x{target:04X}")
                i += 3
            elif op == EX_EQUAL_EQUAL_INT_INT:
                lines.append(f"  0x{i:04X}: EX_EQUAL_EQUAL_INT_INT (==)")
                i += 1
            elif op == EX_END_FUNCTION_PARMS:
                lines.append(f"  0x{i:04X}: EX_END_FUNCTION_PARMS")
                i += 1
            elif op == EX_RETURN:
                lines.append(f"  0x{i:04X}: EX_RETURN")
                i += 1
            elif op == EX_END_OF_SCRIPT:
                lines.append(f"  0x{i:04X}: EX_END_OF_SCRIPT")
                i += 1
                break
            elif op == EX_NOTHING:
                i += 1
            else:
                lines.append(f"  0x{i:04X}: 0x{op:02X}")
                i += 1
        return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="TAGame.upk Inspector & Slot Dumper")
    parser.add_argument("tagame_path", nargs="?", default="", help="Path to TAGame.upk")
    parser.add_argument("--slots", action="store_true", help="Dump all slot enum mappings from TAGame.upk")
    parser.add_argument("--search", "-s", default="", help="Search pattern for exports")
    parser.add_argument("--decompile", "-d", help="Export name to decompile bytecode for")
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

    print(f"[*] Reading and decrypting header from: {tagame_path}")
    inspector = TAGameInspector(tagame_path)
    print(f"[*] Loaded {len(inspector.names)} Names, {len(inspector.exports)} Exports, {len(inspector.chunks)} Chunks.")

    if args.slots or (not args.search and not args.decompile):
        inspector.dump_slots()
        return

    if args.decompile:
        matches = [e for e in inspector.exports if e.name.lower() == args.decompile.lower()]
        if not matches:
            print(f"[-] Export '{args.decompile}' not found.")
            return
        target = matches[0]
        print(f"\n[+] Decompiling Export #{target.idx}: {target.name} (Serial size: {target.serial_size}, Offset: 0x{target.serial_offset:X})")
        decomp0 = inspector.decompress_chunk(0)
        func_bytes = decomp0[target.serial_offset:target.serial_offset + target.serial_size]
        print(f"[+] Raw Hex: {func_bytes.hex()}")
        print("[+] Bytecode Disassembly:")
        print(inspector.disassemble_bytecode(func_bytes[48:] if len(func_bytes) > 48 else func_bytes))
        return

    if args.search:
        print(f"\n[+] Searching exports matching '{args.search}':")
        results = inspector.search_exports(args.search)
        print(f"Found {len(results)} matches:")
        for r in results[:50]:
            print(f"  #{r.idx:05d} | Offset: 0x{r.serial_offset:08X} | Size: {r.serial_size:05d} | {r.name}")

if __name__ == "__main__":
    main()
