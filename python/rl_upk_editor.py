#!/usr/bin/env python3
import argparse
import base64
import concurrent.futures
import ctypes
import hashlib
import io
import os
import struct
import sys
import threading
import traceback
import zlib
from dataclasses import dataclass, field
import re
import zipfile
from pathlib import Path
from typing import BinaryIO, Dict, List, Optional, Tuple

try:
    import tkinter as tk
    from tkinter import filedialog, messagebox, simpledialog, ttk
    HAS_GUI = True
except Exception:
    HAS_GUI = False
    class _TkStub:
        def __getattr__(self, n): return self
        def __call__(self, *a, **kw): return self
        def __bool__(self): return False
        def __iter__(self): return iter([])
        def pack(self, *a, **kw): return self
        def grid(self, *a, **kw): return self
        def configure(self, *a, **kw): return self
    tk = filedialog = messagebox = simpledialog = ttk = _TkStub()

from cryptography.hazmat.backends import default_backend
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

PACKAGE_FILE_TAG = 0x9E2A83C1
COMPRESS_NONE = 0x00
COMPRESS_ZLIB = 0x01
PKG_COOKED = 0x00000008
DEFAULT_KEY = bytes([
    0xC7, 0xDF, 0x6B, 0x13, 0x25, 0x2A, 0xCC, 0x71,
    0x47, 0xBB, 0x51, 0xC9, 0x8A, 0xD7, 0xE3, 0x4B,
    0x7F, 0xE5, 0x00, 0xB7, 0x7F, 0xA5, 0xFA, 0xB2,
    0x93, 0xE2, 0xF2, 0x4E, 0x6B, 0x17, 0xE7, 0x79,
])
HEX_PREVIEW_LIMIT = 65536
COMPACT_INDEX_DEPRECATED = 178
NUMBER_ADDED_TO_NAME = 343
ENUM_NAME_ADDED_TO_BYTE_PROPERTY_TAG = 633
BOOL_VALUE_TO_BYTE_FOR_BOOL_PROPERTY_TAG = 673


class BinaryReader:
    def __init__(self, fh: BinaryIO):
        self.fh = fh

    def tell(self) -> int:
        return self.fh.tell()

    def seek(self, offset: int, whence: int = os.SEEK_SET) -> int:
        return self.fh.seek(offset, whence)

    def read_exact(self, size: int) -> bytes:
        data = self.fh.read(size)
        if len(data) != size:
            raise EOFError(f"Expected {size} bytes, got {len(data)}")
        return data

    def read_i32(self) -> int:
        return struct.unpack("<i", self.read_exact(4))[0]

    def read_u32(self) -> int:
        return struct.unpack("<I", self.read_exact(4))[0]

    def read_u64(self) -> int:
        return struct.unpack("<Q", self.read_exact(8))[0]

    def read_u16(self) -> int:
        return struct.unpack("<H", self.read_exact(2))[0]

    def read_i64(self) -> int:
        return struct.unpack("<q", self.read_exact(8))[0]

    def read_u8(self) -> int:
        return struct.unpack("<B", self.read_exact(1))[0]

    def read_i8(self) -> int:
        return struct.unpack("<b", self.read_exact(1))[0]

    def read_f32(self) -> float:
        return struct.unpack("<f", self.read_exact(4))[0]

    def remaining(self) -> int:
        cur = self.tell()
        self.seek(0, os.SEEK_END)
        end = self.tell()
        self.seek(cur)
        return end - cur

    def read_fstring(self) -> str:
        length = self.read_i32()
        if length == 0:
            return ""
        if length < 0:
            char_count = -length
            raw = self.read_exact(char_count * 2)
            return raw[:-2].decode("utf-16-le", errors="ignore")
        raw = self.read_exact(length - 1)
        self.read_exact(1)
        # UE3 positive length strings are ANSI/Windows-1252, not UTF-8
        return raw.decode("windows-1252", errors="ignore")


@dataclass
class FNameRef:
    name_index: int
    instance_number: int


@dataclass
class NameEntry:
    index: int
    name: str
    flags: int


@dataclass
class ImportEntry:
    table_index: int
    class_package: FNameRef
    class_name: FNameRef
    outer_index: int
    object_name: FNameRef


@dataclass
class ExportEntry:
    table_index: int
    class_index: int
    super_index: int
    outer_index: int
    object_name: FNameRef
    archetype_index: int
    object_flags: int
    serial_size: int
    serial_offset: int
    export_flags: int
    net_objects: List[int]
    package_guid: Tuple[int, int, int, int]
    package_flags: int


@dataclass
class FCompressedChunk:
    uncompressed_offset: int
    uncompressed_size: int
    compressed_offset: int
    compressed_size: int


@dataclass
class FileSummary:
    tag: int = 0
    file_version: int = 0
    licensee_version: int = 0
    total_header_size: int = 0
    folder_name: str = ""
    package_flags_flags_offset: int = 0
    package_flags: int = 0
    name_count: int = 0
    name_offset: int = 0
    export_count: int = 0
    export_offset: int = 0
    import_count: int = 0
    import_offset: int = 0
    depends_offset: int = 0
    import_export_guids_offset: int = 0
    import_guids_count: int = 0
    export_guids_count: int = 0
    thumbnail_table_offset: int = 0
    guid: Tuple[int, int, int, int] = (0, 0, 0, 0)
    generations: List[Tuple[int, int, int]] = field(default_factory=list)
    engine_version: int = 0
    cooker_version: int = 0
    compression_flags_offset: int = 0
    compression_flags: int = 0
    compressed_chunks: List[FCompressedChunk] = field(default_factory=list)


@dataclass
class FileCompressionMetaData:
    garbage_size: int
    compressed_chunks_offset: int
    last_block_size: int


@dataclass
class ParsedPackage:
    file_path: Path
    summary: FileSummary
    names: List[NameEntry]
    imports: List[ImportEntry]
    exports: List[ExportEntry]
    file_bytes: bytes

    def object_data(self, export: ExportEntry) -> bytes:
        start = export.serial_offset
        end = start + export.serial_size
        if start < 0 or end < start or end > len(self.file_bytes):
            return b""
        return self.file_bytes[start:end]

    def resolve_name(self, ref: FNameRef) -> str:
        if 0 <= ref.name_index < len(self.names):
            base = self.names[ref.name_index].name
        else:
            base = f"<Name#{ref.name_index}>"
        return f"{base}_{ref.instance_number}" if ref.instance_number > 0 else base

    def resolve_object_ref(self, index: int) -> str:
        if index == 0:
            return "None"
        if index > 0:
            export_index = index - 1
            if 0 <= export_index < len(self.exports):
                exp = self.exports[export_index]
                return f"Export[{export_index}] {self.resolve_name(exp.object_name)}"
            return f"Export[{export_index}] <invalid>"
        import_index = -index - 1
        if 0 <= import_index < len(self.imports):
            imp = self.imports[import_index]
            return f"Import[{import_index}] {self.resolve_name(imp.object_name)}"
        return f"Import[{import_index}] <invalid>"

    def resolve_object_path(self, index: int, seen: Optional[set] = None) -> str:
        if index == 0:
            return "None"
        if seen is None:
            seen = set()
        if index in seen:
            return "<cycle>"
        seen.add(index)
        if index > 0:
            exp = self.exports[index - 1]
            name = self.resolve_name(exp.object_name)
            if exp.outer_index == 0:
                return name
            return f"{self.resolve_object_path(exp.outer_index, seen)}.{name}"
        imp = self.imports[-index - 1]
        name = self.resolve_name(imp.object_name)
        if imp.outer_index == 0:
            return name
        return f"{self.resolve_object_path(imp.outer_index, seen)}.{name}"

    def export_class_name(self, export: ExportEntry) -> str:
        if export.class_index == 0:
            return "Class"
        if export.class_index > 0:
            target = self.exports[export.class_index - 1]
            return self.resolve_name(target.object_name)
        target = self.imports[-export.class_index - 1]
        return self.resolve_name(target.object_name)

    def is_placeholder_export(self, export: ExportEntry) -> bool:
        # An export is a placeholder/garbage slot if its class is the meta
        # 'Class' (class_index == 0), its name resolves to literal 'None'
        # (name table index 0 in UE3), it has no outer, no serial body, and
        # no flags set. UE Explorer filters these out of its class list using
        # essentially the same predicate (ClassIndex == 0 && Name == 'None').
        # We additionally require zero size/offset/flags to avoid false
        # positives on rare native objects whose class index is 0.
        if export.class_index != 0:
            return False
        name = self.resolve_name(export.object_name)
        if name.lower() != "none":
            return False
        if export.outer_index != 0:
            return False
        if export.serial_size != 0 or export.serial_offset != 0:
            return False
        if export.object_flags != 0 or export.export_flags != 0:
            return False
        return True

    def resolve_export_class_candidates(self, export: ExportEntry) -> List[str]:
        raw = self.export_class_name(export)
        candidates = [raw]
        for prefix in ("A", "U", "F"):
            candidates.append(f"{prefix}{raw}")
        return candidates


@dataclass
class SDKField:
    name: str
    type_name: str
    offset: int
    size: int
    owner: str


@dataclass
class SDKType:
    name: str
    kind: str
    super_name: Optional[str]
    fields: List[SDKField] = field(default_factory=list)


@dataclass
class ParsedProperty:
    index: int
    name: str
    tag_type: str
    size: int
    array_index: int
    tag_offset: int
    value_offset: int
    value: str
    declared_type: str = "?"
    owner_type: str = "?"
    struct_name: Optional[str] = None
    enum_name: Optional[str] = None
    bool_value: Optional[bool] = None
    raw_hex: str = ""


class RLSDKDatabase:
    def __init__(self):
        self.types: Dict[str, SDKType] = {}

    def get_type(self, name: str) -> Optional[SDKType]:
        if name in self.types:
            return self.types[name]
        for candidate in (name, f"A{name}", f"U{name}", f"F{name}"):
            if candidate in self.types:
                return self.types[candidate]
        return None

    def resolve_field(self, owner_name: str, field_name: str) -> Tuple[Optional[SDKField], Optional[str]]:
        seen = set()
        cur = self.get_type(owner_name)
        while cur and cur.name not in seen:
            seen.add(cur.name)
            for field in cur.fields:
                if field.name == field_name:
                    return field, cur.name
            cur = self.get_type(cur.super_name) if cur.super_name else None
        return None, None


def parse_rlsdk_database(zip_path: Path) -> RLSDKDatabase:
    db = RLSDKDatabase()
    class_re = re.compile(r"//\s+(?:Class|ScriptStruct)\s+[^\n]+\n//[^\n]*\n(?:class|struct)\s+(\w+)(?:\s*:\s*public\s+(\w+))?\s*\{(.*?)\n\};", re.S)
    field_re = re.compile(r"^\s*(.+?)\s+(\w+)(?:\[[^\]]+\])?;\s*//\s*0x([0-9A-Fa-f]+)\s*\(0x([0-9A-Fa-f]+)\)", re.M)
    with zipfile.ZipFile(zip_path) as zf:
        for name in zf.namelist():
            if not name.endswith(("_classes.hpp", "_structs.hpp")):
                continue
            text = zf.read(name).decode("utf-8", errors="ignore")
            kind = "class" if name.endswith("_classes.hpp") else "struct"
            for m in class_re.finditer(text):
                type_name, super_name, body = m.groups()
                sdk_type = db.types.get(type_name)
                if sdk_type is None:
                    sdk_type = SDKType(name=type_name, kind=kind, super_name=super_name)
                    db.types[type_name] = sdk_type
                else:
                    sdk_type.kind = kind
                    sdk_type.super_name = super_name
                fields: List[SDKField] = []
                for fm in field_re.finditer(body):
                    type_name_raw, field_name, offset_hex, size_hex = fm.groups()
                    fields.append(SDKField(
                        name=field_name,
                        type_name=" ".join(type_name_raw.split()),
                        offset=int(offset_hex, 16),
                        size=int(size_hex, 16),
                        owner=type_name,
                    ))
                sdk_type.fields = fields
    return db


COMMON_STRUCT_DECODERS = {
    "FVector": lambda r: f"({r.read_f32():.6g}, {r.read_f32():.6g}, {r.read_f32():.6g})",
    "FVector2D": lambda r: f"({r.read_f32():.6g}, {r.read_f32():.6g})",
    "FRotator": lambda r: f"({r.read_i32()}, {r.read_i32()}, {r.read_i32()})",
    "FColor": lambda r: f"RGBA({r.read_u8()}, {r.read_u8()}, {r.read_u8()}, {r.read_u8()})",
    "FLinearColor": lambda r: f"({r.read_f32():.6g}, {r.read_f32():.6g}, {r.read_f32():.6g}, {r.read_f32():.6g})",
    "FQuat": lambda r: f"({r.read_f32():.6g}, {r.read_f32():.6g}, {r.read_f32():.6g}, {r.read_f32():.6g})",
    "FGuid": lambda r: f"{r.read_u32():08X}-{r.read_u32():08X}-{r.read_u32():08X}-{r.read_u32():08X}",
}


def parse_tarray_inner_type(type_name: str) -> Optional[str]:
    m = re.search(r"TArray<(.+)>", type_name)
    if not m:
        return None
    return " ".join(m.group(1).split())


def clean_cpp_type_name(type_name: str) -> str:
    t = type_name.replace("class ", "").replace("struct ", "").strip()
    return t.rstrip("*").strip()


def decode_name_ref(raw: bytes, package: ParsedPackage) -> str:
    if not raw:
        return ""
    bio = io.BytesIO(raw)
    r = BinaryReader(bio)
    ref = read_fname_pkg(r, package)
    return package.resolve_name(ref)


def decode_object_ref(raw: bytes, package: ParsedPackage) -> str:
    if not raw:
        return ""
    bio = io.BytesIO(raw)
    r = BinaryReader(bio)
    index = read_index_pkg(r, package)
    return f"{index} ({package.resolve_object_ref(index)})"


def decode_array_preview(raw: bytes, inner_type: Optional[str], package: ParsedPackage) -> str:
    if len(raw) < 4:
        return raw.hex(" ").upper()
    bio = io.BytesIO(raw)
    r = BinaryReader(bio)
    count = read_index_pkg(r, package)
    if count < 0:
        return f"count={count} (invalid)"
    if count == 0:
        return "count=0"
    if not inner_type:
        return f"count={count}, data={raw[4:36].hex(' ').upper()}"
    inner_clean = clean_cpp_type_name(inner_type)
    preview = []
    try:
        for _ in range(min(count, 4)):
            if inner_clean in ("int32_t", "INT", "DWORD") and r.remaining() >= 4:
                preview.append(str(r.read_i32()))
            elif inner_clean == "float" and r.remaining() >= 4:
                preview.append(f"{r.read_f32():.6g}")
            elif inner_clean in ("FName", "class FName") and r.remaining() >= 8:
                preview.append(package.resolve_name(read_fname_pkg(r, package)))
            elif inner_clean.startswith("U") and r.remaining() >= 4:
                preview.append(package.resolve_object_ref(read_index_pkg(r, package)))
            elif inner_clean in COMMON_STRUCT_DECODERS:
                preview.append(COMMON_STRUCT_DECODERS[inner_clean](r))
            else:
                break
    except Exception:
        pass
    if preview:
        return f"count={count}, preview=[{', '.join(preview)}]"
    return f"count={count}, data={raw[4:36].hex(' ').upper()}"


def decode_property_value(tag_type: str, raw: bytes, package: ParsedPackage, declared_type: str = "", struct_name: Optional[str] = None, enum_name: Optional[str] = None, bool_value: Optional[bool] = None) -> str:
    try:
        if tag_type == "BoolProperty":
            if bool_value is not None:
                return "True" if bool_value else "False"
            if raw:
                return "True" if raw[0] else "False"
            return "False"
        if tag_type == "IntProperty" and len(raw) >= 4:
            return str(struct.unpack("<i", raw[:4])[0])
        if tag_type == "FloatProperty" and len(raw) >= 4:
            return f"{struct.unpack('<f', raw[:4])[0]:.6g}"
        if tag_type in ("ObjectProperty", "ClassProperty", "ComponentProperty", "InterfaceProperty"):
            return decode_object_ref(raw, package)
        if tag_type == "NameProperty":
            return decode_name_ref(raw, package)
        if tag_type == "StrProperty":
            return BinaryReader(io.BytesIO(raw)).read_fstring()
        if tag_type == "ByteProperty":
            if enum_name and len(raw) >= 8:
                return decode_name_ref(raw, package)
            if raw:
                return str(raw[0])
        if tag_type == "StructProperty":
            if struct_name in COMMON_STRUCT_DECODERS:
                return COMMON_STRUCT_DECODERS[struct_name](BinaryReader(io.BytesIO(raw)))
            return f"{struct_name or '?'} ({len(raw)} bytes)"
        if tag_type == "ArrayProperty":
            return decode_array_preview(raw, parse_tarray_inner_type(declared_type), package)
        if tag_type == "QWordProperty" and len(raw) >= 8:
            return str(struct.unpack("<Q", raw[:8])[0])
        if tag_type == "StringRefProperty" and len(raw) >= 4:
            return str(struct.unpack("<I", raw[:4])[0])
        if tag_type == "DelegateProperty":
            if raw:
                rr = BinaryReader(io.BytesIO(raw))
                obj = read_index_pkg(rr, package)
                func = package.resolve_name(read_fname_pkg(rr, package))
                return f"obj={package.resolve_object_ref(obj)}, func={func}"
        if raw:
            return raw[:32].hex(" ").upper()
        return ""
    except Exception as exc:
        return f"<decode error: {exc}>"


VALID_PROPERTY_TYPES = {
    "ByteProperty", "IntProperty", "BoolProperty", "FloatProperty", "ObjectProperty",
    "NameProperty", "DelegateProperty", "ClassProperty", "ArrayProperty", "StructProperty",
    "VectorProperty", "RotatorProperty", "StrProperty", "MapProperty", "FixedArrayProperty",
    "InterfaceProperty", "ComponentProperty", "QWordProperty", "PointerProperty",
    "StringRefProperty", "BioMask4Property", "GuidProperty"
}


def _valid_name_ref(ref: FNameRef, package: ParsedPackage) -> bool:
    return 0 <= ref.name_index < len(package.names) and ref.instance_number >= -1


def _parse_property_tag_at(package: ParsedPackage, raw: bytes, offset: int, index: int) -> Tuple[Optional[ParsedProperty], int, bool]:
    if offset < 0 or offset + 8 > len(raw):
        return None, offset, False
    r = BinaryReader(io.BytesIO(raw))
    r.seek(offset)
    try:
        tag_offset = offset
        name_ref = read_fname_pkg(r, package)
        if not _valid_name_ref(name_ref, package):
            return None, offset, False
        name = package.resolve_name(name_ref)
        if name == "None":
            return None, r.tell(), True

        type_ref = read_fname_pkg(r, package)
        if not _valid_name_ref(type_ref, package):
            return None, offset, False
        tag_type = package.resolve_name(type_ref)
        if tag_type not in VALID_PROPERTY_TYPES:
            return None, offset, False

        size = r.read_i32()
        array_index = r.read_i32()
        if size < 0 or array_index < 0:
            return None, offset, False

        struct_name = None
        enum_name = None
        bool_value = None
        declared_type = "?"

        if tag_type == "StructProperty":
            sref = read_fname_pkg(r, package)
            if not _valid_name_ref(sref, package):
                return None, offset, False
            struct_name = package.resolve_name(sref)
            declared_type = struct_name
        elif tag_type == "ByteProperty":
            if package.summary.file_version >= ENUM_NAME_ADDED_TO_BYTE_PROPERTY_TAG:
                eref = read_fname_pkg(r, package)
                if not _valid_name_ref(eref, package):
                    return None, offset, False
                enum_name = package.resolve_name(eref)
                declared_type = enum_name or "Byte"
            else:
                declared_type = "Byte"
        elif tag_type == "ArrayProperty":
            declared_type = "TArray"
        elif tag_type == "BoolProperty":
            if package.summary.file_version >= BOOL_VALUE_TO_BYTE_FOR_BOOL_PROPERTY_TAG:
                bool_value = bool(r.read_u8())
            declared_type = "bool"
        else:
            declared_type = {
                "IntProperty": "int",
                "FloatProperty": "float",
                "ObjectProperty": "UObject*",
                "ClassProperty": "UClass*",
                "ComponentProperty": "UObject*",
                "InterfaceProperty": "UObject*",
                "NameProperty": "FName",
                "StrProperty": "FString",
                "DelegateProperty": "FScriptDelegate",
                "QWordProperty": "uint64",
                "PointerProperty": "pointer",
                "StringRefProperty": "uint32",
                "MapProperty": "TMap",
                "FixedArrayProperty": "array",
                "GuidProperty": "FGuid",
                "BioMask4Property": "BioMask4",
            }.get(tag_type, tag_type)

        value_offset = r.tell()
        if value_offset + size > len(raw):
            return None, offset, False
        value_raw = raw[value_offset:value_offset + size]
        value = decode_property_value(tag_type, value_raw, package, declared_type, struct_name, enum_name, bool_value)
        prop = ParsedProperty(
            index=index,
            name=name,
            tag_type=tag_type,
            size=size,
            array_index=array_index,
            tag_offset=tag_offset,
            value_offset=value_offset,
            value=value,
            declared_type=declared_type,
            owner_type="SerializedTag",
            struct_name=struct_name,
            enum_name=enum_name,
            bool_value=bool_value,
            raw_hex=value_raw[:64].hex(" ").upper(),
        )
        return prop, value_offset + size, False
    except Exception:
        return None, offset, False


def _try_parse_property_stream(package: ParsedPackage, raw: bytes, start_offset: int) -> Tuple[int, List[ParsedProperty], bool]:
    props: List[ParsedProperty] = []
    offset = start_offset
    seen = set()
    ended = False
    for i in range(4096):
        if offset in seen:
            break
        seen.add(offset)
        prop, next_offset, hit_end = _parse_property_tag_at(package, raw, offset, i)
        if hit_end:
            ended = True
            offset = next_offset
            break
        if prop is None:
            break
        props.append(prop)
        offset = next_offset
    return offset, props, ended


def _find_best_property_stream_offset(package: ParsedPackage, raw: bytes, class_type: Optional[SDKType] = None, sdk_db: Optional[RLSDKDatabase] = None) -> Tuple[int, List[ParsedProperty]]:
    del class_type, sdk_db
    if len(raw) < 24:
        return 0, []

    best_offset = 0
    best_props: List[ParsedProperty] = []
    best_score = -1
    max_scan = max(0, len(raw) - 24)
    for start in range(max_scan + 1):
        name_index = struct.unpack_from('<i', raw, start)[0]
        if not (0 <= name_index < len(package.names)):
            continue
        end_off, props, ended = _try_parse_property_stream(package, raw, start)
        if not props:
            continue
        score = len(props) * 1000
        if ended:
            score += 250
        score += min(end_off - start, 512)
        score -= start
        if score > best_score:
            best_score = score
            best_offset = start
            best_props = props
    return best_offset, best_props


def parse_serialized_properties(package: ParsedPackage, export: ExportEntry, sdk_db: Optional[RLSDKDatabase]) -> List[ParsedProperty]:
    del sdk_db
    raw = package.object_data(export)
    if not raw:
        return []
    _, props = _find_best_property_stream_offset(package, raw, None, None)
    return props

class DecryptionProvider:
    def __init__(self, key_file_path: Optional[str] = None):
        if key_file_path is None:
            self.decryption_keys = [DEFAULT_KEY]
        else:
            if not os.path.exists(key_file_path):
                raise FileNotFoundError(f"Failed to load the key file: {key_file_path}")
            with open(key_file_path, "r", encoding="utf-8") as fh:
                self.decryption_keys = [
                    base64.b64decode(line.strip())
                    for line in fh
                    if line.strip()
                ]

    @staticmethod
    def decrypt_ecb(key: bytes, data: bytes) -> bytes:
        cipher = Cipher(algorithms.AES(key), modes.ECB(), backend=default_backend())
        decryptor = cipher.decryptor()
        return decryptor.update(data) + decryptor.finalize()

    @staticmethod
    def encrypt_ecb(key: bytes, data: bytes) -> bytes:
        cipher = Cipher(algorithms.AES(key), modes.ECB(), backend=default_backend())
        encryptor = cipher.encryptor()
        return encryptor.update(data) + encryptor.finalize()


def find_valid_key(encrypted_path: Path, provider: DecryptionProvider) -> Tuple[FileSummary, FileCompressionMetaData, bytes, bytes]:
    with encrypted_path.open("rb") as src:
        summary = parse_file_summary(src)
        meta = parse_file_compression_metadata(src)
        encrypted_size = summary.total_header_size - meta.garbage_size - summary.name_offset
        if encrypted_size < 0:
            raise ValueError(
                f"Computed encrypted region size is negative ({encrypted_size}). "
                f"summary.total_header_size={summary.total_header_size}, "
                f"meta.garbage_size={meta.garbage_size}, "
                f"summary.name_offset={summary.name_offset}. "
                f"This usually indicates a corrupted or already-edited package header."
            )
        encrypted_size = (encrypted_size + 15) & ~15
        src.seek(summary.name_offset)
        encrypted_data = src.read(encrypted_size)
        if len(encrypted_data) != encrypted_size:
            raise ValueError(
                f"Failed to read encrypted region: expected {encrypted_size} bytes "
                f"at offset {summary.name_offset}, got {len(encrypted_data)} (file truncated?)"
            )
    for key in provider.decryption_keys:
        if verify_decryptor(summary, meta, key, encrypted_data):
            return summary, meta, encrypted_data, key
    raise ValueError("Unknown Decryption key")


def serialize_rl_chunk_table(chunks: List[FCompressedChunk]) -> bytes:
    out = bytearray()
    out += struct.pack("<i", len(chunks))
    for chunk in chunks:
        out += struct.pack("<q", chunk.uncompressed_offset)
        out += struct.pack("<i", chunk.uncompressed_size)
        out += struct.pack("<q", chunk.compressed_offset)
        out += struct.pack("<i", chunk.compressed_size)
    return bytes(out)


def compress_chunk_payload(uncompressed: bytes, block_size: int = 0x20000, level: int = 6) -> bytes:
    out = bytearray()
    out += struct.pack("<I", PACKAGE_FILE_TAG)
    out += struct.pack("<i", block_size)
    blocks = []
    total_compressed = 0
    for i in range(0, len(uncompressed), block_size):
        piece = uncompressed[i:i + block_size]
        comp = zlib.compress(piece, level)
        blocks.append((comp, len(piece)))
        total_compressed += len(comp)
    out += struct.pack("<ii", total_compressed, len(uncompressed))
    for comp, uncomp_size in blocks:
        out += struct.pack("<ii", len(comp), uncomp_size)
    for comp, _ in blocks:
        out += comp
    return bytes(out)


def _find_file_compression_metadata_offsets(stream: BinaryIO) -> Dict[str, int]:
    parse_file_summary(stream)
    meta_offset = stream.tell()
    r = BinaryReader(stream)
    garbage_size_offset = meta_offset
    r.read_i32()
    compressed_chunks_offset_offset = stream.tell()
    r.read_i32()
    last_block_size_offset = stream.tell()
    r.read_i32()
    return {
        "meta_offset": meta_offset,
        "garbage_size_offset": garbage_size_offset,
        "compressed_chunks_offset_offset": compressed_chunks_offset_offset,
        "last_block_size_offset": last_block_size_offset,
    }


def find_key_for_encrypted_upk(encrypted_path: Path, provider: DecryptionProvider) -> bytes:
    """Return the first key from *provider* that successfully decrypts *encrypted_path*.

    Raises ValueError if no key in the provider works.
    """
    _, _, _, key = find_valid_key(encrypted_path, provider)
    return key


def build_reencrypted_package(original_encrypted_path: Path, modified_decrypted_bytes: bytes, provider: DecryptionProvider, output_path: Path, *, override_key: Optional[bytes] = None) -> Path:
    summary, meta, original_encrypted_data, valid_key = find_valid_key(original_encrypted_path, provider)
    # If the caller wants to encrypt with a different key (e.g. sourced from a
    # donor encrypted UPK), use that key for the output instead of the key that
    # was used to decrypt the original package.
    if override_key is not None:
        valid_key = override_key
    modified_summary = parse_file_summary(io.BytesIO(modified_decrypted_bytes))
    original_plain = bytearray(DecryptionProvider.decrypt_ecb(valid_key, original_encrypted_data))
    original_chunks = parse_rl_compressed_chunks(bytes(original_plain), meta.compressed_chunks_offset)
    if not original_chunks:
        raise ValueError("No compressed chunks were found in original encrypted header")

    new_chunk_table_offset = modified_summary.depends_offset - modified_summary.name_offset
    patch_limit = max(0, new_chunk_table_offset)
    chunk_shift = modified_summary.depends_offset - original_chunks[0].uncompressed_offset

    rebuilt_chunks: List[FCompressedChunk] = []
    rebuilt_chunk_payloads: List[bytes] = []
    chunk_table_placeholder = serialize_rl_chunk_table([
        FCompressedChunk(0, 0, 0, 0) for _ in original_chunks
    ])
    required_plain_len = new_chunk_table_offset + len(chunk_table_placeholder)
    encrypted_plain_len = (required_plain_len + 15) & ~15
    header_plain = bytearray(encrypted_plain_len)
    copy_len = min(len(original_plain), encrypted_plain_len)
    header_plain[:copy_len] = original_plain[:copy_len]

    new_total_header_size = modified_summary.name_offset + encrypted_plain_len + meta.garbage_size
    current_compressed_offset = new_total_header_size
    for i, chunk in enumerate(original_chunks):
        start = chunk.uncompressed_offset + chunk_shift
        if i + 1 < len(original_chunks):
            end = original_chunks[i + 1].uncompressed_offset + chunk_shift
            if end > len(modified_decrypted_bytes):
                raise ValueError("Modified decrypted package changed size too early for the rebuilt chunk layout")
        else:
            end = len(modified_decrypted_bytes)
        if end < start:
            raise ValueError("Invalid rebuilt chunk bounds")
        payload = compress_chunk_payload(modified_decrypted_bytes[start:end])
        rebuilt_chunk_payloads.append(payload)
        rebuilt_chunks.append(FCompressedChunk(
            uncompressed_offset=start,
            uncompressed_size=end - start,
            compressed_offset=current_compressed_offset,
            compressed_size=len(payload),
        ))
        current_compressed_offset += len(payload)

    if patch_limit > len(header_plain):
        raise ValueError("Modified decrypted header exceeds encrypted header capacity")
    if patch_limit > 0:
        header_plain[:patch_limit] = modified_decrypted_bytes[summary.name_offset:modified_summary.depends_offset]

    chunk_table = serialize_rl_chunk_table(rebuilt_chunks)
    table_end = new_chunk_table_offset + len(chunk_table)
    if table_end > len(header_plain):
        raise ValueError("Rebuilt compressed chunk table does not fit inside encrypted header")
    header_plain[new_chunk_table_offset:table_end] = chunk_table
    encrypted_header = DecryptionProvider.encrypt_ecb(valid_key, bytes(header_plain))

    original_bytes = Path(original_encrypted_path).read_bytes()
    prefix = bytearray(original_bytes[:summary.name_offset])
    summary_offsets = _find_summary_offsets(modified_decrypted_bytes)
    patch_i32_le(prefix, summary_offsets["total_header_size_offset"], new_total_header_size)
    patch_i32_le(prefix, summary_offsets["name_count_offset"], modified_summary.name_count)
    patch_i32_le(prefix, summary_offsets["name_offset_offset"], modified_summary.name_offset)
    patch_i32_le(prefix, summary_offsets["export_count_offset"], modified_summary.export_count)
    patch_i32_le(prefix, summary_offsets["export_offset_offset"], modified_summary.export_offset)
    patch_i32_le(prefix, summary_offsets["import_count_offset"], modified_summary.import_count)
    patch_i32_le(prefix, summary_offsets["import_offset_offset"], modified_summary.import_offset)
    patch_i32_le(prefix, summary_offsets["depends_offset_offset"], modified_summary.depends_offset)
    patch_i32_le(prefix, summary_offsets["import_export_guids_offset_offset"], modified_summary.import_export_guids_offset)
    if "thumbnail_table_offset_offset" in summary_offsets:
        patch_i32_le(prefix, summary_offsets["thumbnail_table_offset_offset"], modified_summary.thumbnail_table_offset)
    _patch_generation_counts(prefix, summary_offsets, modified_summary.export_count, modified_summary.name_count)
    with original_encrypted_path.open("rb") as src:
        meta_offsets = _find_file_compression_metadata_offsets(src)
    patch_i32_le(prefix, meta_offsets["compressed_chunks_offset_offset"], new_chunk_table_offset)
    if rebuilt_chunks:
        patch_i32_le(prefix, meta_offsets["last_block_size_offset"], rebuilt_chunks[-1].uncompressed_size)

    output = bytearray()
    output += prefix
    output += encrypted_header
    gap_start = modified_summary.name_offset + len(encrypted_header)
    original_gap_start = summary.name_offset + len(original_encrypted_data)
    original_gap_end = original_chunks[0].compressed_offset
    gap_bytes = original_bytes[original_gap_start:original_gap_end]
    if len(gap_bytes) != meta.garbage_size:
        gap_bytes = original_bytes[original_gap_end - meta.garbage_size:original_gap_end]
    output += gap_bytes
    for payload in rebuilt_chunk_payloads:
        output += payload

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(output)
    return output_path


def _pack_fname_value(package: ParsedPackage, text: str) -> bytes:
    text = text.strip()
    # Allow either "#<index>" to pick a name table entry by raw index, or a
    # plain base/base_<N> string to match by name. Instance suffixes are
    # split off so users can write things like "Foo_3" and have it round-trip
    # through the version-aware serialize_fname adjustment.
    base_text, instance_number = _split_name_instance(text)
    match = None
    if base_text.startswith("#"):
        try:
            idx = int(base_text[1:])
            if 0 <= idx < len(package.names):
                match = package.names[idx]
        except Exception:
            pass
    if match is None:
        for entry in package.names:
            if entry.name == base_text:
                match = entry
                break
    if match is None:
        # Fall back to the original full string (for the legacy case where a
        # name literally contained '_<digits>').
        for entry in package.names:
            if entry.name == text:
                match = entry
                instance_number = 0
                break
    if match is None:
        raise ValueError(f"FName not found in package name table: {text}")
    # instance_number == 0 from _split_name_instance means "no suffix typed",
    # which corresponds to in-memory -1 for >= NUMBER_ADDED_TO_NAME packages
    # (so serialize_fname writes 0 on disk). Translate appropriately.
    if package.summary.file_version >= NUMBER_ADDED_TO_NAME and instance_number == 0:
        in_memory_instance = -1
    else:
        in_memory_instance = instance_number
    return serialize_fname(FNameRef(match.index, in_memory_instance), package.summary)


def _parse_struct_numbers(text: str) -> List[float]:
    parts = [p.strip() for p in text.replace("(", "").replace(")", "").split(",") if p.strip()]
    return [float(p) for p in parts]



def get_export_entry_offsets(package: ParsedPackage) -> List[int]:
    bio = io.BytesIO(package.file_bytes)
    bio.seek(package.summary.export_offset)
    r = BinaryReader(bio)
    offsets: List[int] = []
    generation_count = len(package.summary.generations)
    for _ in range(package.summary.export_count):
        offsets.append(bio.tell())
        parse_export_entry(r, len(offsets) - 1, generation_count, package.summary)
    return offsets


def patch_i32_le(data: bytearray, offset: int, value: int) -> None:
    data[offset:offset + 4] = struct.pack("<i", value)


def patch_i64_le(data: bytearray, offset: int, value: int) -> None:
    data[offset:offset + 8] = struct.pack("<q", value)


def apply_property_edit_bytes(package: ParsedPackage, export: ExportEntry, prop: ParsedProperty, text: str) -> bytes:
    rel_offset, replacement = encode_property_value(package, prop, text)
    abs_offset = export.serial_offset + rel_offset
    size_delta = len(replacement) - prop.size
    target_offset = prop.value_offset - 1 if prop.tag_type == "BoolProperty" and prop.bool_value is not None else prop.value_offset

    if size_delta == 0:
        data = bytearray(package.file_bytes)
        data[abs_offset:abs_offset + len(replacement)] = replacement
        return bytes(data)

    if prop.tag_type != "StrProperty":
        raise ValueError("Variable-size edits are currently supported for StrProperty only")

    if export.serial_offset < 0 or export.serial_size < 0:
        raise ValueError("Invalid export serial bounds")
    export_end = export.serial_offset + export.serial_size
    if export_end > len(package.file_bytes):
        raise ValueError("Export serial data exceeds package size")

    value_abs_offset = export.serial_offset + target_offset
    tag_size_abs_offset = export.serial_offset + prop.tag_offset + 16

    new_data = bytearray()
    new_data += package.file_bytes[:value_abs_offset]
    new_data += replacement
    new_data += package.file_bytes[value_abs_offset + prop.size:]

    export_entry_offsets = get_export_entry_offsets(package)
    if export.table_index >= len(export_entry_offsets):
        raise ValueError("Export table index out of range")

    entry_offset = export_entry_offsets[export.table_index]
    patch_i32_le(new_data, tag_size_abs_offset, len(replacement))
    patch_i32_le(new_data, entry_offset + 32, export.serial_size + size_delta)

    export_shift_point = export.serial_offset
    for idx, other in enumerate(package.exports):
        if idx == export.table_index:
            continue
        if other.serial_offset > export_shift_point:
            other_entry_offset = export_entry_offsets[idx]
            patch_i64_le(new_data, other_entry_offset + 36, other.serial_offset + size_delta)

    return bytes(new_data)


def encode_property_value(package: ParsedPackage, prop: ParsedProperty, text: str) -> Tuple[int, bytes]:
    text = text.strip()
    if text.lower().startswith("hex:"):
        raw = bytes.fromhex(text[4:].strip())
        target_offset = prop.value_offset - 1 if prop.tag_type == "BoolProperty" and prop.bool_value is not None else prop.value_offset
        expected = 1 if prop.tag_type == "BoolProperty" and prop.bool_value is not None else prop.size
        if len(raw) != expected:
            raise ValueError(f"hex payload must be exactly {expected} bytes")
        return target_offset, raw
    if prop.tag_type == "BoolProperty":
        v = text.lower()
        if v in ("1", "true", "yes", "on"):
            return prop.value_offset - 1, b""
        if v in ("0", "false", "no", "off"):
            return prop.value_offset - 1, b"\x00"
        raise ValueError("BoolProperty expects true/false")
    if prop.tag_type == "IntProperty":
        return prop.value_offset, struct.pack("<i", int(text, 0))
    if prop.tag_type == "FloatProperty":
        return prop.value_offset, struct.pack("<f", float(text))
    if prop.tag_type == "QWordProperty":
        return prop.value_offset, struct.pack("<Q", int(text, 0))
    if prop.tag_type == "StringRefProperty":
        return prop.value_offset, struct.pack("<I", int(text, 0))
    if prop.tag_type in ("ObjectProperty", "ClassProperty", "ComponentProperty", "InterfaceProperty"):
        resolved = resolve_object_index_by_text(package, text)
        if resolved is None:
            raise ValueError("Object reference not found in exports/imports; use an index like -12 or a full object path")
        return prop.value_offset, struct.pack("<i", resolved)
    if prop.tag_type == "NameProperty":
        return prop.value_offset, _pack_fname_value(package, text)
    if prop.tag_type == "ByteProperty":
        if prop.enum_name:
            return prop.value_offset, _pack_fname_value(package, text)
        return prop.value_offset, struct.pack("<B", int(text, 0) & 0xFF)
    if prop.tag_type == "StrProperty":
        encoded = write_fstring_bytes(text)
        return prop.value_offset, encoded
    if prop.tag_type == "StructProperty":
        if prop.struct_name == "FVector":
            vals = _parse_struct_numbers(text)
            if len(vals) != 3:
                raise ValueError("FVector expects x,y,z")
            return prop.value_offset, struct.pack("<fff", *vals)
        if prop.struct_name == "FVector2D":
            vals = _parse_struct_numbers(text)
            if len(vals) != 2:
                raise ValueError("FVector2D expects x,y")
            return prop.value_offset, struct.pack("<ff", *vals)
        if prop.struct_name == "FRotator":
            vals = [int(v) for v in _parse_struct_numbers(text)]
            if len(vals) != 3:
                raise ValueError("FRotator expects pitch,yaw,roll")
            return prop.value_offset, struct.pack("<iii", *vals)
        if prop.struct_name == "FColor":
            vals = [int(v) for v in _parse_struct_numbers(text)]
            if len(vals) != 4:
                raise ValueError("FColor expects r,g,b,a")
            return prop.value_offset, bytes(v & 0xFF for v in vals)
        if prop.struct_name == "FLinearColor":
            vals = _parse_struct_numbers(text)
            if len(vals) != 4:
                raise ValueError("FLinearColor expects r,g,b,a")
            return prop.value_offset, struct.pack("<ffff", *vals)
        if prop.struct_name == "FGuid":
            cleaned = text.replace('-', '').replace('{', '').replace('}', '').strip()
            if len(cleaned) != 32:
                raise ValueError("FGuid expects 32 hex digits or a dashed guid")
            vals = [int(cleaned[i:i+8], 16) for i in range(0, 32, 8)]
            return prop.value_offset, struct.pack("<IIII", *vals)
    raise ValueError(f"Editing is not implemented for {prop.tag_type}")

def write_fstring_bytes(text: str) -> bytes:
    if not text:
        return struct.pack('<i', 0)
    try:
        # If it's pure ASCII, serialize as 1-byte ANSI (positive length)
        encoded = text.encode('ascii') + b'\x00'
        return struct.pack('<i', len(encoded)) + encoded
    except UnicodeEncodeError:
        # If it contains non-ASCII characters, serialize as 2-byte UTF-16LE (negative length)
        encoded = text.encode('utf-16-le') + b'\x00\x00'
        char_count = len(text) + 1
        return struct.pack('<i', -char_count) + encoded

def serialize_fname(ref: FNameRef, summary: Optional["FileSummary"] = None) -> bytes:
    # Mirror the version-aware adjustment in read_fname: for >= NUMBER_ADDED_TO_NAME
    # the on-disk stored value is (instance_number + 1), so we add 1 here.
    # When summary is None (legacy callers), fall back to writing the raw
    # in-memory value, which preserves prior behaviour for any code path
    # that hasn't been threaded through.
    if summary is not None and summary.file_version >= NUMBER_ADDED_TO_NAME:
        stored_instance = ref.instance_number + 1
    else:
        stored_instance = ref.instance_number
    return struct.pack("<ii", ref.name_index, stored_instance)


def serialize_name_entry(entry: NameEntry) -> bytes:
    return write_fstring_bytes(entry.name) + struct.pack("<Q", entry.flags)


def serialize_import_entry(entry: ImportEntry, summary: Optional["FileSummary"] = None) -> bytes:
    return b"".join([
        serialize_fname(entry.class_package, summary),
        serialize_fname(entry.class_name, summary),
        struct.pack("<i", entry.outer_index),
        serialize_fname(entry.object_name, summary),
    ])


def serialize_export_entry(entry: ExportEntry, summary: Optional["FileSummary"] = None) -> bytes:
    out = bytearray()
    out += struct.pack("<i", entry.class_index)
    out += struct.pack("<i", entry.super_index)
    out += struct.pack("<i", entry.outer_index)
    out += serialize_fname(entry.object_name, summary)
    out += struct.pack("<i", entry.archetype_index)
    out += struct.pack("<Q", entry.object_flags)
    out += struct.pack("<i", entry.serial_size)
    out += struct.pack("<q", entry.serial_offset)
    out += struct.pack("<i", entry.export_flags)
    out += struct.pack("<i", len(entry.net_objects))
    for value in entry.net_objects:
        out += struct.pack("<i", value)
    out += struct.pack("<IIII", *entry.package_guid)
    out += struct.pack("<i", entry.package_flags)
    return bytes(out)


def _find_summary_offsets(data: bytes) -> Dict[str, int]:
    bio = io.BytesIO(data)
    r = BinaryReader(bio)
    if r.read_u32() != PACKAGE_FILE_TAG:
        raise ValueError("Not a valid Unreal Engine package")
    r.read_u16()
    r.read_u16()
    total_header_size_offset = bio.tell()
    r.read_i32()
    r.read_fstring()
    package_flags_offset = bio.tell()
    r.read_u32()
    name_count_offset = bio.tell()
    r.read_i32()
    name_offset_offset = bio.tell()
    r.read_i32()
    export_count_offset = bio.tell()
    r.read_i32()
    export_offset_offset = bio.tell()
    r.read_i32()
    import_count_offset = bio.tell()
    r.read_i32()
    import_offset_offset = bio.tell()
    r.read_i32()
    depends_offset_offset = bio.tell()
    r.read_i32()
    import_export_guids_offset_offset = bio.tell()
    r.read_i32()
    r.read_i32()
    r.read_i32()
    thumbnail_table_offset_offset = bio.tell()
    r.read_i32()
    read_guid(r)
    generations_count_offset = bio.tell()
    gen_count = r.read_i32()
    generation_entries_offset = bio.tell()
    return {
        "total_header_size_offset": total_header_size_offset,
        "package_flags_offset": package_flags_offset,
        "name_count_offset": name_count_offset,
        "name_offset_offset": name_offset_offset,
        "export_count_offset": export_count_offset,
        "export_offset_offset": export_offset_offset,
        "import_count_offset": import_count_offset,
        "import_offset_offset": import_offset_offset,
        "depends_offset_offset": depends_offset_offset,
        "import_export_guids_offset_offset": import_export_guids_offset_offset,
        "thumbnail_table_offset_offset": thumbnail_table_offset_offset,
        "generations_count_offset": generations_count_offset,
        "generation_entries_offset": generation_entries_offset,
        "generation_count": gen_count,
    }


def _patch_generation_counts(data: bytearray, offsets: Dict[str, int], export_count: int, name_count: int) -> None:
    gen_count = offsets.get("generation_count", 0)
    if gen_count <= 0:
        return
    base = offsets["generation_entries_offset"] + (gen_count - 1) * 12
    if base + 8 > len(data):
        return
    patch_i32_le(data, base, export_count)
    patch_i32_le(data, base + 4, name_count)


def _replace_header_tables(package: ParsedPackage, names: List[NameEntry], imports: List[ImportEntry]) -> bytes:
    summary = package.summary
    offsets = _find_summary_offsets(package.file_bytes)
    old_depends_offset = summary.depends_offset

    prefix = bytearray(package.file_bytes[:summary.name_offset])
    patched_exports: List[ExportEntry] = []
    for x in package.exports:
        patched_exports.append(ExportEntry(
            table_index=x.table_index,
            class_index=x.class_index,
            super_index=x.super_index,
            outer_index=x.outer_index,
            object_name=FNameRef(x.object_name.name_index, x.object_name.instance_number),
            archetype_index=x.archetype_index,
            object_flags=x.object_flags,
            serial_size=x.serial_size,
            serial_offset=x.serial_offset,
            export_flags=x.export_flags,
            net_objects=list(x.net_objects),
            package_guid=x.package_guid,
            package_flags=x.package_flags,
        ))

    names_blob = b"".join(serialize_name_entry(x) for x in names)
    imports_blob = b"".join(serialize_import_entry(x, summary) for x in imports)
    export_offset = summary.name_offset + len(names_blob) + len(imports_blob)
    depends_offset = export_offset + sum(len(serialize_export_entry(x, summary)) for x in patched_exports)
    delta = depends_offset - old_depends_offset

    if delta != 0:
        for exp in patched_exports:
            if exp.serial_offset >= old_depends_offset:
                exp.serial_offset += delta

    exports_blob = b"".join(serialize_export_entry(x, summary) for x in patched_exports)
    depends_offset = export_offset + len(exports_blob)
    delta = depends_offset - old_depends_offset

    header_blob = prefix + names_blob + imports_blob + exports_blob
    patch_i32_le(header_blob, offsets["name_count_offset"], len(names))
    patch_i32_le(header_blob, offsets["name_offset_offset"], summary.name_offset)
    patch_i32_le(header_blob, offsets["export_count_offset"], len(patched_exports))
    patch_i32_le(header_blob, offsets["export_offset_offset"], export_offset)
    patch_i32_le(header_blob, offsets["import_count_offset"], len(imports))
    patch_i32_le(header_blob, offsets["import_offset_offset"], summary.name_offset + len(names_blob))
    patch_i32_le(header_blob, offsets["depends_offset_offset"], depends_offset)

    import_export_guids_offset = summary.import_export_guids_offset
    if import_export_guids_offset >= old_depends_offset and import_export_guids_offset != 0:
        import_export_guids_offset += delta
    patch_i32_le(header_blob, offsets["import_export_guids_offset_offset"], import_export_guids_offset)

    thumbnail_table_offset = summary.thumbnail_table_offset
    if thumbnail_table_offset >= old_depends_offset and thumbnail_table_offset != 0:
        thumbnail_table_offset += delta
    if "thumbnail_table_offset_offset" in offsets:
        patch_i32_le(header_blob, offsets["thumbnail_table_offset_offset"], thumbnail_table_offset)

    # NOTE: total_header_size is intentionally written back UNCHANGED. In a
    # decrypted RL package this field carries over the value from the original
    # encrypted file (unpack_package copies the encrypted prefix verbatim into
    # the decrypted output and never adjusts this field). The encrypted-save
    # path (build_reencrypted_package) computes its own correct value from
    # name_offset + encrypted_plain_len + garbage_size and patches it
    # independently, so the value we write here only matters for
    # 'Save Decrypted UPK' where preserving the original-encrypted semantics
    # is the right behaviour. An earlier attempt to "fix" this by adding the
    # names+imports growth delta produced corrupt encrypted files because the
    # delta concept doesn't apply to the encrypted-layout meaning of this field.
    patch_i32_le(header_blob, offsets["total_header_size_offset"], summary.total_header_size)
    _patch_generation_counts(header_blob, offsets, len(patched_exports), len(names))

    new_data = bytearray()
    new_data += header_blob
    new_data += package.file_bytes[old_depends_offset:]
    return bytes(new_data)


def _split_name_instance(text: str) -> Tuple[str, int]:
    if '_' in text:
        base, suffix = text.rsplit('_', 1)
        if suffix.isdigit():
            return base, int(suffix)
    return text, 0


def _find_existing_name_ref(names: List[NameEntry], text: str) -> Optional[FNameRef]:
    base, instance = _split_name_instance(text)
    for entry in names:
        if entry.name == base:
            return FNameRef(entry.index, instance)
    return None


def _ensure_name_entry(names: List[NameEntry], text: str, flags: int = 0) -> FNameRef:
    found = _find_existing_name_ref(names, text)
    if found is not None:
        return found
    base, instance = _split_name_instance(text)
    names.append(NameEntry(index=len(names), name=base, flags=flags))
    return FNameRef(len(names) - 1, instance)


def import_donor_names(package: ParsedPackage, donor_package: ParsedPackage, selected_names: Optional[List[str]] = None) -> ParsedPackage:
    names = [NameEntry(index=n.index, name=n.name, flags=n.flags) for n in package.names]
    wanted = None if not selected_names else set(selected_names)
    added = 0
    for entry in donor_package.names:
        if wanted is not None and entry.name not in wanted:
            continue
        if _find_existing_name_ref(names, entry.name) is None:
            names.append(NameEntry(index=len(names), name=entry.name, flags=entry.flags))
            added += 1
    if added == 0:
        result = ParsedPackage(package.file_path, package.summary, names, package.imports, package.exports, package.file_bytes)
        setattr(result, '_merge_added_names', 0)
        return result
    patched = _replace_header_tables(package, names, package.imports)
    temp_path = package.file_path.with_name(package.file_path.stem + '_names_merged.upk')
    temp_path.write_bytes(patched)
    result = parse_decrypted_package(temp_path)
    setattr(result, '_merge_added_names', added)
    return result


def _collect_existing_import_paths(package: ParsedPackage) -> Dict[str, int]:
    out: Dict[str, int] = {}
    for i in range(len(package.imports)):
        out[package.resolve_object_path(-(i + 1))] = -(i + 1)
    return out


def _class_package_and_name_for_ref(package: ParsedPackage, class_index: int) -> Tuple[str, str]:
    if class_index == 0:
        return "Core", "Class"
    path = package.resolve_object_path(class_index)
    parts = [p for p in path.split('.') if p and p != 'None']
    if not parts:
        return "Core", "Class"
    if len(parts) == 1:
        return "Core", parts[-1]
    return parts[-2], parts[-1]


def _derive_donor_package_name(donor_package: ParsedPackage, override: Optional[str] = None) -> str:
    """Return the package name UE will use to LoadPackage the donor at runtime.

    Priority:
      1. Explicit override from the caller (e.g. user typed it in).
      2. The donor file's stem (e.g. 'MyDonorAssets.upk' -> 'MyDonorAssets').
         This is what the engine resolves through its package search paths,
         so it must match how the file is actually deployed in the game's
         cooked content directory.
      3. The donor's embedded summary.folder_name. Often empty in cooked
         RL packages but used as a last resort.

    Raises ValueError if no usable name can be derived.
    """
    if override and override.strip():
        return override.strip()
    stem = donor_package.file_path.stem
    # Strip our own '_decrypted' / '_decompressed' suffixes that resolve_input_package
    # appends when it produces a working copy - the file the game loads has the
    # original stem.
    for suffix in ("_decrypted", "_decompressed"):
        if stem.endswith(suffix):
            stem = stem[: -len(suffix)]
            break
    if stem:
        return stem
    folder = (donor_package.summary.folder_name or "").strip()
    if folder:
        return folder
    raise ValueError("Could not determine donor package name; pass it explicitly")


def merge_donor_exports_as_imports(target_package: ParsedPackage, donor_package: ParsedPackage, donor_package_name: Optional[str] = None) -> ParsedPackage:
    # The donor's package name is what the engine will look up at runtime to
    # locate and LoadPackage the donor .upk. Every donor export we re-import
    # MUST be rooted under a Core.Package import with this name, otherwise
    # the engine has no way to know which file to open to resolve the
    # reference. Previously donor root exports were imported with
    # outer_index=0 (i.e. as if they themselves were top-level packages),
    # which left the engine unable to resolve them.
    resolved_donor_name = _derive_donor_package_name(donor_package, donor_package_name)

    names = [NameEntry(index=n.index, name=n.name, flags=n.flags) for n in target_package.names]
    imports = [ImportEntry(table_index=i, class_package=FNameRef(x.class_package.name_index, x.class_package.instance_number), class_name=FNameRef(x.class_name.name_index, x.class_name.instance_number), outer_index=x.outer_index, object_name=FNameRef(x.object_name.name_index, x.object_name.instance_number)) for i, x in enumerate(target_package.imports)]
    existing_paths = _collect_existing_import_paths(target_package)
    donor_cache: Dict[int, int] = {}

    def ensure_package_root(package_name: str) -> int:
        existing = existing_paths.get(package_name)
        if existing is not None:
            return existing
        cp = _ensure_name_entry(names, 'Core')
        cn = _ensure_name_entry(names, 'Package')
        on = _ensure_name_entry(names, package_name)
        imports.append(ImportEntry(len(imports), cp, cn, 0, on))
        idx = -len(imports)
        existing_paths[package_name] = idx
        return idx

    # Pre-create the donor package import up front. Even if no donor exports
    # ended up needing it (e.g. all collisions with existing imports), having
    # this entry guarantees the engine will attempt to load the donor file
    # when the target is loaded, which is what users typically want when
    # they "import donor exports".
    donor_root_index = ensure_package_root(resolved_donor_name)

    def ensure_donor_object(index: int) -> int:
        if index == 0:
            return 0
        if index in donor_cache:
            return donor_cache[index]
        path = donor_package.resolve_object_path(index)
        # When matching against existing target imports, prepend the donor
        # package name so a donor export "Foo" doesn't collide with an
        # unrelated existing import literally named "Foo". For donor
        # imports we keep the original path because those refer to the same
        # external packages (Engine, Core, etc.) the target may also
        # reference, and we WANT to share those.
        scoped_path = f"{resolved_donor_name}.{path}" if index > 0 else path
        if scoped_path in existing_paths:
            donor_cache[index] = existing_paths[scoped_path]
            return existing_paths[scoped_path]
        if index > 0:
            obj = donor_package.exports[index - 1]
            obj_name = donor_package.resolve_name(obj.object_name)
            outer_index = ensure_donor_object(obj.outer_index) if obj.outer_index else 0
            if outer_index == 0:
                # Root donor export: parent it to the donor package import so
                # the engine knows to LoadPackage(donor_name) to resolve it.
                outer_index = donor_root_index
            class_pkg_name, class_name_name = _class_package_and_name_for_ref(donor_package, obj.class_index)
        else:
            obj = donor_package.imports[-index - 1]
            obj_name = donor_package.resolve_name(obj.object_name)
            outer_index = ensure_donor_object(obj.outer_index) if obj.outer_index else 0
            class_pkg_name = donor_package.resolve_name(obj.class_package)
            class_name_name = donor_package.resolve_name(obj.class_name)
        cp = _ensure_name_entry(names, class_pkg_name)
        cn = _ensure_name_entry(names, class_name_name)
        on = _ensure_name_entry(names, obj_name)
        imports.append(ImportEntry(len(imports), cp, cn, outer_index, on))
        new_index = -len(imports)
        donor_cache[index] = new_index
        existing_paths[scoped_path] = new_index
        return new_index

    imported = 0
    for i in range(1, len(donor_package.exports) + 1):
        before = len(imports)
        ensure_donor_object(i)
        if len(imports) != before:
            imported += 1

    patched = _replace_header_tables(target_package, names, imports)
    result = parse_decrypted_package_bytes(target_package.file_path, patched)
    setattr(result, '_merge_added_imports', len(imports) - len(target_package.imports))
    setattr(result, '_merge_added_names', len(names) - len(target_package.names))
    setattr(result, '_merge_donor_export_count', len(donor_package.exports))
    setattr(result, '_merge_donor_package_name', resolved_donor_name)
    return result



def replace_export_with_donor_export(target_package: ParsedPackage, donor_package: ParsedPackage, target_export_path: str, donor_export_path: str) -> ParsedPackage:
    merged = import_donor_names(target_package, donor_package, None)
    merged = merge_donor_exports_as_imports(merged, donor_package)

    target_index = resolve_object_index_by_text(merged, target_export_path)
    donor_index = resolve_object_index_by_text(donor_package, donor_export_path)
    if target_index is None or target_index <= 0:
        raise ValueError(f"Target export not found: {target_export_path}")
    if donor_index is None or donor_index <= 0:
        raise ValueError(f"Donor export not found: {donor_export_path}")

    target_export = merged.exports[target_index - 1]
    donor_export = donor_package.exports[donor_index - 1]

    target_class = merged.export_class_name(target_export)
    donor_class = donor_package.export_class_name(donor_export)
    if target_class != donor_class:
        raise ValueError(f"Class mismatch: target is {target_class}, donor is {donor_class}")

    donor_bytes = donor_package.object_data(donor_export)
    if not donor_bytes:
        raise ValueError("Donor export has no serial data")

    size_delta = len(donor_bytes) - target_export.serial_size
    new_data = bytearray()
    new_data += merged.file_bytes[:target_export.serial_offset]
    new_data += donor_bytes
    new_data += merged.file_bytes[target_export.serial_offset + target_export.serial_size:]

    export_entry_offsets = get_export_entry_offsets(merged)
    entry_offset = export_entry_offsets[target_index - 1]
    patch_i32_le(new_data, entry_offset + 32, len(donor_bytes))

    for idx, other in enumerate(merged.exports):
        if idx == target_index - 1:
            continue
        if other.serial_offset > target_export.serial_offset:
            other_entry_offset = export_entry_offsets[idx]
            patch_i64_le(new_data, other_entry_offset + 36, other.serial_offset + size_delta)

    result = parse_decrypted_package_bytes(merged.file_path, bytes(new_data))
    setattr(result, '_replace_target_export_path', target_export_path)
    setattr(result, '_replace_donor_export_path', donor_export_path)
    setattr(result, '_replace_note', 'Raw donor serial data copied into target export. Name/index remapping inside arbitrary native data is not performed.')
    return result


def rename_export_fname(package: ParsedPackage, export: ExportEntry, new_name_text: str) -> ParsedPackage:
    """Rename the FName (object_name) of a single export.

    Accepts either a bare base name ("MyName") or a base+instance form ("MyName_3").
    If the base name already exists in the package's name table, the export entry
    is patched in place (8 bytes at object_name field). If the base name is new,
    it is appended to the name table via _replace_header_tables, and the export's
    FName field is then patched to point at the newly added name.
    """
    new_name_text = (new_name_text or "").strip()
    if not new_name_text:
        raise ValueError("Empty FName")
    base, instance = _split_name_instance(new_name_text)
    if not base:
        raise ValueError("Empty base name")
    if instance < 0:
        raise ValueError("Instance number must be >= 0")

    # Locate where this export's entry sits inside the export table so we can
    # patch the 8-byte object_name field in place. Layout of an export entry:
    #   class_index (i32) | super_index (i32) | outer_index (i32) |
    #   object_name (i32 name_index + i32 instance_number) | ...
    # so object_name starts at entry_offset + 12.
    export_entry_offsets = get_export_entry_offsets(package)
    if export.table_index < 0 or export.table_index >= len(export_entry_offsets):
        raise ValueError("Export table index out of range")
    fname_field_abs = export_entry_offsets[export.table_index] + 12

    # The user-typed instance number is the "displayed" value (0 for no
    # suffix, 3 for _3, etc.). On disk for >= NUMBER_ADDED_TO_NAME the
    # stored value is (instance + 1), so we add 1 here. For older versions
    # the stored value equals the displayed value.
    if package.summary.file_version >= NUMBER_ADDED_TO_NAME:
        stored_instance = instance + 1 if instance > 0 else 0
    else:
        stored_instance = instance

    # Try to reuse an existing name first.
    existing_idx: Optional[int] = None
    for entry in package.names:
        if entry.name == base:
            existing_idx = entry.index
            break

    if existing_idx is not None:
        # Fast path: in-place 8-byte patch of the export entry's FName field.
        new_data = bytearray(package.file_bytes)
        new_data[fname_field_abs:fname_field_abs + 8] = struct.pack("<ii", existing_idx, stored_instance)
        result = parse_decrypted_package_bytes(package.file_path, bytes(new_data))
        setattr(result, '_rename_added_names', 0)
        setattr(result, '_rename_new_name', new_name_text)
        setattr(result, '_rename_export_index', export.table_index)
        return result

    # Slow path: append a new name entry and rebuild the header tables. The
    # rebuild may shift the depends_offset and any export serial_offsets that
    # come after it, but the entries inside the export table itself stay at the
    # same relative positions because _replace_header_tables preserves their
    # order. So entry_offsets of the rebuilt file equal new_export_offset +
    # (old_offset - old_export_offset).
    names = [NameEntry(index=n.index, name=n.name, flags=n.flags) for n in package.names]
    new_entry_index = len(names)
    names.append(NameEntry(index=new_entry_index, name=base, flags=0))

    rebuilt_bytes = bytearray(_replace_header_tables(package, names, package.imports))

    # Recompute export entry offsets in the rebuilt file (their positions can
    # shift because the names table grew) and patch the FName field.
    rebuilt_pkg = parse_decrypted_package_bytes(package.file_path, bytes(rebuilt_bytes))
    new_offsets = get_export_entry_offsets(rebuilt_pkg)
    if export.table_index >= len(new_offsets):
        raise ValueError("Export entry not present after header rebuild")
    new_fname_field_abs = new_offsets[export.table_index] + 12
    rebuilt_bytes[new_fname_field_abs:new_fname_field_abs + 8] = struct.pack("<ii", new_entry_index, stored_instance)

    result = parse_decrypted_package_bytes(package.file_path, bytes(rebuilt_bytes))
    setattr(result, '_rename_added_names', 1)
    setattr(result, '_rename_new_name', new_name_text)
    setattr(result, '_rename_export_index', export.table_index)
    return result


def rename_name_entry(package: ParsedPackage, name_index: int, new_text: str) -> ParsedPackage:
    """Rewrite the text of a single entry in the package's name table.

    The name's *index* stays the same, only the string changes. Because every
    FNameRef across the package (exports, imports, serialized property tags,
    object names, etc.) references names by index, all references continue to
    resolve correctly and now read the new text.

    The names blob length almost always changes (different string length, or
    ANSI vs UTF-16 encoding), so the entire header is rebuilt via
    _replace_header_tables. That helper recomputes name_offset-following
    offsets (import/export/depends/thumbnail/import-export-guids) and shifts
    every export.serial_offset by the resulting delta, so the package stays
    internally consistent. total_header_size is preserved by the rebuild
    helper's offset patching - the bytes after the header are taken verbatim
    from the original file at old_depends_offset.

    Args:
        package: The package to modify.
        name_index: Zero-based index into package.names of the entry to rename.
        new_text: New text for the name entry. Must be a bare base name (no
            "_<N>" instance suffix - instance numbers live on FNameRefs, not
            on name table entries).

    Raises:
        ValueError: If name_index is out of range, new_text is empty, new_text
            contains an instance suffix, or new_text already exists elsewhere
            in the name table (which would create a duplicate base name and
            ambiguous lookups).

    Returns:
        A re-parsed ParsedPackage with attributes:
            _name_rename_index: int  - index that was renamed
            _name_rename_old: str    - previous text
            _name_rename_new: str    - new text
            _name_size_delta: int    - bytes added (+) or removed (-) by the
                                       rename, useful for status reporting
    """
    new_text = (new_text or "").strip()
    if not new_text:
        raise ValueError("Empty name text")
    if name_index < 0 or name_index >= len(package.names):
        raise ValueError(f"Name index {name_index} out of range (0..{len(package.names) - 1})")

    # Reject instance suffixes - those belong on FNameRefs, not entries. A
    # name table entry is a pure base string; entries like "Foo_3" only happen
    # if the original asset really had a literal underscore-digit base name.
    base, instance = _split_name_instance(new_text)
    if instance != 0:
        raise ValueError(
            "Name entries cannot include an instance suffix like '_3'. "
            "Instance numbers live on each FName reference, not on the "
            "name table entry. Use the bare base name (e.g. 'MyName')."
        )

    old_entry = package.names[name_index]
    if old_entry.name == new_text:
        # No-op: re-parse so callers always get a fresh ParsedPackage with the
        # standard rename metadata attached.
        result = parse_decrypted_package_bytes(package.file_path, bytes(package.file_bytes))
        setattr(result, '_name_rename_index', name_index)
        setattr(result, '_name_rename_old', old_entry.name)
        setattr(result, '_name_rename_new', new_text)
        setattr(result, '_name_size_delta', 0)
        return result

    # Reject collisions with other entries. Merging duplicates would require
    # remapping every FNameRef across exports/imports/serialized properties to
    # the surviving index, which is a much larger operation than a rename.
    for entry in package.names:
        if entry.index == name_index:
            continue
        if entry.name == new_text:
            raise ValueError(
                f"Name '{new_text}' already exists at index {entry.index}. "
                "Renaming would create a duplicate base name. Choose a unique "
                "name, or rename the other entry first."
            )

    # Build the modified names list and let _replace_header_tables redo the
    # names blob, recompute downstream offsets, and shift export serial_offsets
    # by the size delta.
    old_blob_len = len(serialize_name_entry(old_entry))
    new_entry = NameEntry(index=name_index, name=new_text, flags=old_entry.flags)
    new_blob_len = len(serialize_name_entry(new_entry))
    size_delta = new_blob_len - old_blob_len

    names = [NameEntry(index=n.index, name=n.name, flags=n.flags) for n in package.names]
    names[name_index] = new_entry

    rebuilt_bytes = _replace_header_tables(package, names, package.imports)
    result = parse_decrypted_package_bytes(package.file_path, rebuilt_bytes)
    setattr(result, '_name_rename_index', name_index)
    setattr(result, '_name_rename_old', old_entry.name)
    setattr(result, '_name_rename_new', new_text)
    setattr(result, '_name_size_delta', size_delta)
    return result


def resolve_object_index_by_text(package: ParsedPackage, text: str) -> Optional[int]:
    text = text.strip()
    try:
        return int(text, 0)
    except Exception:
        pass
    if text.startswith('Import[') or text.startswith('Export['):
        m = re.match(r'^(Import|Export)\[(\d+)\]', text)
        if m:
            kind, num = m.groups()
            idx = int(num)
            return -(idx + 1) if kind == 'Import' else (idx + 1)
    for i in range(len(package.exports)):
        if package.resolve_object_path(i + 1) == text:
            return i + 1
    for i in range(len(package.imports)):
        if package.resolve_object_path(-(i + 1)) == text:
            return -(i + 1)
    return None


# ── DLLBind support ──────────────────────────────────────────────────────────
#
# In UE3 the compiler keyword `DLLBind(SomeDLL)` on a class declaration
# stores the DLL name as an FString field called DLLBindName inside the
# UClass serial body.  It is the LAST field serialized by UClass::Serialize,
# immediately after NativeClassName (also an FString).
#
# When the engine loads the package it reads this field and calls
# LoadLibrary on the named DLL before the class is fully initialised,
# making DLLBind a clean DLL-injection point for Rocket League mods.
#
# Binary layout at the tail of a cooked UClass serial body:
#   [... UClass-specific fields ...]
#   NativeClassName  : FString  (usually empty → 4 zero bytes)
#   DLLBindName      : FString  (empty = 4 zero bytes; or len+chars+NUL)
#
# FString encoding:  int32 length (including NUL) then ASCII bytes + NUL.
#                    Length == 0 means empty string (no NUL follows).

def is_uclass_export(package: ParsedPackage, export: ExportEntry) -> bool:
    """Return True when *export* is itself a class definition (class_index → Class)."""
    return package.export_class_name(export) == "Class"


def find_uclass_dllbind_fstring_offset(raw: bytes) -> Optional[Tuple[int, str]]:
    """Locate the DLLBindName FString at the tail of a UClass serial body.

    Strategy: DLLBindName is the last thing serialized by UClass::Serialize.
    We scan forward from the last 260 bytes of *raw* looking for the unique
    FString whose byte span ends exactly at len(raw).

    Returns (fstring_start_offset, dll_name) where fstring_start_offset is
    the offset (relative to the start of *raw*) of the 4-byte length field of
    DLLBindName, and dll_name is the current value (empty string if no bind).

    Returns None if no valid FString pattern ending at EOF is found.
    """
    L = len(raw)
    if L < 4:
        return None

    # Determine the search window.  DLLBind names are short (<260 chars),
    # so DLLBindName is at most 4+260 = 264 bytes.  We walk backwards.
    lo = max(0, L - 264 - 4)

    # Try every possible starting offset for an FString that ends at L.
    #   fstring_start = pos
    #   length field  = int32 at pos              (4 bytes)
    #   string data   = raw[pos+4 : pos+4+length] (length bytes)
    #   total size    = 4 + length
    #   must satisfy  = pos + 4 + length == L
    for pos in range(L - 4, lo - 1, -1):
        if pos < 0:
            break
        try:
            length = struct.unpack_from("<i", raw, pos)[0]
        except struct.error:
            break

        if length == 0:
            # Empty FString: occupies exactly 4 bytes.
            if pos + 4 == L:
                return pos, ""
            # Keep scanning — this zero might be padding before the real field.
            continue

        if length < 0 or length > 260:
            # Negative → UTF-16 (unusual for DLL names); too large → noise.
            continue

        # Non-empty ASCII FString: must end exactly at L.
        if pos + 4 + length != L:
            continue

        str_bytes = raw[pos + 4: L]
        if len(str_bytes) != length:
            continue
        # Null-terminated ASCII.
        if str_bytes[-1] != 0:
            continue
        try:
            dll_name = str_bytes[:-1].decode("ascii")
        except (UnicodeDecodeError, ValueError):
            continue
        if not dll_name.isprintable():
            continue
        return pos, dll_name

    return None


def set_uclass_dllbind_name(package: ParsedPackage, export: ExportEntry, dll_name: str) -> bytes:
    """Inject or replace the DLLBindName FString in a UClass serial body.

    *dll_name* is the bare DLL name (e.g. ``'CodeRed.dll'``).  Pass an empty
    string to remove an existing DLLBind.

    Returns the full modified package bytes.  The export's serial_size is
    updated and all subsequent exports' serial_offset values are shifted as
    required, mirroring the variable-size StrProperty edit logic.

    Raises ValueError if the DLLBindName field cannot be found (e.g. the
    export is not a UClass) or if *dll_name* contains non-ASCII characters.
    """
    if dll_name and not dll_name.isascii():
        raise ValueError("DLL name must be ASCII (no unicode characters).")

    raw = package.object_data(export)
    if not raw:
        raise ValueError("Export has no serial data.")

    result = find_uclass_dllbind_fstring_offset(raw)
    if result is None:
        raise ValueError(
            "Could not locate DLLBindName in this export's serial data.\n"
            "Make sure the selected export is a UClass (class definition) "
            "and that its serial body is intact."
        )

    fstring_offset, current_dll_name = result

    # Build old and new FString byte representations.
    def encode_fstring(name: str) -> bytes:
        if not name:
            return struct.pack("<i", 0)
        enc = name.encode("ascii") + b"\x00"
        return struct.pack("<i", len(enc)) + enc

    old_fstring = encode_fstring(current_dll_name)
    new_fstring = encode_fstring(dll_name)

    if old_fstring == new_fstring:
        return bytes(package.file_bytes)  # nothing to do

    size_delta = len(new_fstring) - len(old_fstring)

    # Absolute byte range of the old FString inside the package file.
    abs_start = export.serial_offset + fstring_offset
    abs_end   = abs_start + len(old_fstring)

    new_data = bytearray()
    new_data += package.file_bytes[:abs_start]
    new_data += new_fstring
    new_data += package.file_bytes[abs_end:]

    if size_delta != 0:
        export_entry_offsets = get_export_entry_offsets(package)
        if export.table_index >= len(export_entry_offsets):
            raise ValueError("Export table index out of range.")

        # Patch this export's serial_size (at entry_offset + 32).
        entry_offset = export_entry_offsets[export.table_index]
        patch_i32_le(new_data, entry_offset + 32, export.serial_size + size_delta)

        # Shift all exports whose bodies come after this one (at entry_offset + 36).
        for idx, other in enumerate(package.exports):
            if idx == export.table_index:
                continue
            if other.serial_offset > export.serial_offset:
                other_entry_offset = export_entry_offsets[idx]
                patch_i64_le(new_data, other_entry_offset + 36, other.serial_offset + size_delta)

    return bytes(new_data)


class NativeWindowsDropTarget:
    WM_DROPFILES = 0x0233
    GWL_WNDPROC = -4

    def __init__(self, widget: tk.Misc, callback):
        self.widget = widget
        self.callback = callback
        self.enabled = False
        if sys.platform != "win32":
            return
        self.user32 = ctypes.windll.user32
        self.shell32 = ctypes.windll.shell32
        self.user32.SetWindowLongPtrW.restype = ctypes.c_void_p
        self.user32.SetWindowLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p]
        self.user32.CallWindowProcW.restype = ctypes.c_longlong
        self.user32.CallWindowProcW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p]
        self.shell32.DragAcceptFiles.argtypes = [ctypes.c_void_p, ctypes.c_bool]
        self.shell32.DragAcceptFiles.restype = None
        self.shell32.DragQueryFileW.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_wchar_p, ctypes.c_uint]
        self.shell32.DragQueryFileW.restype = ctypes.c_uint
        self.shell32.DragFinish.argtypes = [ctypes.c_void_p]
        self.shell32.DragFinish.restype = None
        self.old_proc = None
        self.new_proc = None
        widget.after(100, self._install)

    def _install(self):
        if sys.platform != "win32" or not self.widget.winfo_exists():
            return
        hwnd = self.widget.winfo_id()
        self.shell32.DragAcceptFiles(hwnd, True)
        WNDPROC = ctypes.WINFUNCTYPE(ctypes.c_longlong, ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p)

        def _wnd_proc(hwnd, msg, wparam, lparam):
            if msg == self.WM_DROPFILES:
                hdrop = ctypes.c_void_p(wparam)
                count = self.shell32.DragQueryFileW(hdrop, 0xFFFFFFFF, None, 0)
                files = []
                for i in range(count):
                    length = self.shell32.DragQueryFileW(hdrop, i, None, 0)
                    buffer = ctypes.create_unicode_buffer(length + 1)
                    self.shell32.DragQueryFileW(hdrop, i, buffer, length + 1)
                    files.append(buffer.value)
                self.shell32.DragFinish(hdrop)
                self.widget.after(0, lambda: self.callback(files))
                return 0
            return self.user32.CallWindowProcW(self.old_proc, hwnd, msg, wparam, lparam)

        self.new_proc = WNDPROC(_wnd_proc)
        self.old_proc = self.user32.SetWindowLongPtrW(hwnd, self.GWL_WNDPROC, self.new_proc)
        self.enabled = True


def read_tarray(reader: BinaryReader, read_item):
    count = reader.read_i32()
    return [read_item(reader) for _ in range(count)]


def read_guid(reader: BinaryReader) -> Tuple[int, int, int, int]:
    return (reader.read_u32(), reader.read_u32(), reader.read_u32(), reader.read_u32())


def read_generation(reader: BinaryReader) -> Tuple[int, int, int]:
    return (reader.read_i32(), reader.read_i32(), reader.read_i32())


def read_texture_allocation(reader: BinaryReader):
    reader.read_i32()
    reader.read_i32()
    reader.read_i32()
    reader.read_i32()
    reader.read_i32()
    read_tarray(reader, lambda r: r.read_i32())
    return None


def read_compact_index(reader: BinaryReader) -> int:
    index = 0
    b0 = reader.read_u8()
    if (b0 & 0x40) != 0:
        b1 = reader.read_u8()
        if (b1 & 0x80) != 0:
            b2 = reader.read_u8()
            if (b2 & 0x80) != 0:
                b3 = reader.read_u8()
                if (b3 & 0x80) != 0:
                    b4 = reader.read_u8()
                    index = b4
                index = (index << 7) | (b3 & 0x7F)
            index = (index << 7) | (b2 & 0x7F)
        index = (index << 7) | (b1 & 0x7F)
    index = (index << 6) | (b0 & 0x3F)
    if (b0 & 0x80) != 0:
        index *= -1
    return index


def read_index_pkg(reader: BinaryReader, package: ParsedPackage) -> int:
    if package.summary.file_version >= COMPACT_INDEX_DEPRECATED:
        return reader.read_i32()
    return read_compact_index(reader)


def read_fname_pkg(reader: BinaryReader, package: ParsedPackage) -> FNameRef:
    name_index = read_index_pkg(reader, package)
    if package.summary.file_version >= NUMBER_ADDED_TO_NAME:
        instance_number = reader.read_i32() - 1
    else:
        instance_number = -1
    return FNameRef(name_index, instance_number)


def read_fname(reader: BinaryReader, summary: Optional["FileSummary"] = None) -> FNameRef:
    # When called with a FileSummary, applies the same UE3 instance-number
    # convention as read_fname_pkg: the value stored on disk is (number + 1),
    # so we subtract 1 to recover the in-memory number where -1 means "no
    # suffix" and 0/1/2/... are real instance suffixes. UE Explorer's
    # ReadName/ReadNameReference does the same thing
    # (see UELib/src/UnrealStream.cs ReadName, line ~509-516). When called
    # without a summary we keep the legacy raw read for any caller that
    # genuinely wants two i32s with no adjustment - currently nothing in the
    # codebase relies on that, but the default keeps the signature backwards
    # compatible if external callers exist.
    name_index = reader.read_i32()
    raw_instance = reader.read_i32()
    if summary is not None and summary.file_version >= NUMBER_ADDED_TO_NAME:
        instance_number = raw_instance - 1
    else:
        instance_number = raw_instance
    return FNameRef(name_index, instance_number)


def read_name_entry(reader: BinaryReader, index: int) -> NameEntry:
    return NameEntry(index=index, name=reader.read_fstring(), flags=reader.read_u64())


def read_compressed_chunk_32(reader: BinaryReader) -> FCompressedChunk:
    return FCompressedChunk(
        uncompressed_offset=reader.read_i32(),
        uncompressed_size=reader.read_i32(),
        compressed_offset=reader.read_i32(),
        compressed_size=reader.read_i32(),
    )


def read_compressed_chunk_64(reader: BinaryReader) -> FCompressedChunk:
    return FCompressedChunk(
        uncompressed_offset=reader.read_i64(),
        uncompressed_size=reader.read_i32(),
        compressed_offset=reader.read_i64(),
        compressed_size=reader.read_i32(),
    )


def parse_file_summary(stream: BinaryIO) -> FileSummary:
    r = BinaryReader(stream)
    summary = FileSummary()
    summary.tag = r.read_u32()
    if summary.tag != PACKAGE_FILE_TAG:
        raise ValueError("Not a valid Unreal Engine package")
    summary.file_version = r.read_u16()
    summary.licensee_version = r.read_u16()
    summary.total_header_size = r.read_i32()
    summary.folder_name = r.read_fstring()
    summary.package_flags_flags_offset = r.tell()
    summary.package_flags = r.read_u32()
    summary.name_count = r.read_i32()
    summary.name_offset = r.read_i32()
    summary.export_count = r.read_i32()
    summary.export_offset = r.read_i32()
    summary.import_count = r.read_i32()
    summary.import_offset = r.read_i32()
    summary.depends_offset = r.read_i32()
    summary.import_export_guids_offset = r.read_i32()
    summary.import_guids_count = r.read_i32()
    summary.export_guids_count = r.read_i32()
    summary.thumbnail_table_offset = r.read_i32()
    summary.guid = read_guid(r)
    summary.generations = read_tarray(r, read_generation)
    summary.engine_version = r.read_u32()
    summary.cooker_version = r.read_u32()
    summary.compression_flags_offset = r.tell()
    summary.compression_flags = r.read_u32()
    summary.compressed_chunks = read_tarray(r, read_compressed_chunk_32)
    r.read_i32()
    read_tarray(r, lambda rr: rr.read_fstring())
    read_tarray(r, read_texture_allocation)
    return summary


def parse_file_compression_metadata(stream: BinaryIO) -> FileCompressionMetaData:
    r = BinaryReader(stream)
    return FileCompressionMetaData(
        garbage_size=r.read_i32(),
        compressed_chunks_offset=r.read_i32(),
        last_block_size=r.read_i32(),
    )


def verify_decryptor(summary: FileSummary, meta: FileCompressionMetaData, key: bytes, encrypted_data: bytes) -> bool:
    block_offset = meta.compressed_chunks_offset % 16
    block_start = meta.compressed_chunks_offset - block_offset
    probe = encrypted_data[block_start:block_start + 32]
    if len(probe) != 32:
        return False
    decrypted = DecryptionProvider.decrypt_ecb(key, probe)
    view = decrypted[block_offset:]
    if len(view) < 8:
        return False
    chunk_info_length, first_uncompressed_offset = struct.unpack("<ii", view[:8])
    return chunk_info_length >= 1 and first_uncompressed_offset == summary.depends_offset


def decrypt_data(stream: BinaryIO, summary: FileSummary, meta: FileCompressionMetaData, provider: DecryptionProvider) -> bytes:
    encrypted_size = summary.total_header_size - meta.garbage_size - summary.name_offset
    encrypted_size = (encrypted_size + 15) & ~15
    stream.seek(summary.name_offset)
    encrypted_data = stream.read(encrypted_size)
    if len(encrypted_data) != encrypted_size:
        raise ValueError("Failed to read the encrypted data from the stream")
    valid_key = None
    for key in provider.decryption_keys:
        if verify_decryptor(summary, meta, key, encrypted_data):
            valid_key = key
            break
    if valid_key is None:
        raise ValueError("Unknown Decryption key")
    return DecryptionProvider.decrypt_ecb(valid_key, encrypted_data)


def parse_rl_compressed_chunks(decrypted_data: bytes, offset: int) -> List[FCompressedChunk]:
    bio = io.BytesIO(decrypted_data)
    bio.seek(offset)
    r = BinaryReader(bio)
    return read_tarray(r, read_compressed_chunk_64)


def process_compressed_data(output: BinaryIO, package_stream: BinaryIO, summary: FileSummary) -> None:
    if not summary.compressed_chunks:
        raise ValueError("No compressed chunks were found in decrypted data")
    first_uncompressed_offset = summary.compressed_chunks[0].uncompressed_offset
    last_chunk = summary.compressed_chunks[-1]
    final_size = last_chunk.uncompressed_offset + last_chunk.uncompressed_size
    output.truncate(final_size)
    output.seek(first_uncompressed_offset)
    r = BinaryReader(package_stream)
    for chunk in summary.compressed_chunks:
        package_stream.seek(chunk.compressed_offset)
        r.read_i32()
        r.read_i32()
        r.read_i32()
        total_uncompressed_size = r.read_i32()
        sum_uncompressed_size = 0
        blocks: List[Tuple[int, int]] = []
        while sum_uncompressed_size < total_uncompressed_size:
            comp_size = r.read_i32()
            uncomp_size = r.read_i32()
            blocks.append((comp_size, uncomp_size))
            sum_uncompressed_size += uncomp_size
        for comp_size, uncomp_size in blocks:
            compressed_block = r.read_exact(comp_size)
            inflated = zlib.decompress(compressed_block)
            if len(inflated) != uncomp_size:
                raise ValueError(f"Unexpected uncompressed block size: expected {uncomp_size}, got {len(inflated)}")
            output.write(inflated)
    output.seek(summary.package_flags_flags_offset)
    output.write(struct.pack("<I", summary.package_flags & ~PKG_COOKED))
    output.seek(summary.compression_flags_offset)
    output.write(struct.pack("<I", COMPRESS_NONE))


def unpack_package(input_path: str, output_path: str, provider: DecryptionProvider) -> Path:
    with open(input_path, "rb") as src:
        summary = parse_file_summary(src)
        if (summary.compression_flags & COMPRESS_ZLIB) == 0:
            raise ValueError("Package compression type is unsupported")
        meta = parse_file_compression_metadata(src)
        src.seek(0)
        header_bytes = src.read(summary.name_offset)
        decrypted_data = decrypt_data(src, summary, meta, provider)
        summary.compressed_chunks = parse_rl_compressed_chunks(decrypted_data, meta.compressed_chunks_offset)
        if not summary.compressed_chunks or summary.compressed_chunks[0].uncompressed_offset != summary.depends_offset:
            raise ValueError("Failed to parse decrypted compressed chunk table")
        output_path = str(output_path)
        os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
        with open(output_path, "wb+") as dst:
            dst.write(header_bytes)
            dst.write(decrypted_data)
            process_compressed_data(dst, src, summary)
    return Path(output_path)


def unpack_plain_package(input_path: str, output_path: str) -> Path:
    with open(input_path, "rb") as src:
        summary = parse_file_summary(src)
        if (summary.compression_flags & COMPRESS_ZLIB) == 0:
            raise ValueError("Package compression type is unsupported")
        output_path = str(output_path)
        os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
        original_bytes = Path(input_path).read_bytes()
        with open(output_path, "wb+") as dst:
            dst.write(original_bytes)
            process_compressed_data(dst, src, summary)
    return Path(output_path)


def try_parse_plain_package(input_path: Path) -> Optional["ParsedPackage"]:
    try:
        return parse_decrypted_package(input_path)
    except Exception:
        return None


def resolve_input_package(input_path: Path, decrypted_dir: Path, script_dir: Path) -> Tuple[Path, "ParsedPackage", Optional[DecryptionProvider], Optional[Path], bool]:
    plain_package = try_parse_plain_package(input_path)
    if plain_package is not None:
        return input_path, plain_package, None, None, False

    with input_path.open("rb") as fh:
        summary = parse_file_summary(fh)

    if (summary.compression_flags & COMPRESS_ZLIB) != 0:
        plain_decompressed_path = decrypted_dir / f"{input_path.stem}_decompressed.upk"
        try:
            unpack_plain_package(str(input_path), str(plain_decompressed_path))
            return plain_decompressed_path, parse_decrypted_package(plain_decompressed_path), None, None, False
        except Exception:
            pass

    keys_path = find_keys_path(script_dir, input_path)
    if keys_path is None:
        raise FileNotFoundError("Could not find keys.txt next to the script, current directory, or selected file")
    provider = DecryptionProvider(str(keys_path))
    decrypted_path = decrypted_dir / f"{input_path.stem}_decrypted.upk"
    unpack_package(str(input_path), str(decrypted_path), provider)
    return decrypted_path, parse_decrypted_package(decrypted_path), provider, keys_path, True


def parse_import_entry(reader: BinaryReader, table_index: int, summary: "FileSummary") -> ImportEntry:
    return ImportEntry(
        table_index=table_index,
        class_package=read_fname(reader, summary),
        class_name=read_fname(reader, summary),
        outer_index=reader.read_i32(),
        object_name=read_fname(reader, summary),
    )


def parse_export_entry(reader: BinaryReader, table_index: int, generation_count: int, summary: "FileSummary") -> ExportEntry:
    # The export entry layout in this UE3 build is:
    #   class_index (i32) | super_index (i32) | outer_index (i32) |
    #   object_name (FName: i32 name_index + i32 instance_number) |
    #   archetype_index (i32) | object_flags (u64) |
    #   serial_size (i32) | serial_offset (i64) | export_flags (i32) |
    #   net_objects (TArray<i32>: i32 count + count * i32) |
    #   package_guid (4*u32) | package_flags (i32)
    #
    # net_objects IS length-prefixed in this package version - it was the
    # generation_count assumption that was wrong. The original "None / Class
    # / 0 / 0" tail in the GUI is most likely an artifact of the export count
    # in the summary being larger than the number of real entries on disk
    # (the table is followed by zero padding), not a parser desync. The
    # generation_count parameter is kept for signature stability with
    # callers, but is not used.
    del generation_count
    class_index = reader.read_i32()
    super_index = reader.read_i32()
    outer_index = reader.read_i32()
    object_name = read_fname(reader, summary)
    archetype_index = reader.read_i32()
    object_flags = reader.read_u64()
    serial_size = reader.read_i32()
    serial_offset = reader.read_i64()
    export_flags = reader.read_i32()
    net_objects = read_tarray(reader, lambda rr: rr.read_i32())
    package_guid = read_guid(reader)
    package_flags = reader.read_i32()
    return ExportEntry(
        table_index=table_index,
        class_index=class_index,
        super_index=super_index,
        outer_index=outer_index,
        object_name=object_name,
        archetype_index=archetype_index,
        object_flags=object_flags,
        serial_size=serial_size,
        serial_offset=serial_offset,
        export_flags=export_flags,
        net_objects=net_objects,
        package_guid=package_guid,
        package_flags=package_flags,
    )


def parse_decrypted_package(file_path: Path) -> ParsedPackage:
    data = file_path.read_bytes()
    bio = io.BytesIO(data)
    summary = parse_file_summary(bio)
    if summary.compression_flags != COMPRESS_NONE:
        raise ValueError("The decrypted package is still marked as compressed")
    r = BinaryReader(bio)
    bio.seek(summary.name_offset)
    names = [read_name_entry(r, i) for i in range(summary.name_count)]
    bio.seek(summary.import_offset)
    imports = [parse_import_entry(r, i, summary) for i in range(summary.import_count)]
    bio.seek(summary.export_offset)
    exports = [parse_export_entry(r, i, len(summary.generations), summary) for i in range(summary.export_count)]
    return ParsedPackage(file_path=file_path, summary=summary, names=names, imports=imports, exports=exports, file_bytes=data)


def parse_decrypted_package_bytes(file_path: Path, data: bytes) -> ParsedPackage:
    bio = io.BytesIO(data)
    summary = parse_file_summary(bio)
    if summary.compression_flags != COMPRESS_NONE:
        raise ValueError("The decrypted package is still marked as compressed")
    r = BinaryReader(bio)
    bio.seek(summary.name_offset)
    names = [read_name_entry(r, i) for i in range(summary.name_count)]
    bio.seek(summary.import_offset)
    imports = [parse_import_entry(r, i, summary) for i in range(summary.import_count)]
    bio.seek(summary.export_offset)
    exports = [parse_export_entry(r, i, len(summary.generations), summary) for i in range(summary.export_count)]
    return ParsedPackage(file_path=file_path, summary=summary, names=names, imports=imports, exports=exports, file_bytes=data)


def verify_package(package: ParsedPackage) -> List[Tuple[str, str]]:
    """Deep consistency check on a parsed package's header tables.

    Returns a list of (severity, message) tuples where severity is one of
    'OK', 'WARN', 'ERROR'. An 'ERROR' indicates the package is internally
    inconsistent in a way that the engine is likely to choke on at load
    time (often manifesting as a freeze or crash). 'WARN' flags things
    that look unusual but might be intentional. 'OK' lines summarize
    successful invariant checks.

    The checks here are derived from cross-referencing UE Explorer's
    canonical loader and the offset bookkeeping in our own _replace_header_tables.
    """
    findings: List[Tuple[str, str]] = []
    s = package.summary
    file_len = len(package.file_bytes)

    # Summary-level offset sanity.
    if s.name_offset <= 0 or s.name_offset >= file_len:
        findings.append(("ERROR", f"name_offset {s.name_offset} is out of bounds (file size {file_len})"))
    if s.import_offset <= 0 or s.import_offset >= file_len:
        findings.append(("ERROR", f"import_offset {s.import_offset} is out of bounds"))
    if s.export_offset <= 0 or s.export_offset >= file_len:
        findings.append(("ERROR", f"export_offset {s.export_offset} is out of bounds"))
    if s.depends_offset <= 0 or s.depends_offset > file_len:
        findings.append(("ERROR", f"depends_offset {s.depends_offset} is out of bounds"))

    # Tables must be in the canonical order: names < imports < exports < depends.
    if not (s.name_offset < s.import_offset < s.export_offset < s.depends_offset):
        findings.append((
            "ERROR",
            f"Header tables out of order: names@{s.name_offset} imports@{s.import_offset} "
            f"exports@{s.export_offset} depends@{s.depends_offset}",
        ))
    else:
        findings.append(("OK", "Header tables are in canonical order"))

    # total_header_size sanity. For plain (decompressed) packages this should
    # equal depends_offset + (size of depends table). We can't compute the
    # depends table size without re-parsing it, but we can at least require
    # total_header_size >= depends_offset.
    if s.total_header_size < s.depends_offset:
        findings.append((
            "ERROR",
            f"total_header_size {s.total_header_size} is less than depends_offset {s.depends_offset}; "
            f"header region claims to end before the depends table starts",
        ))
    else:
        findings.append(("OK", f"total_header_size {s.total_header_size} >= depends_offset {s.depends_offset}"))

    # Re-parse the export table from disk and verify the cursor lands at
    # depends_offset. If it doesn't, the export entries on disk don't match
    # what our parser thinks they look like, which would also confuse the
    # engine.
    try:
        bio = io.BytesIO(package.file_bytes)
        bio.seek(s.export_offset)
        r = BinaryReader(bio)
        for i in range(s.export_count):
            parse_export_entry(r, i, len(s.generations), s)
        end_cursor = bio.tell()
        if end_cursor != s.depends_offset:
            findings.append((
                "ERROR",
                f"After parsing {s.export_count} exports cursor is at {end_cursor}, "
                f"expected depends_offset {s.depends_offset} (delta {end_cursor - s.depends_offset})",
            ))
        else:
            findings.append(("OK", f"Export table parse cursor lands exactly at depends_offset"))
    except Exception as exc:
        findings.append(("ERROR", f"Export table re-parse failed: {exc}"))

    # Per-export bounds. Every export's [serial_offset, serial_offset + serial_size)
    # must lie inside the file, and must lie at or after total_header_size
    # (the export bodies live in the data region, not the header region).
    body_violations = 0
    for exp in package.exports:
        if package.is_placeholder_export(exp):
            continue
        if exp.serial_size < 0:
            findings.append(("ERROR", f"Export[{exp.table_index}] has negative serial_size {exp.serial_size}"))
            body_violations += 1
            continue
        if exp.serial_size == 0:
            continue  # Zero-size exports legitimately have offset 0.
        if exp.serial_offset < s.total_header_size:
            findings.append((
                "ERROR",
                f"Export[{exp.table_index}] '{package.resolve_name(exp.object_name)}' "
                f"serial_offset {exp.serial_offset} is before total_header_size {s.total_header_size}",
            ))
            body_violations += 1
        if exp.serial_offset + exp.serial_size > file_len:
            findings.append((
                "ERROR",
                f"Export[{exp.table_index}] '{package.resolve_name(exp.object_name)}' "
                f"body extends past EOF: serial_offset={exp.serial_offset} + serial_size={exp.serial_size} > file_size={file_len}",
            ))
            body_violations += 1
    if body_violations == 0:
        findings.append(("OK", f"All {sum(1 for e in package.exports if not package.is_placeholder_export(e))} non-placeholder export bodies are in-bounds"))

    # Detect overlapping export bodies. Two exports' [start, end) ranges
    # should not overlap.
    bodies = sorted(
        ((exp.serial_offset, exp.serial_offset + exp.serial_size, exp.table_index, package.resolve_name(exp.object_name))
         for exp in package.exports
         if exp.serial_size > 0 and not package.is_placeholder_export(exp)),
        key=lambda x: x[0],
    )
    overlap_count = 0
    for prev, curr in zip(bodies, bodies[1:]):
        if curr[0] < prev[1]:
            findings.append((
                "ERROR",
                f"Export bodies overlap: Export[{prev[2]}] '{prev[3]}' [{prev[0]}, {prev[1]}) "
                f"vs Export[{curr[2]}] '{curr[3]}' [{curr[0]}, {curr[1]})",
            ))
            overlap_count += 1
    if overlap_count == 0 and bodies:
        findings.append(("OK", f"No overlapping export bodies among {len(bodies)} non-placeholder exports"))

    # Cross-reference checks. Every export.class_index, super_index,
    # outer_index, archetype_index must be a valid export or import index.
    def _index_label(idx: int) -> str:
        if idx == 0:
            return "None"
        if idx > 0:
            return f"Export[{idx - 1}]"
        return f"Import[{-idx - 1}]"

    bad_refs = 0
    for exp in package.exports:
        for field_name in ("class_index", "super_index", "outer_index", "archetype_index"):
            idx = getattr(exp, field_name)
            if idx == 0:
                continue
            if idx > 0:
                if not (1 <= idx <= len(package.exports)):
                    findings.append((
                        "ERROR",
                        f"Export[{exp.table_index}] '{package.resolve_name(exp.object_name)}' "
                        f"{field_name}={idx} -> {_index_label(idx)} is out of range",
                    ))
                    bad_refs += 1
            else:
                if not (1 <= -idx <= len(package.imports)):
                    findings.append((
                        "ERROR",
                        f"Export[{exp.table_index}] '{package.resolve_name(exp.object_name)}' "
                        f"{field_name}={idx} -> {_index_label(idx)} is out of range",
                    ))
                    bad_refs += 1
    for imp in package.imports:
        # Imports' outer_index can be 0 (top-level) or another import (negative)
        # or an export (positive). All three are legal in UE3 - imports
        # parented to exports happen for sub-objects of exports referenced
        # from outside.
        if imp.outer_index == 0:
            continue
        if imp.outer_index > 0:
            if not (1 <= imp.outer_index <= len(package.exports)):
                findings.append((
                    "ERROR",
                    f"Import[{imp.table_index}] '{package.resolve_name(imp.object_name)}' "
                    f"outer_index={imp.outer_index} is out of range",
                ))
                bad_refs += 1
        else:
            if not (1 <= -imp.outer_index <= len(package.imports)):
                findings.append((
                    "ERROR",
                    f"Import[{imp.table_index}] '{package.resolve_name(imp.object_name)}' "
                    f"outer_index={imp.outer_index} is out of range",
                ))
                bad_refs += 1
    if bad_refs == 0:
        findings.append(("OK", "All export/import cross-references resolve"))

    # Name index validity. Every FNameRef in every import/export must have
    # a name_index inside the names table.
    bad_names = 0
    name_count = len(package.names)
    for imp in package.imports:
        for label, ref in (("class_package", imp.class_package), ("class_name", imp.class_name), ("object_name", imp.object_name)):
            if not (0 <= ref.name_index < name_count):
                findings.append((
                    "ERROR",
                    f"Import[{imp.table_index}] {label}.name_index={ref.name_index} is out of range (name_count={name_count})",
                ))
                bad_names += 1
    for exp in package.exports:
        if not (0 <= exp.object_name.name_index < name_count):
            findings.append((
                "ERROR",
                f"Export[{exp.table_index}] object_name.name_index={exp.object_name.name_index} is out of range",
            ))
            bad_names += 1
    if bad_names == 0:
        findings.append(("OK", f"All FName references point inside the {name_count}-entry name table"))

    # Detect orphan exports: any non-placeholder export whose outer_index
    # cannot be reached from a root (outer_index == 0). Exports that descend
    # into invalid outers are loadable in isolation but the engine may
    # behave oddly when iterating the package tree.
    orphans = 0
    visited: Dict[int, bool] = {}

    def _has_root(idx: int, depth: int = 0) -> bool:
        if depth > len(package.exports) + len(package.imports) + 2:
            return False  # cycle
        if idx == 0:
            return True
        if idx in visited:
            return visited[idx]
        if idx > 0:
            outer = package.exports[idx - 1].outer_index
        else:
            outer = package.imports[-idx - 1].outer_index
        result = _has_root(outer, depth + 1)
        visited[idx] = result
        return result

    for exp in package.exports:
        if package.is_placeholder_export(exp):
            continue
        if not _has_root(exp.table_index + 1):
            findings.append((
                "WARN",
                f"Export[{exp.table_index}] '{package.resolve_name(exp.object_name)}' "
                "has an unreachable outer chain (cycle or invalid outer)",
            ))
            orphans += 1
    if orphans == 0:
        findings.append(("OK", "All non-placeholder exports have valid outer chains"))

    return findings


def format_verify_report(findings: List[Tuple[str, str]]) -> str:
    error_count = sum(1 for sev, _ in findings if sev == "ERROR")
    warn_count = sum(1 for sev, _ in findings if sev == "WARN")
    ok_count = sum(1 for sev, _ in findings if sev == "OK")
    header = f"Package verification: {error_count} error(s), {warn_count} warning(s), {ok_count} check(s) passed"
    lines = [header, "=" * len(header), ""]
    # Errors first, then warnings, then OK.
    for severity in ("ERROR", "WARN", "OK"):
        for sev, msg in findings:
            if sev == severity:
                lines.append(f"[{sev}] {msg}")
    return "\n".join(lines)


def sha1_file(path: Path) -> str:
    digest = hashlib.sha1()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def patch_sha1_in_exe(exe_path: Path, old_sha1_hex: str, new_sha1_hex: str) -> int:
    old_sha1_hex = old_sha1_hex.strip().lower()
    new_sha1_hex = new_sha1_hex.strip().lower()
    if len(old_sha1_hex) != 40 or len(new_sha1_hex) != 40:
        raise ValueError("SHA-1 values must be 40 hex characters")
    old_bytes = bytes.fromhex(old_sha1_hex)
    new_bytes = bytes.fromhex(new_sha1_hex)
    data = bytearray(exe_path.read_bytes())
    count = 0
    start = 0
    while True:
        idx = data.find(old_bytes, start)
        if idx < 0:
            break
        data[idx:idx + len(old_bytes)] = new_bytes
        count += 1
        start = idx + len(new_bytes)
    if count == 0:
        raise ValueError(f"Original SHA-1 bytes were not found in {exe_path.name}")
    exe_path.write_bytes(data)
    return count


def find_keys_path(script_dir: Path, selected_file: Path) -> Optional[Path]:
    candidates = [
        script_dir / "keys.txt",
        Path.cwd() / "keys.txt",
        selected_file.parent / "keys.txt",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return None


def format_hex_preview(data: bytes, base_offset: int = 0) -> str:
    if not data:
        return ""
    lines = []
    for i in range(0, len(data), 16):
        chunk = data[i:i + 16]
        hex_part = " ".join(f"{b:02X}" for b in chunk)
        ascii_part = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        lines.append(f"{base_offset + i:08X}  {hex_part:<47}  {ascii_part}")
    return "\n".join(lines)


class App:
    def __init__(self, root: tk.Tk):
        self.root = root
        self.root.title("RL UPK All-in-One GUI")
        self.root.geometry("1500x920")
        self.script_dir = Path(__file__).resolve().parent
        self.decrypted_dir = self.script_dir / "Decrypted"
        self.decrypted_dir.mkdir(exist_ok=True)
        self.sha1_memory: Dict[str, Dict[str, str]] = {}
        self.package: Optional[ParsedPackage] = None
        self.current_content_map: Dict[str, ExportEntry] = {}
        self.current_export_map: Dict[str, ExportEntry] = {}
        self.current_import_map: Dict[str, ImportEntry] = {}
        self.current_name_map: Dict[str, NameEntry] = {}
        self.selected_input_path: Optional[Path] = None
        self.sdk_db: Optional[RLSDKDatabase] = None
        self.sdk_path: Optional[Path] = None
        self.current_properties: List[ParsedProperty] = []
        self.current_export: Optional[ExportEntry] = None
        self.current_property: Optional[ParsedProperty] = None
        self.current_provider: Optional[DecryptionProvider] = None
        self.current_encrypted_input_path: Optional[Path] = None
        self.current_keys_path: Optional[Path] = None
        self.current_original_sha1: Optional[str] = None
        self.donor_key_upk_path: Optional[Path] = None  # path to donor encrypted UPK for key sourcing
        self.use_donor_key_var: Optional[tk.BooleanVar] = None  # set during _build_ui
        self.donor_key_path_var: Optional[tk.StringVar] = None  # display label, set during _build_ui
        self.status_var = tk.StringVar(value="Ready")
        self.original_var = tk.StringVar(value="Original: -")
        self.sha1_var = tk.StringVar(value="SHA-1: -")
        self.decrypted_var = tk.StringVar(value="Decrypted: -")
        self.keys_var = tk.StringVar(value="Keys: -")
        self.sdk_var = tk.StringVar(value="Property parser: package-native UE3 tags")
        self._build_ui()
        self.drop_target = NativeWindowsDropTarget(self.root, self._on_drop_files)
        self.root.after(200, self._update_drop_label)

    def _build_ui(self):
        self.root.configure(bg="#101114")
        style = ttk.Style()
        try:
            style.theme_use("clam")
        except tk.TclError:
            pass
        style.configure("Treeview", rowheight=24)

        top = tk.Frame(self.root, bg="#101114")
        top.pack(fill="x", padx=12, pady=12)

        open_button = ttk.Button(top, text="Open UPK", command=self.open_file)
        open_button.pack(side="left")

        self.drop_label = tk.Label(
            top,
            text="Drop an encrypted .upk here or use Open UPK",
            bg="#171a21",
            fg="#dbe6ff",
            padx=18,
            pady=12,
            relief="groove",
            bd=2,
        )
        self.drop_label.pack(side="left", fill="x", expand=True, padx=(10, 0))

        info = tk.Frame(self.root, bg="#101114")
        info.pack(fill="x", padx=12)
        for value in [self.original_var, self.sha1_var, self.decrypted_var, self.keys_var]:
            tk.Label(info, textvariable=value, anchor="w", bg="#101114", fg="#dbe6ff").pack(fill="x")

        main = tk.PanedWindow(self.root, sashrelief="flat", sashwidth=6, bg="#101114")
        main.pack(fill="both", expand=True, padx=12, pady=12)

        left = tk.Frame(main, bg="#101114")
        right = tk.Frame(main, bg="#101114")
        main.add(left, minsize=520)
        main.add(right, minsize=520)

        session_frame = ttk.LabelFrame(left, text="Session SHA-1 Memory")
        session_frame.pack(fill="x")
        self.history_tree = ttk.Treeview(session_frame, columns=("sha1",), show="tree headings", height=6)
        self.history_tree.heading("#0", text="File")
        self.history_tree.heading("sha1", text="SHA-1")
        self.history_tree.column("#0", width=320, anchor="w")
        self.history_tree.column("sha1", width=260, anchor="w")
        self.history_tree.pack(fill="x", padx=6, pady=6)

        left_notebook = ttk.Notebook(left)
        left_notebook.pack(fill="both", expand=True, pady=(12, 0))

        self.content_tree = self._make_tree(left_notebook, ("class", "path"), (240, 380))
        self.content_tree.heading("#0", text="Name")
        left_notebook.add(self.content_tree.master, text="Content")
        self.content_tree.bind("<<TreeviewSelect>>", self._on_content_select)

        # Exports tab: wrap the tree in a frame that also holds a
        # 'Hide placeholder exports' checkbox. RL .upk files contain export
        # slots that are entirely zeroed (class=0, name='None', no body) -
        # these are real on-disk entries, not parser bugs, but they clutter
        # the table. The checkbox lets the user filter them out, matching
        # how UE Explorer hides them from its class list. Default is OFF so
        # nothing changes for existing workflows; flipping it on hides the
        # placeholders. Same pack-order rule as the Names tab: bottom row
        # first, tree second, so the tree's expand=True doesn't shove the
        # row off-screen.
        exports_tab = ttk.Frame(left_notebook)
        left_notebook.add(exports_tab, text="Exports")

        self.hide_placeholder_exports_var = tk.BooleanVar(value=False)
        exports_top_row = ttk.Frame(exports_tab)
        exports_top_row.pack(side="top", fill="x", padx=6, pady=(6, 0))
        ttk.Checkbutton(
            exports_top_row,
            text="Hide placeholder exports (class=0, name=None, empty)",
            variable=self.hide_placeholder_exports_var,
            command=self._populate_exports_tree,
        ).pack(side="left")
        self.exports_placeholder_count_var = tk.StringVar(value="")
        ttk.Label(exports_top_row, textvariable=self.exports_placeholder_count_var).pack(side="left", padx=(12, 0))

        self.exports_tree = self._make_tree(exports_tab, ("name", "class", "outer", "size", "offset"), (220, 160, 240, 90, 110))
        self.exports_tree.heading("#0", text="Index")
        self.exports_tree.master.pack(side="top", fill="both", expand=True)
        self.exports_tree.bind("<<TreeviewSelect>>", self._on_exports_select)
        # Visual styling for placeholder rows so they're clearly distinct
        # even when shown. Treeview supports tag_configure for per-row
        # foreground colors.
        self.exports_tree.tag_configure("placeholder", foreground="#5a6577")

        self.imports_tree = self._make_tree(left_notebook, ("name", "class", "package", "outer"), (220, 160, 160, 260))
        self.imports_tree.heading("#0", text="Index")
        left_notebook.add(self.imports_tree.master, text="Imports")
        self.imports_tree.bind("<<TreeviewSelect>>", self._on_imports_select)

        # Names tab: a wrapper frame holds both the names tree (top, fills
        # available space) and the inline rename row (bottom, fixed height).
        # Without the wrapper, the edit row would be inside the tree's own
        # grid-managed frame and could get clipped by the notebook's height
        # calculation, so we mimic the Properties-tab layout: pack the tree
        # frame and the edit row as siblings inside one container that gets
        # added to the notebook.
        #
        # Pack order matters: the bottom edit row is packed FIRST so pack
        # reserves its space before the tree's expand=True consumes the rest.
        # If we packed the tree first, it would claim all available height
        # and the edit row would be pushed below the visible area.
        names_tab = ttk.Frame(left_notebook)
        left_notebook.add(names_tab, text="Names")

        # Edit row pinned to the bottom of the Names tab, outside the tree's
        # internal grid so the notebook can never hide it. Packed first.
        names_edit = ttk.Frame(names_tab)
        names_edit.pack(side="bottom", fill="x", padx=6, pady=6)
        self.name_edit_info_var = tk.StringVar(value="Select a name to rename")
        ttk.Label(names_edit, textvariable=self.name_edit_info_var).pack(fill="x")
        name_edit_row = ttk.Frame(names_edit)
        name_edit_row.pack(fill="x", pady=(4, 0))
        ttk.Label(name_edit_row, text="New Text:").pack(side="left")
        self.name_edit_var = tk.StringVar()
        self.name_edit_entry = ttk.Entry(name_edit_row, textvariable=self.name_edit_var)
        self.name_edit_entry.pack(side="left", fill="x", expand=True, padx=(6, 6))
        # Pressing Enter in the entry applies the rename.
        self.name_edit_entry.bind("<Return>", lambda _e: self.rename_selected_name())
        ttk.Button(name_edit_row, text="Rename Name", command=self.rename_selected_name).pack(side="left")

        # Now pack the tree on top, filling the remaining space.
        self.names_tree = self._make_tree(names_tab, ("name", "flags"), (380, 220))
        self.names_tree.heading("#0", text="Index")
        # The tree's own frame (created by _make_tree, exposed as tree.master)
        # is already grid-managed internally; pack it into the wrapper.
        self.names_tree.master.pack(side="top", fill="both", expand=True)
        self.names_tree.bind("<<TreeviewSelect>>", self._on_names_select)
        # Double-clicking a row in the names tree focuses the entry so the
        # user can immediately type a replacement.
        self.names_tree.bind("<Double-1>", self._on_names_double_click)

        self.summary_tree = self._make_tree(left_notebook, ("value",), (520,))
        self.summary_tree.heading("#0", text="Field")
        left_notebook.add(self.summary_tree.master, text="Summary")

        right_notebook = ttk.Notebook(right)
        right_notebook.pack(fill="both", expand=True)

        details_frame = ttk.Frame(right_notebook)
        right_notebook.add(details_frame, text="Details")
        self.details_text = tk.Text(details_frame, wrap="none", bg="#111318", fg="#e8ecf4", insertbackground="#ffffff")
        self._pack_text_with_scrollbars(details_frame, self.details_text)

        properties_frame = ttk.Frame(right_notebook)
        right_notebook.add(properties_frame, text="Properties")
        self.properties_tree = self._make_tree(properties_frame, ("declared_type", "tag_type", "owner_type", "size", "array_index", "value"), (180, 130, 220, 80, 90, 420))
        self.properties_tree.heading("#0", text="Property")
        self.properties_tree.master.pack(fill="both", expand=True)
        self.properties_tree.bind("<<TreeviewSelect>>", self._on_property_select)
        prop_edit = ttk.Frame(properties_frame)
        prop_edit.pack(fill="x", padx=6, pady=6)
        self.property_info_var = tk.StringVar(value="Select a property to edit")
        ttk.Label(prop_edit, textvariable=self.property_info_var).pack(fill="x")
        # ── Donor-key row ────────────────────────────────────────────────────
        # When enabled, re-encryption uses the key that decrypts a *different*
        # encrypted UPK (selected by the user) rather than the key that was
        # used to decrypt the currently-loaded package.  This lets you take an
        # edit from one version's package and encrypt it with a key from a
        # different version without needing to know the key in advance.
        donor_key_row = ttk.Frame(prop_edit)
        donor_key_row.pack(fill="x", pady=(4, 0))
        self.use_donor_key_var = tk.BooleanVar(value=False)
        ttk.Checkbutton(
            donor_key_row,
            text="Encrypt with key from another UPK:",
            variable=self.use_donor_key_var,
        ).pack(side="left")
        self.donor_key_path_var = tk.StringVar(value="(none selected)")
        tk.Label(donor_key_row, textvariable=self.donor_key_path_var,
                 bg="#101114", fg="#a8b4cc", anchor="w").pack(side="left", fill="x", expand=True, padx=(6, 6))
        ttk.Button(donor_key_row, text="Pick Donor UPK…",
                   command=self._pick_donor_key_upk).pack(side="left")

        # ── Action buttons ───────────────────────────────────────────────────
        edit_row = ttk.Frame(prop_edit)
        edit_row.pack(fill="x", pady=(6, 0))
        ttk.Label(edit_row, text="New Value:").pack(side="left")
        self.property_edit_var = tk.StringVar()
        self.property_edit_entry = ttk.Entry(edit_row, textvariable=self.property_edit_var)
        self.property_edit_entry.pack(side="left", fill="x", expand=True, padx=(6, 6))
        ttk.Button(edit_row, text="Apply Property Edit", command=self.apply_property_edit).pack(side="left")
        ttk.Button(edit_row, text="Rename Export FName", command=self.rename_export_fname).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Import Donor Names", command=self.import_donor_names).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Import Donor Exports", command=self.import_donor_exports).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Replace Export From Donor", command=self.replace_export_from_donor).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Save Re-Encrypted UPK", command=self.save_reencrypted_upk).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Save Decrypted UPK", command=self.save_decrypted_upk).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Set DLLBind", command=self.set_dll_bind).pack(side="left", padx=(6, 0))
        ttk.Button(edit_row, text="Verify Package", command=self.verify_current_package).pack(side="left", padx=(6, 0))

        raw_frame = ttk.Frame(right_notebook)
        right_notebook.add(raw_frame, text="Raw Data")
        self.raw_text = tk.Text(raw_frame, wrap="none", bg="#111318", fg="#e8ecf4", insertbackground="#ffffff")
        self._pack_text_with_scrollbars(raw_frame, self.raw_text)

        status = tk.Label(self.root, textvariable=self.status_var, anchor="w", bg="#0b0c0f", fg="#dbe6ff", padx=12, pady=6)
        status.pack(fill="x", side="bottom")

    def _make_tree(self, parent, columns: Tuple[str, ...], widths: Tuple[int, ...]):
        frame = ttk.Frame(parent)
        tree = ttk.Treeview(frame, columns=columns, show="tree headings")
        tree.heading("#0", text="#")
        tree.column("#0", width=70, anchor="w")
        for column, width in zip(columns, widths):
            tree.heading(column, text=column.title())
            tree.column(column, width=width, anchor="w")
        vsb = ttk.Scrollbar(frame, orient="vertical", command=tree.yview)
        hsb = ttk.Scrollbar(frame, orient="horizontal", command=tree.xview)
        tree.configure(yscrollcommand=vsb.set, xscrollcommand=hsb.set)
        tree.grid(row=0, column=0, sticky="nsew")
        vsb.grid(row=0, column=1, sticky="ns")
        hsb.grid(row=1, column=0, sticky="ew")
        frame.rowconfigure(0, weight=1)
        frame.columnconfigure(0, weight=1)
        return tree

    def _pack_text_with_scrollbars(self, frame, text):
        vsb = ttk.Scrollbar(frame, orient="vertical", command=text.yview)
        hsb = ttk.Scrollbar(frame, orient="horizontal", command=text.xview)
        text.configure(yscrollcommand=vsb.set, xscrollcommand=hsb.set)
        text.grid(row=0, column=0, sticky="nsew")
        vsb.grid(row=0, column=1, sticky="ns")
        hsb.grid(row=1, column=0, sticky="ew")
        frame.rowconfigure(0, weight=1)
        frame.columnconfigure(0, weight=1)

    def _load_rlsdk(self):
        candidates = [
            self.script_dir / "RLSDK.zip",
            Path.cwd() / "RLSDK.zip",
            self.script_dir / "SDK.zip",
            Path.cwd() / "SDK.zip",
        ]
        for candidate in candidates:
            if candidate.exists():
                try:
                    self.sdk_db = parse_rlsdk_database(candidate)
                    self.sdk_path = candidate
                    self.sdk_var.set(f"RLSDK: {candidate}")
                    return
                except Exception as exc:
                    self.sdk_var.set(f"RLSDK: failed to load ({exc})")
                    return
        self.sdk_var.set("RLSDK: not found (put RLSDK.zip next to the script)")

    def _update_drop_label(self):
        if self.drop_target.enabled:
            self.drop_label.config(text="Drop an encrypted .upk anywhere on this window or use Open UPK")
        elif sys.platform == "win32":
            self.drop_label.config(text="Open UPK is ready. Drag/drop initializes after the window is fully created.")
        else:
            self.drop_label.config(text="Open UPK is available. Native drag/drop is enabled on Windows.")

    def set_status(self, text: str):
        self.status_var.set(text)
        self.root.update_idletasks()

    def open_file(self):
        path = filedialog.askopenfilename(filetypes=[("Unreal Packages", "*.upk"), ("All Files", "*.*")])
        if path:
            self.load_input(Path(path))

    def _on_drop_files(self, files: List[str]):
        for file_path in files:
            path = Path(file_path)
            if path.suffix.lower() == ".upk":
                self.load_input(path)
                return
        messagebox.showerror("Invalid file", "Drop a .upk file.")

    def load_input(self, input_path: Path):
        self.selected_input_path = input_path
        self.set_status(f"Hashing {input_path.name}...")
        threading.Thread(target=self._load_input_worker, args=(input_path,), daemon=True).start()

    def _load_input_worker(self, input_path: Path):
        try:
            sha1 = sha1_file(input_path)
            self.sha1_memory[str(input_path)] = {"sha1": sha1}
            self.root.after(0, lambda: self._refresh_history())
            self.root.after(0, lambda: self.set_status(f"Opening {input_path.name}..."))
            decrypted_path, package, provider, keys_path, was_encrypted = resolve_input_package(input_path, self.decrypted_dir, self.script_dir)
            label = keys_path if keys_path is not None else "not required"
            self.root.after(0, lambda: self.keys_var.set(f"Keys: {label}"))
            if decrypted_path != input_path:
                self.root.after(0, lambda: self.set_status(f"Parsing {decrypted_path.name}..."))
            self.root.after(0, lambda: self._apply_loaded_package(input_path, sha1, keys_path or Path(""), decrypted_path, package, provider))
        except Exception as exc:
            details = "".join(traceback.format_exception_only(type(exc), exc)).strip()
            self.root.after(0, lambda: self._show_error(details))

    def _refresh_history(self):
        for item in self.history_tree.get_children():
            self.history_tree.delete(item)
        for path, record in self.sha1_memory.items():
            self.history_tree.insert("", "end", text=Path(path).name, values=(record["sha1"],))

    def _clear_tree(self, tree: ttk.Treeview):
        for item in tree.get_children():
            tree.delete(item)

    def _apply_loaded_package(self, input_path: Path, sha1: str, keys_path: Path, decrypted_path: Path, package: ParsedPackage, provider: Optional[DecryptionProvider]):
        self.package = package
        self.current_provider = provider
        self.current_keys_path = keys_path if str(keys_path) else None
        self.current_encrypted_input_path = input_path if provider is not None else None
        self.current_original_sha1 = sha1
        self.original_var.set(f"Original: {input_path}")
        self.sha1_var.set(f"SHA-1: {sha1}")
        self.decrypted_var.set(f"Decrypted: {decrypted_path}")
        self.keys_var.set(f"Keys: {keys_path if str(keys_path) else 'not required'}")
        self._populate_summary()
        self._populate_content_tree()
        self._populate_exports_tree()
        self._populate_imports_tree()
        self._populate_names_tree()
        self.details_text.delete("1.0", "end")
        self.raw_text.delete("1.0", "end")
        self._clear_tree(self.properties_tree)
        self.current_export = None
        self.current_property = None
        self.property_edit_var.set("")
        self.property_info_var.set("Select a property to edit")
        self.current_properties = []
        self.details_text.insert("1.0", self._format_package_overview())
        self.set_status(f"Loaded {input_path.name}")

    def _populate_summary(self):
        self._clear_tree(self.summary_tree)
        if not self.package:
            return
        s = self.package.summary
        rows = [
            ("FileVersion", s.file_version),
            ("LicenseeVersion", s.licensee_version),
            ("TotalHeaderSize", s.total_header_size),
            ("FolderName", s.folder_name),
            ("PackageFlags", hex(s.package_flags)),
            ("NameCount", s.name_count),
            ("NameOffset", s.name_offset),
            ("ExportCount", s.export_count),
            ("ExportOffset", s.export_offset),
            ("ImportCount", s.import_count),
            ("ImportOffset", s.import_offset),
            ("DependsOffset", s.depends_offset),
            ("CompressionFlags", hex(s.compression_flags)),
            ("EngineVersion", s.engine_version),
            ("CookerVersion", s.cooker_version),
            ("Guid", f"{s.guid[0]:08X}-{s.guid[1]:08X}-{s.guid[2]:08X}-{s.guid[3]:08X}"),
            ("Generations", len(s.generations)),
        ]
        for key, value in rows:
            self.summary_tree.insert("", "end", text=key, values=(value,))

    def _populate_content_tree(self):
        self._clear_tree(self.content_tree)
        self.current_content_map.clear()
        if not self.package:
            return
        children: Dict[int, List[ExportEntry]] = {}
        for export in self.package.exports:
            children.setdefault(export.outer_index, []).append(export)
        roots = [export for export in self.package.exports if export.outer_index == 0 and export.class_index != 0]
        for export in roots:
            self._insert_content_export("", export, children)

    def _insert_content_export(self, parent: str, export: ExportEntry, children: Dict[int, List[ExportEntry]]):
        iid = f"export:{export.table_index}"
        name = self.package.resolve_name(export.object_name)
        self.current_content_map[iid] = export
        path = self.package.resolve_object_path(export.table_index + 1)
        node = self.content_tree.insert(parent, "end", iid=iid, text=name, values=(self.package.export_class_name(export), path), open=False)
        for child in children.get(export.table_index + 1, []):
            self._insert_content_export(node, child, children)

    def _populate_exports_tree(self):
        self._clear_tree(self.exports_tree)
        self.current_export_map.clear()
        if not self.package:
            self.exports_placeholder_count_var.set("")
            return
        hide_placeholders = bool(self.hide_placeholder_exports_var.get())
        placeholder_count = 0
        shown = 0
        for export in self.package.exports:
            is_placeholder = self.package.is_placeholder_export(export)
            if is_placeholder:
                placeholder_count += 1
                if hide_placeholders:
                    continue
            iid = f"export:{export.table_index}"
            self.current_export_map[iid] = export
            tags = ("placeholder",) if is_placeholder else ()
            class_label = "Class (placeholder)" if is_placeholder else self.package.export_class_name(export)
            self.exports_tree.insert(
                "",
                "end",
                iid=iid,
                text=str(export.table_index),
                values=(
                    self.package.resolve_name(export.object_name),
                    class_label,
                    self.package.resolve_object_ref(export.outer_index),
                    export.serial_size,
                    export.serial_offset,
                ),
                tags=tags,
            )
            shown += 1
        if placeholder_count == 0:
            self.exports_placeholder_count_var.set(f"{shown} exports, no placeholders")
        elif hide_placeholders:
            self.exports_placeholder_count_var.set(
                f"{shown} shown, {placeholder_count} placeholders hidden"
            )
        else:
            self.exports_placeholder_count_var.set(
                f"{shown} exports ({placeholder_count} placeholders, marked gray)"
            )

    def _populate_imports_tree(self):
        self._clear_tree(self.imports_tree)
        self.current_import_map.clear()
        if not self.package:
            return
        for imp in self.package.imports:
            iid = f"import:{imp.table_index}"
            self.current_import_map[iid] = imp
            self.imports_tree.insert(
                "",
                "end",
                iid=iid,
                text=str(imp.table_index),
                values=(
                    self.package.resolve_name(imp.object_name),
                    self.package.resolve_name(imp.class_name),
                    self.package.resolve_name(imp.class_package),
                    self.package.resolve_object_ref(imp.outer_index),
                ),
            )

    def _populate_names_tree(self):
        self._clear_tree(self.names_tree)
        self.current_name_map.clear()
        # Reset the inline rename row so stale text from a previous package or
        # selection doesn't carry over after a reload.
        if hasattr(self, "name_edit_var"):
            self.name_edit_var.set("")
        if hasattr(self, "name_edit_info_var"):
            self.name_edit_info_var.set("Select a name to rename")
        if not self.package:
            return
        for name in self.package.names:
            iid = f"name:{name.index}"
            self.current_name_map[iid] = name
            self.names_tree.insert("", "end", iid=iid, text=str(name.index), values=(name.name, hex(name.flags)))

    def _format_package_overview(self) -> str:
        if not self.package:
            return ""
        s = self.package.summary
        return "\n".join([
            f"Package: {self.package.file_path.name}",
            f"Names: {len(self.package.names)}",
            f"Imports: {len(self.package.imports)}",
            f"Exports: {len(self.package.exports)}",
            f"File Version: {s.file_version}",
            f"Licensee Version: {s.licensee_version}",
            f"Header Size: {s.total_header_size}",
            f"Package Flags: {hex(s.package_flags)}",
            f"Compression Flags: {hex(s.compression_flags)}",
            f"Engine Version: {s.engine_version}",
            f"Cooker Version: {s.cooker_version}",
        ])

    def _populate_properties(self, export: ExportEntry):
        self._clear_tree(self.properties_tree)
        self.current_export = export
        self.current_property = None
        self.property_edit_var.set("")
        self.property_info_var.set("Select a property to edit")
        self.current_properties = parse_serialized_properties(self.package, export, None) if self.package else []
        if not self.current_properties:
            self.properties_tree.insert("", "end", text="No serialized property tags parsed", values=("", "", "", "", "", ""))
            return
        for prop in self.current_properties:
            label = f"{prop.index}: {prop.name}"
            iid = f"prop:{prop.index}"
            self.properties_tree.insert("", "end", iid=iid, text=label, values=(prop.declared_type, prop.tag_type, prop.owner_type, prop.size, prop.array_index, prop.value))

    def _show_export(self, export: ExportEntry):
        class_name = self.package.export_class_name(export)
        raw = self.package.object_data(export)
        preview = raw[:HEX_PREVIEW_LIMIT]
        self._populate_properties(export)
        details = [
            f"Type: Export[{export.table_index}]",
            f"Name: {self.package.resolve_name(export.object_name)}",
            f"Path: {self.package.resolve_object_path(export.table_index + 1)}",
            f"Class: {class_name}",
            f"ClassIndex: {export.class_index} ({self.package.resolve_object_ref(export.class_index)})",
            f"SuperIndex: {export.super_index} ({self.package.resolve_object_ref(export.super_index)})",
            f"OuterIndex: {export.outer_index} ({self.package.resolve_object_ref(export.outer_index)})",
            f"ArchetypeIndex: {export.archetype_index} ({self.package.resolve_object_ref(export.archetype_index)})",
            f"ObjectFlags: {hex(export.object_flags)}",
            f"ExportFlags: {hex(export.export_flags)}",
            f"PackageFlags: {hex(export.package_flags)}",
            f"SerialSize: {export.serial_size}",
            f"SerialOffset: {export.serial_offset}",
            f"PropertiesParsed: {len(self.current_properties)}",
            f"NetObjects: {export.net_objects}",
            f"PackageGuid: {export.package_guid[0]:08X}-{export.package_guid[1]:08X}-{export.package_guid[2]:08X}-{export.package_guid[3]:08X}",
        ]
        # ── DLLBind info (UClass exports only) ──────────────────────────────
        if is_uclass_export(self.package, export):
            dllbind_result = find_uclass_dllbind_fstring_offset(raw)
            if dllbind_result is not None:
                dllbind_offset, dllbind_name = dllbind_result
                details.append(
                    f"DLLBind: {dllbind_name!r}  "
                    f"(FString at serial+0x{dllbind_offset:X})"
                    if dllbind_name else
                    f"DLLBind: (none)  (FString at serial+0x{dllbind_offset:X})"
                )
            else:
                details.append("DLLBind: <could not locate DLLBindName field>")
        self.details_text.delete("1.0", "end")
        self.details_text.insert("1.0", "\n".join(details))
        self.raw_text.delete("1.0", "end")
        if raw:
            header = f"Object data preview: {len(preview)} / {len(raw)} bytes"
            if len(raw) > HEX_PREVIEW_LIMIT:
                header += f"\nPreview truncated at {HEX_PREVIEW_LIMIT} bytes\n"
            else:
                header += "\n"
            self.raw_text.insert("1.0", header + "\n" + format_hex_preview(preview, export.serial_offset))
        else:
            self.raw_text.insert("1.0", "No serial data available for this export.")

    def _show_import(self, imp: ImportEntry):
        details = [
            f"Type: Import[{imp.table_index}]",
            f"Name: {self.package.resolve_name(imp.object_name)}",
            f"Path: {self.package.resolve_object_path(-imp.table_index - 1)}",
            f"ClassName: {self.package.resolve_name(imp.class_name)}",
            f"ClassPackage: {self.package.resolve_name(imp.class_package)}",
            f"OuterIndex: {imp.outer_index} ({self.package.resolve_object_ref(imp.outer_index)})",
        ]
        self.details_text.delete("1.0", "end")
        self.details_text.insert("1.0", "\n".join(details))
        self.raw_text.delete("1.0", "end")
        self.raw_text.insert("1.0", "Imports do not contain local serial data in the package.")
        self._clear_tree(self.properties_tree)
        self.current_export = None
        self.current_property = None
        self.property_edit_var.set("")
        self.property_info_var.set("Select a property to edit")
        self.current_properties = []

    def _show_name(self, name: NameEntry):
        self.details_text.delete("1.0", "end")
        self.details_text.insert("1.0", f"Type: Name[{name.index}]\nName: {name.name}\nFlags: {hex(name.flags)}")
        self.raw_text.delete("1.0", "end")
        self.raw_text.insert("1.0", "")
        self._clear_tree(self.properties_tree)
        self.current_properties = []

    def _on_content_select(self, _event):
        selection = self.content_tree.selection()
        if not selection:
            return
        iid = selection[0]
        export = self.current_content_map.get(iid)
        if export:
            self._show_export(export)

    def _on_exports_select(self, _event):
        selection = self.exports_tree.selection()
        if not selection:
            return
        export = self.current_export_map.get(selection[0])
        if export:
            self._show_export(export)

    def _on_imports_select(self, _event):
        selection = self.imports_tree.selection()
        if not selection:
            return
        imp = self.current_import_map.get(selection[0])
        if imp:
            self._show_import(imp)

    def _on_names_select(self, _event):
        selection = self.names_tree.selection()
        if not selection:
            return
        name = self.current_name_map.get(selection[0])
        if name:
            self._show_name(name)
            # Pre-fill the rename entry with the current text and update the
            # info label so the user knows exactly which entry will be edited.
            self.name_edit_var.set(name.name)
            self.name_edit_info_var.set(
                f"Name[{name.index}]  flags={hex(name.flags)}  ({len(name.name)} chars)"
            )

    def _on_names_double_click(self, _event):
        # Convenience: double-click jumps focus into the rename field with the
        # current text pre-selected so the user can just start typing.
        if not self.names_tree.selection():
            return
        self.name_edit_entry.focus_set()
        self.name_edit_entry.select_range(0, "end")
        self.name_edit_entry.icursor("end")

    def _on_property_select(self, _event):
        selection = self.properties_tree.selection()
        if not selection:
            return
        iid = selection[0]
        if not iid.startswith("prop:"):
            return
        try:
            index = int(iid.split(":", 1)[1])
        except Exception:
            return
        if 0 <= index < len(self.current_properties):
            prop = self.current_properties[index]
            self.current_property = prop
            self.property_edit_var.set(prop.value)
            self.property_info_var.set(f"{prop.name} | {prop.tag_type} | file+0x{(self.current_export.serial_offset + (prop.value_offset - 1 if prop.tag_type == 'BoolProperty' and prop.bool_value is not None else prop.value_offset)):X}")

    def apply_property_edit(self):
        if not self.package or not self.current_export or not self.current_property:
            messagebox.showwarning("UPK GUI", "Select an export and a property first.")
            return
        try:
            export_index = self.current_export.table_index
            prop_name = self.current_property.name
            prop_array_index = self.current_property.array_index
            new_bytes = apply_property_edit_bytes(self.package, self.current_export, self.current_property, self.property_edit_var.get())
            self.package = parse_decrypted_package_bytes(self.package.file_path, new_bytes)
            self.current_export = self.package.exports[export_index]
            self._show_export(self.current_export)
            for i, prop in enumerate(self.current_properties):
                if prop.name == prop_name and prop.array_index == prop_array_index:
                    iid = f"prop:{i}"
                    if self.properties_tree.exists(iid):
                        self.properties_tree.selection_set(iid)
                        self.properties_tree.focus(iid)
                        self.current_property = prop
                        self.property_edit_var.set(prop.value)
                        break
            self.set_status(f"Edited {prop_name}")
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def _pick_donor_key_upk(self):
        """Let the user select an encrypted UPK whose decryption key will be
        used when 'Save Re-Encrypted UPK' is next invoked (while the checkbox
        is ticked).  The key is resolved from keys.txt at save time, not here,
        so any recently-updated keys.txt is automatically picked up."""
        path = filedialog.askopenfilename(
            title="Select donor encrypted UPK (to source the encryption key)",
            filetypes=[("UPK files", "*.upk"), ("All files", "*.*")],
            initialdir=str(self.script_dir),
        )
        if not path:
            return
        self.donor_key_upk_path = Path(path)
        self.donor_key_path_var.set(self.donor_key_upk_path.name)
        # Auto-enable the checkbox when the user picks a file.
        self.use_donor_key_var.set(True)
        self.set_status(f"Donor key UPK set: {self.donor_key_upk_path.name}")

    def save_reencrypted_upk(self):
        source_path = self.current_encrypted_input_path or self.selected_input_path
        if not self.package or not source_path or not self.current_provider:
            messagebox.showwarning("UPK GUI", "Load an encrypted package first.")
            return

        # ── Resolve override key from donor UPK if requested ─────────────────
        override_key: Optional[bytes] = None
        if self.use_donor_key_var and self.use_donor_key_var.get():
            donor_path = self.donor_key_upk_path
            if not donor_path or not donor_path.exists():
                messagebox.showwarning(
                    "UPK GUI",
                    "The 'Encrypt with key from another UPK' option is enabled but no valid "
                    "donor UPK has been selected.\n\n"
                    "Click 'Pick Donor UPK…' to choose one, or uncheck the option to use "
                    "the original package's own key.",
                )
                return
            # Build a provider from the same keys.txt so we can try all known keys.
            keys_path = self.current_keys_path
            try:
                donor_provider = DecryptionProvider(str(keys_path) if keys_path and keys_path.exists() else None)
            except Exception as exc:
                messagebox.showerror("UPK GUI", f"Could not load keys.txt for donor key search:\n{exc}")
                return
            try:
                self.set_status(f"Finding key for donor UPK: {donor_path.name}…")
                override_key = find_key_for_encrypted_upk(donor_path, donor_provider)
                import base64 as _b64
                self.set_status(f"Donor key found ({_b64.b64encode(override_key).decode()[:16]}…), saving…")
            except Exception as exc:
                messagebox.showerror(
                    "UPK GUI",
                    f"Could not find a working decryption key for the donor UPK:\n{donor_path.name}\n\n{exc}\n\n"
                    "Make sure keys.txt contains a key that can decrypt the selected donor package.",
                )
                return

        default_name = f"{source_path.stem}_edited{source_path.suffix}"
        initial_dir = str(self.script_dir / "ReEncrypted")
        out_path = filedialog.asksaveasfilename(
            title="Save re-encrypted UPK",
            defaultextension=".upk",
            filetypes=[("UPK files", "*.upk"), ("All files", "*.*")],
            initialdir=initial_dir,
            initialfile=default_name,
        )
        if not out_path:
            return
        try:
            saved = build_reencrypted_package(
                source_path,
                bytes(self.package.file_bytes),
                self.current_provider,
                Path(out_path),
                override_key=override_key,
            )
            new_sha1 = sha1_file(saved)
            patch_note = "RocketLeague.exe patch skipped (file not found)."
            exe_path = self.script_dir / "RocketLeague.exe"
            if self.current_original_sha1 and exe_path.exists():
                replaced = patch_sha1_in_exe(exe_path, self.current_original_sha1, new_sha1)
                patch_note = f"Patched {exe_path.name}: {replaced} occurrence(s)."
            elif self.current_original_sha1 and not exe_path.exists():
                patch_note = f"RocketLeague.exe patch skipped ({exe_path.name} not found)."
            key_note = ""
            if override_key is not None:
                import base64 as _b64
                key_note = f"\nEncryption key sourced from: {self.donor_key_upk_path.name}\n(key prefix: {_b64.b64encode(override_key).decode()[:16]}…)"
            self.set_status(f"Saved {saved.name}")
            prompt = f"Saved re-encrypted UPK:\n{saved}\n\nSHA-1: {new_sha1}\n{patch_note}{key_note}\n\nOpen it now?"
            if messagebox.askyesno("UPK GUI", prompt):
                self._load_selected_file(saved)
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def import_donor_names(self):
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        donor_path = filedialog.askopenfilename(
            title="Select donor UPK",
            filetypes=[("UPK files", "*.upk"), ("All files", "*.*")],
            initialdir=str(self.script_dir),
        )
        if not donor_path:
            return
        try:
            donor_input = Path(donor_path)
            _donor_decrypted, donor_package, _donor_provider, _donor_keys, _donor_was_encrypted = resolve_input_package(donor_input, self.decrypted_dir, self.script_dir)
            name_to_import = simpledialog.askstring("Import Donor Names", "Name to import from donor package (leave blank for all donor names):", parent=self.root)
            selected = [name_to_import.strip()] if name_to_import and name_to_import.strip() else None
            merged = import_donor_names(self.package, donor_package, selected)
            self.package = merged
            self._apply_loaded_package(self.selected_input_path or self.package.file_path, self.current_original_sha1 or "-", self.current_keys_path or Path("keys.txt"), self.package.file_path, merged, self.current_provider)
            added_names = getattr(merged, '_merge_added_names', 0)
            self.set_status(f"Imported donor names: +{added_names} names")
            messagebox.showinfo("UPK GUI", f"Imported donor names: +{added_names}")
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def import_donor_exports(self):
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        donor_path = filedialog.askopenfilename(
            title="Select donor UPK",
            filetypes=[("UPK files", "*.upk"), ("All files", "*.*")],
            initialdir=str(self.script_dir),
        )
        if not donor_path:
            return
        try:
            donor_input = Path(donor_path)
            donor_decrypted, donor_package, _donor_provider, _donor_keys, donor_was_encrypted = resolve_input_package(donor_input, self.decrypted_dir, self.script_dir)
            # Use the original (encrypted) input filename as the donor package
            # name, not the decrypted working copy's stem - the game looks up
            # the file by its deployed name in the cooked content folder.
            donor_pkg_name = donor_input.stem
            merged = merge_donor_exports_as_imports(self.package, donor_package, donor_pkg_name)
            self.package = merged
            self._apply_loaded_package(self.selected_input_path or self.package.file_path, self.current_original_sha1 or "-", self.current_keys_path or Path("keys.txt"), self.package.file_path, merged, self.current_provider)
            added_imports = getattr(merged, '_merge_added_imports', 0)
            added_names = getattr(merged, '_merge_added_names', 0)
            resolved_name = getattr(merged, '_merge_donor_package_name', donor_pkg_name)
            self.set_status(f"Imported donor exports as '{resolved_name}': +{added_imports} imports, +{added_names} names")
            messagebox.showinfo(
                "UPK GUI",
                f"Imported exports from {donor_input.name}\n\n"
                f"Donor package name (used in import paths): {resolved_name}\n"
                f"Added imports: {added_imports}\n"
                f"Added names: {added_names}\n\n"
                "All donor exports have been re-imported under a Core.Package "
                f"\"{resolved_name}\" import, so the engine will attempt to load "
                f"{resolved_name}.upk when this package is loaded.\n\n"
                "IMPORTANT: For the references to actually resolve at runtime, "
                f"deploy {donor_input.name} into the same cooked content folder "
                "where the game looks for .upk files (typically alongside the "
                "other Rocket League .upk files in CookedPCConsole). If the "
                "donor file isn't on disk where the engine can find it, the "
                "imports will fail to resolve and dependent objects will be "
                "missing at load time.\n\n"
                "You can now use the new imported object paths in ObjectProperty "
                "edits and the new imported names in NameProperty edits.",
            )
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def replace_export_from_donor(self):
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        if not self.current_export:
            messagebox.showwarning("UPK GUI", "Select the target export first.")
            return
        donor_path = filedialog.askopenfilename(
            title="Select donor UPK",
            filetypes=[("UPK files", "*.upk"), ("All files", "*.*")],
            initialdir=str(self.script_dir),
        )
        if not donor_path:
            return
        try:
            donor_input = Path(donor_path)
            _donor_decrypted, donor_package, _donor_provider, _donor_keys, _donor_was_encrypted = resolve_input_package(donor_input, self.decrypted_dir, self.script_dir)
            target_path = self.package.resolve_object_path(self.current_export.table_index + 1)
            donor_export_path = simpledialog.askstring(
                "Replace Export From Donor",
                "Donor export path:",
                parent=self.root,
            )
            if not donor_export_path or not donor_export_path.strip():
                return
            replaced = replace_export_with_donor_export(self.package, donor_package, target_path, donor_export_path.strip())
            self.package = replaced
            self._apply_loaded_package(self.selected_input_path or self.package.file_path, self.current_original_sha1 or "-", self.current_keys_path or Path("keys.txt"), self.package.file_path, replaced, self.current_provider)
            note = getattr(replaced, '_replace_note', '')
            self.set_status(f"Replaced export from donor: {Path(donor_path).name}")
            messagebox.showinfo(
                "UPK GUI",
                f"Replaced target export:\n{target_path}\n\nWith donor export:\n{donor_export_path.strip()}\n\n{note}",
            )
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))


    def rename_export_fname(self):
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        if not self.current_export:
            messagebox.showwarning("UPK GUI", "Select the export you want to rename first.")
            return
        export_index = self.current_export.table_index
        current_name = self.package.resolve_name(self.current_export.object_name)
        new_name = simpledialog.askstring(
            "Rename Export FName",
            "New FName for this export.\n"
            "- Use 'BaseName' or 'BaseName_<N>' (e.g. 'MyMesh_3').\n"
            "- An existing name is reused; a new base name is appended to the name table.\n"
            f"\nCurrent: {current_name}",
            initialvalue=current_name,
            parent=self.root,
        )
        if new_name is None:
            return
        new_name = new_name.strip()
        if not new_name:
            return
        if new_name == current_name:
            self.set_status("Rename skipped (name unchanged)")
            return
        try:
            renamed = rename_export_fname(self.package, self.current_export, new_name)
            self.package = renamed
            self._apply_loaded_package(
                self.selected_input_path or self.package.file_path,
                self.current_original_sha1 or "-",
                self.current_keys_path or Path("keys.txt"),
                self.package.file_path,
                renamed,
                self.current_provider,
            )
            # Re-select the renamed export so the user can immediately see/save it.
            if 0 <= export_index < len(self.package.exports):
                self.current_export = self.package.exports[export_index]
                self._show_export(self.current_export)
                iid = f"export:{export_index}"
                if self.exports_tree.exists(iid):
                    self.exports_tree.selection_set(iid)
                    self.exports_tree.focus(iid)
                    self.exports_tree.see(iid)
            added = getattr(renamed, '_rename_added_names', 0)
            self.set_status(f"Renamed export[{export_index}] -> {new_name} (+{added} name)")
            messagebox.showinfo(
                "UPK GUI",
                f"Renamed export[{export_index}]:\n  {current_name}\n  ->\n  {new_name}\n\n"
                f"Names added to package: {added}\n\n"
                "Use 'Save Decrypted UPK' (for plain packages) or 'Save Re-Encrypted UPK' "
                "(for encrypted packages) to write the change to disk.",
            )
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def rename_selected_name(self):
        """Rewrite the text of the currently-selected name table entry.

        Triggered from the Names tab edit row (button or <Return> in the
        entry). Reads the new text from self.name_edit_var, calls the
        rename_name_entry helper which rebuilds the package's header (names
        blob + offsets + serial_offsets), and reloads the GUI from the
        rebuilt bytes. The same name index is re-selected after reload so
        the user can immediately verify the change.
        """
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        selection = self.names_tree.selection()
        if not selection:
            messagebox.showwarning("UPK GUI", "Select a name in the Names list first.")
            return
        name_entry = self.current_name_map.get(selection[0])
        if name_entry is None:
            messagebox.showwarning("UPK GUI", "Could not resolve the selected name.")
            return
        new_text = self.name_edit_var.get().strip()
        if not new_text:
            messagebox.showwarning("UPK GUI", "New name text cannot be empty.")
            return
        if new_text == name_entry.name:
            self.set_status("Rename skipped (name unchanged)")
            return
        target_index = name_entry.index
        try:
            renamed = rename_name_entry(self.package, target_index, new_text)
            self.package = renamed
            self._apply_loaded_package(
                self.selected_input_path or self.package.file_path,
                self.current_original_sha1 or "-",
                self.current_keys_path or Path("keys.txt"),
                self.package.file_path,
                renamed,
                self.current_provider,
            )
            # Re-select the renamed entry. _populate_names_tree rebuilt the
            # tree from scratch, so the iid is regenerated but uses the same
            # name:<index> pattern.
            iid = f"name:{target_index}"
            if self.names_tree.exists(iid):
                self.names_tree.selection_set(iid)
                self.names_tree.focus(iid)
                self.names_tree.see(iid)
            delta = getattr(renamed, '_name_size_delta', 0)
            old_text = getattr(renamed, '_name_rename_old', name_entry.name)
            sign = "+" if delta >= 0 else ""
            self.set_status(
                f"Renamed name[{target_index}]: {old_text!r} -> {new_text!r} "
                f"({sign}{delta} bytes)"
            )
            messagebox.showinfo(
                "UPK GUI",
                f"Renamed name table entry [{target_index}]:\n"
                f"  {old_text!r}\n"
                f"  ->\n"
                f"  {new_text!r}\n\n"
                f"Names blob delta: {sign}{delta} bytes\n"
                "Header offsets and export serial_offsets were rebuilt to "
                "match the new size.\n\n"
                "Use 'Save Decrypted UPK' (for plain packages) or "
                "'Save Re-Encrypted UPK' (for encrypted packages) to write "
                "the change to disk.",
            )
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def set_dll_bind(self):
        """Inject or change the DLLBind DLL name on the currently selected UClass export.

        DLLBind causes the Unreal Engine to call LoadLibrary on the given DLL
        name immediately when the class is loaded from the package.  This is
        the standard injection point used by Rocket League mod frameworks such
        as BakkesMod and CodeRed.

        The DLL name is stored as an FString (DLLBindName) at the very end of
        the UClass serial body.  This method locates that FString, replaces it
        with the user-supplied value, and adjusts all relevant package offsets
        so the result is a valid, save-able package.

        Requirements:
          • The selected export must be a class definition (class_index → Class).
          • The DLL name must be pure ASCII (as required by the UE3 FString
            encoding used for DLLBindName in cooked packages).
          • No path separators — supply only the DLL filename, e.g. 'MyMod.dll'.
        """
        if not self.package or not self.current_export:
            messagebox.showwarning("UPK GUI", "Select a UClass export first.")
            return

        if not is_uclass_export(self.package, self.current_export):
            cls = self.package.export_class_name(self.current_export)
            messagebox.showwarning(
                "UPK GUI",
                f"The selected export is a '{cls}', not a Class.\n\n"
                "DLLBind can only be set on class definition exports "
                "(exports whose class is 'Class' in the Exports tab).  "
                "Select a UClass export and try again.",
            )
            return

        raw = self.package.object_data(self.current_export)
        if not raw:
            messagebox.showerror("UPK GUI", "The selected UClass export has no serial data.")
            return

        dllbind_result = find_uclass_dllbind_fstring_offset(raw)
        if dllbind_result is None:
            messagebox.showerror(
                "UPK GUI",
                "Could not locate the DLLBindName field in this UClass export's serial data.\n\n"
                "The export's binary layout may be non-standard or the serial data may be "
                "truncated.  Check the Raw Data tab for the hex dump.",
            )
            return

        fstring_offset, current_dll_name = dllbind_result

        new_dll_name = simpledialog.askstring(
            "Set DLLBind",
            f"Enter the DLL filename to bind (ASCII only, no path separators).\n"
            f"Example: CodeRed.dll\n\n"
            f"Leave blank to remove an existing DLLBind.\n\n"
            f"Current DLLBind: {repr(current_dll_name) if current_dll_name else '(none)'}\n"
            f"FString at serial+0x{fstring_offset:X}",
            initialvalue=current_dll_name,
            parent=self.root,
        )
        if new_dll_name is None:
            return  # user cancelled

        new_dll_name = new_dll_name.strip()

        if new_dll_name == current_dll_name:
            self.set_status("DLLBind unchanged — no action taken.")
            return

        try:
            export_index = self.current_export.table_index
            export_name = self.package.resolve_name(self.current_export.object_name)
            new_bytes = set_uclass_dllbind_name(self.package, self.current_export, new_dll_name)
            self.package = parse_decrypted_package_bytes(self.package.file_path, new_bytes)
            self.current_export = self.package.exports[export_index]
            self._show_export(self.current_export)

            if new_dll_name:
                action_msg = f"Set DLLBind → '{new_dll_name}'"
                detail_msg = (
                    f"Export[{export_index}] '{export_name}' will now cause the engine "
                    f"to load '{new_dll_name}' via LoadLibrary when this package is opened."
                )
            else:
                action_msg = "Removed DLLBind"
                detail_msg = f"DLLBind was cleared on Export[{export_index}] '{export_name}'."

            self.set_status(action_msg)
            messagebox.showinfo(
                "UPK GUI",
                f"{action_msg}\n\n"
                f"{detail_msg}\n\n"
                "Use 'Save Re-Encrypted UPK' (for encrypted RL packages) or "
                "'Save Decrypted UPK' (for plain packages) to write the change to disk.",
            )
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))

    def save_decrypted_upk(self):
        """Save the current (possibly edited) decrypted package bytes to a .upk file.

        Use this when the loaded package is plain/decompressed (not encrypted).
        For encrypted Rocket League packages, use Save Re-Encrypted UPK instead.
        """
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        # Pick a sensible default location/name. If the original input was a
        # plain UPK, write next to it; otherwise drop into the decrypted_dir.
        source_path = self.selected_input_path or self.package.file_path
        if self.current_provider is not None:
            initial_dir = str(self.decrypted_dir)
            default_name = f"{source_path.stem}_edited_decrypted.upk"
        else:
            initial_dir = str(source_path.parent)
            default_name = f"{source_path.stem}_edited{source_path.suffix or '.upk'}"
        out_path = filedialog.asksaveasfilename(
            title="Save decrypted UPK",
            defaultextension=".upk",
            filetypes=[("UPK files", "*.upk"), ("All files", "*.*")],
            initialdir=initial_dir,
            initialfile=default_name,
        )
        if not out_path:
            return
        try:
            out = Path(out_path)
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(bytes(self.package.file_bytes))
            self.set_status(f"Saved {out.name}")
            note = ""
            if self.current_provider is not None:
                note = (
                    "\n\nNote: this is a decrypted/plain copy. The Rocket League game "
                    "loader expects the encrypted format - use 'Save Re-Encrypted UPK' "
                    "to produce a file the game will load."
                )
            prompt = f"Saved decrypted UPK:\n{out}{note}\n\nOpen it now?"
            if messagebox.askyesno("UPK GUI", prompt):
                self._load_selected_file(out)
        except Exception as exc:
            messagebox.showerror("UPK GUI", str(exc))


    def verify_current_package(self):
        """Run consistency checks on the loaded package and show a report dialog.

        Useful when an edited package freezes or crashes the game on load -
        the report will pinpoint which header invariant is violated. Read-only,
        does not modify or save the package.
        """
        if not self.package:
            messagebox.showwarning("UPK GUI", "Load a package first.")
            return
        try:
            findings = verify_package(self.package)
            report = format_verify_report(findings)
        except Exception as exc:
            messagebox.showerror("UPK GUI", f"Verification failed to run: {exc}")
            return

        # Show the report in a dedicated scrollable dialog so long reports
        # don't get clipped by messagebox's fixed sizing.
        dlg = tk.Toplevel(self.root)
        dlg.title("Package Verification Report")
        dlg.geometry("980x560")
        dlg.transient(self.root)
        text = tk.Text(dlg, wrap="word", bg="#111318", fg="#e8ecf4", insertbackground="#ffffff")
        vsb = ttk.Scrollbar(dlg, orient="vertical", command=text.yview)
        text.configure(yscrollcommand=vsb.set)
        text.grid(row=0, column=0, sticky="nsew")
        vsb.grid(row=0, column=1, sticky="ns")
        dlg.rowconfigure(0, weight=1)
        dlg.columnconfigure(0, weight=1)
        text.insert("1.0", report)
        text.tag_configure("error", foreground="#ff7676")
        text.tag_configure("warn", foreground="#ffd166")
        text.tag_configure("ok", foreground="#7be39a")
        for line_idx, line in enumerate(report.split("\n"), start=1):
            tag = None
            if line.startswith("[ERROR]"):
                tag = "error"
            elif line.startswith("[WARN]"):
                tag = "warn"
            elif line.startswith("[OK]"):
                tag = "ok"
            if tag is not None:
                text.tag_add(tag, f"{line_idx}.0", f"{line_idx}.end")
        text.configure(state="disabled")

        button_row = ttk.Frame(dlg)
        button_row.grid(row=1, column=0, columnspan=2, sticky="ew", padx=8, pady=6)

        def _copy():
            self.root.clipboard_clear()
            self.root.clipboard_append(report)
            self.set_status("Verification report copied to clipboard")

        ttk.Button(button_row, text="Copy to Clipboard", command=_copy).pack(side="left")
        ttk.Button(button_row, text="Close", command=dlg.destroy).pack(side="right")

        error_count = sum(1 for sev, _ in findings if sev == "ERROR")
        if error_count:
            self.set_status(f"Verification: {error_count} error(s) found")
        else:
            self.set_status("Verification: no errors found")


    def _show_error(self, details: str):
        self.set_status("Failed")
        messagebox.showerror("UPK GUI", details)


def main() -> int:
    parser = argparse.ArgumentParser(add_help=False)
    parser.parse_args()
    root = tk.Tk()
    App(root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())