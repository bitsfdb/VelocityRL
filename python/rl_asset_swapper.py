#!/usr/bin/env python3
import argparse
import base64
import importlib
import importlib.util
import io
import json
import os
import shutil
import struct
import sys
import traceback
import hashlib
import hmac
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Dict, Iterable, List, Optional, Sequence, Tuple


# Dummy imports for PyInstaller to include dependencies of dynamically loaded rl_upk_editor
if False:
    import concurrent.futures
    import ctypes
    import hashlib
    import zlib
    import re
    import zipfile
    from cryptography.hazmat.backends import default_backend
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes



@dataclass(frozen=True)
class Item:
    id: int
    product: str
    quality: str
    slot: str
    asset_package: str
    asset_path: str

    @property
    def package_stem(self) -> str:
        return Path(self.asset_package).stem

    @property
    def asset_parts(self) -> List[str]:
        return [p for p in self.asset_path.split(".") if p]

    @property
    def asset_base(self) -> str:
        parts = self.asset_parts
        return parts[0] if parts else self.package_stem.removesuffix("_SF")

    @property
    def thumbnail_package(self) -> str:
        return f"{self.asset_base}_T_SF.upk"

    @property
    def label(self) -> str:
        quality = f" / {self.quality}" if self.quality else ""
        slot = f" / {self.slot}" if self.slot else ""
        return f"[{self.id}] {self.product}{quality}{slot} ({self.asset_package})"


@dataclass
class SwapOptions:
    items_path: Path
    keys_path: Optional[Path]
    donor_dir: Path
    output_dir: Path
    key_source_dir: Optional[Path]
    include_thumbnails: bool
    preserve_header_offsets: bool
    overwrite: bool
    logger: Optional[Callable[[str], None]] = None


def script_dir() -> Path:
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent


def default_path(names: Sequence[str]) -> Path:
    here = script_dir()
    for name in names:
        candidates = [
            here / name,
            here.parent / "python" / name,
            here.parent / "resources" / "python" / name,
            here.parent / "resources" / name,
            here.parent.parent / "python" / name,
            here.parent.parent / "resources" / "python" / name,
            Path.cwd() / name,
            Path.cwd() / "python" / name,
            Path.cwd() / "resources" / "python" / name,
        ]
        if getattr(sys, "_MEIPASS", None):
            candidates.insert(0, Path(sys._MEIPASS) / name)
            
        for candidate in candidates:
            if candidate.exists():
                return candidate
    return here / names[0]


def import_rl_upk_editor():
    try:
        return importlib.import_module("rl_upk_editor")
    except Exception:
        pass

    here = script_dir()
    names = ["rl_upk_editor.py", "rl_upk_editor(1).py"]
    candidates = []
    
    for name in names:
        candidates.extend([
            here / name,
            here.parent / "python" / name,
            here.parent / "resources" / "python" / name,
            here.parent / "resources" / name,
            here.parent.parent / "python" / name,
            here.parent.parent / "resources" / "python" / name,
            Path.cwd() / name,
            Path.cwd() / "python" / name,
            Path.cwd() / "resources" / "python" / name,
        ])
        if getattr(sys, "_MEIPASS", None):
            candidates.insert(0, Path(sys._MEIPASS) / name)

    last_err = None
    for candidate in candidates:
        if not candidate.exists():
            continue
        try:
            spec = importlib.util.spec_from_file_location("rl_upk_editor", candidate)
            if spec is None or spec.loader is None:
                continue
            module = importlib.util.module_from_spec(spec)
            sys.modules["rl_upk_editor"] = module
            spec.loader.exec_module(module)
            return module
        except Exception as e:
            last_err = e
            continue

    if last_err:
        raise ImportError(f"Failed to load rl_upk_editor from {len(candidates)} candidates. Last error: {last_err}")
    raise ImportError("Could not find rl_upk_editor.py in any search path.")


def load_items(path: Path) -> List[Item]:
    raw = json.loads(path.read_text(encoding="utf-8-sig"))
    rows = raw.get("Items", raw if isinstance(raw, list) else [])
    out: List[Item] = []
    for row in rows:
        try:
            pkg = str(row.get("AssetPackage", "") or "")
            asset_path = str(row.get("AssetPath", "") or "")
            if not pkg or not asset_path:
                continue
            out.append(Item(
                id=int(row.get("ID", 0) or 0),
                product=str(row.get("Product", "") or ""),
                quality=str(row.get("Quality", "") or ""),
                slot=str(row.get("Slot", "") or ""),
                asset_package=pkg,
                asset_path=asset_path,
            ))
        except Exception:
            continue
    out.sort(key=lambda x: (x.slot.lower(), x.product.lower(), x.id))
    return out


def find_item(items: Sequence[Item], value: str, slot: str = "") -> Item:
    value = str(value).strip()
    rows = [x for x in items if not slot or x.slot.lower() == slot.lower()]
    if value.isdigit():
        wanted = int(value)
        matches = [x for x in rows if x.id == wanted]
    else:
        q = value.lower()
        matches = [x for x in rows if q in x.product.lower() or q in x.asset_package.lower() or q in x.asset_path.lower()]
    if not matches:
        raise ValueError(f"No item matched {value!r}" + (f" in slot {slot!r}" if slot else ""))
    if len(matches) > 1:
        # 1. Try exact matches on product name or package (handle .upk suffix)
        exact = []
        for x in matches:
            p_low = x.product.lower()
            pkg_low = x.asset_package.lower()
            val_low = value.lower()
            if p_low == val_low or pkg_low == val_low or pkg_low.removesuffix(".upk") == val_low:
                exact.append(x)
        
        if len(exact) == 1:
            return exact[0]
            
        # 2. If still ambiguous, check if they are all functionally identical (same package & path)
        unique_assets = {(x.asset_package.lower(), x.asset_path.lower()) for x in (exact if exact else matches)}
        if len(unique_assets) == 1:
            return (exact if exact else matches)[0]

        raise ValueError("Ambiguous item match:\n" + "\n".join(x.label for x in matches[:20]))
    return matches[0]


def add_pair(pairs: List[Tuple[str, str]], old: str, new: str) -> None:
    old = (old or "").strip()
    new = (new or "").strip()
    if not old or not new or old == new:
        return
    if (old, new) not in pairs:
        pairs.append((old, new))


def infer_name_pairs(target: Item, donor: Item) -> List[Tuple[str, str]]:
    pairs: List[Tuple[str, str]] = []
    donor_parts = donor.asset_parts
    target_parts = target.asset_parts
    if len(donor_parts) == len(target_parts):
        for old, new in zip(donor_parts, target_parts):
            add_pair(pairs, old, new)
    else:
        if donor_parts and target_parts:
            add_pair(pairs, donor_parts[0], target_parts[0])
            add_pair(pairs, donor_parts[-1], target_parts[-1])
        for old, new in zip(donor_parts, target_parts):
            add_pair(pairs, old, new)
    add_pair(pairs, donor.package_stem, target.package_stem)
    return pairs


def infer_thumbnail_pairs(target: Item, donor: Item) -> List[Tuple[str, str]]:
    return [
        (f"{donor.asset_base}_T", f"{target.asset_base}_T"),
        (f"{donor.asset_base}_T_SF", f"{target.asset_base}_T_SF"),
    ]


def clean_name(text: str) -> str:
    return str(text).split("\x00", 1)[0].strip()


def find_name_indices(package, name: str) -> Tuple[List[int], bool]:
    exact = [n.index for n in package.names if clean_name(n.name) == name]
    if exact:
        return exact, False
    q = name.lower()
    fuzzy = [n.index for n in package.names if clean_name(n.name).lower() == q]
    return fuzzy, bool(fuzzy)


def name_exists(package, name: str) -> bool:
    return bool(find_name_indices(package, name)[0])


def parse_name_entry_spans(upk, package) -> List[Tuple[int, int, int, int]]:
    data = package.file_bytes
    pos = package.summary.name_offset
    spans: List[Tuple[int, int, int, int]] = []
    for _ in range(package.summary.name_count):
        start = pos
        if pos + 4 > len(data):
            raise ValueError("Name table is truncated")
        length = struct.unpack_from("<i", data, pos)[0]
        pos += 4
        if length > 0:
            byte_count = length
            pos += byte_count
        elif length < 0:
            byte_count = -length * 2
            pos += byte_count
        else:
            byte_count = 0
        flags_offset = pos
        pos += 8
        spans.append((start, flags_offset + 8, length, flags_offset))
    return spans


def make_fixed_fstring(old_len: int, new_text: str) -> Optional[bytes]:
    if old_len > 0:
        try:
            raw = new_text.encode("ascii")
        except UnicodeEncodeError:
            return None
        if len(raw) + 1 > old_len:
            return None
        return struct.pack("<i", old_len) + raw + b"\x00" + (b"\x00" * (old_len - len(raw) - 1))
    if old_len < 0:
        char_count = -old_len
        raw = new_text.encode("utf-16-le")
        if len(new_text) + 1 > char_count:
            return None
        pad_chars = char_count - len(new_text) - 1
        return struct.pack("<i", old_len) + raw + b"\x00\x00" + (b"\x00\x00" * pad_chars)
    return None


def fixed_rename_name_entry(upk, package, name_index: int, new_text: str):
    spans = parse_name_entry_spans(upk, package)
    start, end, old_len, flags_offset = spans[name_index]
    payload = make_fixed_fstring(old_len, new_text)
    if payload is None:
        return None, 0
    flags = package.file_bytes[flags_offset:flags_offset + 8]
    replacement = payload + flags
    if len(replacement) != end - start:
        raise ValueError("Fixed name replacement length mismatch")
    data = bytearray(package.file_bytes)
    data[start:end] = replacement
    result = upk.parse_decrypted_package_bytes(package.file_path, bytes(data))
    old_display = clean_name(package.names[name_index].name)
    pad = max(0, abs(old_len) - len(new_text) - 1)
    setattr(result, "_fixed_rename_index", name_index)
    setattr(result, "_fixed_rename_old", old_display)
    setattr(result, "_fixed_rename_new", new_text)
    setattr(result, "_fixed_rename_pad", pad)
    return result, pad


def patch_header_object_name_refs(upk, package, old_name: str, new_name: str) -> Tuple[object, List[str]]:
    old_indices, _ = find_name_indices(package, old_name)
    new_indices, _ = find_name_indices(package, new_name)
    if not old_indices or not new_indices:
        return package, []
    old_set = set(old_indices)
    new_idx = new_indices[0]
    data = bytearray(package.file_bytes)
    log: List[str] = []

    if hasattr(upk, "get_export_entry_offsets"):
        offsets = upk.get_export_entry_offsets(package)
        for exp, off in zip(package.exports, offsets):
            if exp.object_name.name_index in old_set:
                data[off + 12:off + 16] = struct.pack("<i", new_idx)
                log.append(f"PATCHED: export[{exp.table_index}] object_name {old_name!r} -> existing {new_name!r}")

    import_off = package.summary.import_offset
    for imp in package.imports:
        off = import_off + imp.table_index * 28
        if imp.object_name.name_index in old_set:
            data[off + 20:off + 24] = struct.pack("<i", new_idx)
            log.append(f"PATCHED: import[{imp.table_index}] object_name {old_name!r} -> existing {new_name!r}")

    if not log:
        return package, []
    return upk.parse_decrypted_package_bytes(package.file_path, bytes(data)), log


def apply_name_pairs(upk, package, pairs: Sequence[Tuple[str, str]], preserve_header_offsets: bool) -> Tuple[object, List[str]]:
    current = package
    log: List[str] = []
    for old, new in pairs:
        indices, case_match = find_name_indices(current, old)
        if not indices:
            log.append(f"MISS: no name-table entry matching {old!r}")
            continue
        if case_match:
            log.append(f"CASE: matched {old!r} case-insensitively")

        # FIX: Instead of only patching header refs (which causes a Header/Body desync crash),
        # we free up the target name if it already exists in the file's dictionary.
        colliding_indices, _ = find_name_indices(current, new)
        for c_idx in colliding_indices:
            dummy_name = f"FREEDNAME{c_idx}" # No underscores, engine treats as pure base name
            if preserve_header_offsets:
                fixed, pad = fixed_rename_name_entry(upk, current, c_idx, dummy_name)
                if fixed is not None:
                    current = fixed
                    log.append(f"FREED(FIXED): name[{c_idx}] freed to {dummy_name} in-place; pad={pad}.")
                    continue
            try:
                current = upk.rename_name_entry(current, c_idx, dummy_name)
                log.append(f"FREED: Renamed colliding name at index {c_idx} to {dummy_name}")
            except Exception as e:
                log.append(f"WARN: Could not free colliding name: {e}")

        # Now force the physical text replacement so body and header stay perfectly synced
        for idx in indices:
            old_actual = clean_name(current.names[idx].name)
            if preserve_header_offsets:
                fixed, pad = fixed_rename_name_entry(upk, current, idx, new)
                if fixed is not None:
                    current = fixed
                    log.append(f"FIXED: name[{idx}] {old_actual!r} -> {new!r} in-place; preserved header offsets; pad={pad}.")
                    continue
            try:
                current = upk.rename_name_entry(current, idx, new)
                log.append(f"RENAMED: name[{idx}] {old_actual!r} -> {new!r}; header offsets may change.")
            except Exception as e:
                log.append(f"ERROR: could not rename {old_actual!r}: {e}")
                
    return current, log


def load_provider(upk, keys_path: Optional[Path], donor_path: Path, script_path: Path):
    if keys_path and keys_path.exists():
        return upk.DecryptionProvider(str(keys_path)), keys_path
    found = upk.find_keys_path(script_path, donor_path) if hasattr(upk, "find_keys_path") else None
    if found:
        return upk.DecryptionProvider(str(found)), Path(found)
    return None, None


def resolve_with_optional_keys(upk, input_path: Path, temp_dir: Path, keys_path: Optional[Path]):
    if not keys_path:
        return upk.resolve_input_package(input_path, temp_dir, script_dir())
    old_find = getattr(upk, "find_keys_path", None)
    if old_find is None:
        return upk.resolve_input_package(input_path, temp_dir, script_dir())
    def forced(_script_dir, _selected_file):
        return keys_path
    upk.find_keys_path = forced
    try:
        return upk.resolve_input_package(input_path, temp_dir, script_dir())
    finally:
        upk.find_keys_path = old_find


def summary_line(package) -> str:
    return f"names={package.summary.name_count}, depends={package.summary.depends_offset}, first_export={package.exports[0].serial_offset if package.exports else 0}"



def build_reencrypted_package_with_output_key(upk, original_encrypted_path: Path, modified_decrypted_bytes: bytes, provider, output_path: Path, output_key: bytes) -> Path:
    summary, meta, original_encrypted_data, donor_key = upk.find_valid_key(original_encrypted_path, provider)
    modified_summary = upk.parse_file_summary(io.BytesIO(modified_decrypted_bytes))
    original_plain = bytearray(upk.DecryptionProvider.decrypt_ecb(donor_key, original_encrypted_data))
    original_chunks = upk.parse_rl_compressed_chunks(bytes(original_plain), meta.compressed_chunks_offset)
    if not original_chunks:
        raise ValueError("No compressed chunks were found in original encrypted header")

    new_chunk_table_offset = modified_summary.depends_offset - modified_summary.name_offset
    patch_limit = max(0, new_chunk_table_offset)
    chunk_shift = modified_summary.depends_offset - original_chunks[0].uncompressed_offset

    rebuilt_chunks = []
    rebuilt_chunk_payloads = []
    chunk_table_placeholder = upk.serialize_rl_chunk_table([
        upk.FCompressedChunk(0, 0, 0, 0) for _ in original_chunks
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
        payload = upk.compress_chunk_payload(modified_decrypted_bytes[start:end])
        rebuilt_chunk_payloads.append(payload)
        rebuilt_chunks.append(upk.FCompressedChunk(
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

    chunk_table = upk.serialize_rl_chunk_table(rebuilt_chunks)
    table_end = new_chunk_table_offset + len(chunk_table)
    if table_end > len(header_plain):
        raise ValueError("Rebuilt compressed chunk table does not fit inside encrypted header")
    header_plain[new_chunk_table_offset:table_end] = chunk_table
    encrypted_header = upk.DecryptionProvider.encrypt_ecb(output_key, bytes(header_plain))

    original_bytes = Path(original_encrypted_path).read_bytes()
    prefix = bytearray(original_bytes[:summary.name_offset])
    summary_offsets = upk._find_summary_offsets(modified_decrypted_bytes)
    upk.patch_i32_le(prefix, summary_offsets["total_header_size_offset"], new_total_header_size)
    upk.patch_i32_le(prefix, summary_offsets["name_count_offset"], modified_summary.name_count)
    upk.patch_i32_le(prefix, summary_offsets["name_offset_offset"], modified_summary.name_offset)
    upk.patch_i32_le(prefix, summary_offsets["export_count_offset"], modified_summary.export_count)
    upk.patch_i32_le(prefix, summary_offsets["export_offset_offset"], modified_summary.export_offset)
    upk.patch_i32_le(prefix, summary_offsets["import_count_offset"], modified_summary.import_count)
    upk.patch_i32_le(prefix, summary_offsets["import_offset_offset"], modified_summary.import_offset)
    upk.patch_i32_le(prefix, summary_offsets["depends_offset_offset"], modified_summary.depends_offset)
    upk.patch_i32_le(prefix, summary_offsets["import_export_guids_offset_offset"], modified_summary.import_export_guids_offset)
    if "thumbnail_table_offset_offset" in summary_offsets:
        upk.patch_i32_le(prefix, summary_offsets["thumbnail_table_offset_offset"], modified_summary.thumbnail_table_offset)
    upk._patch_generation_counts(prefix, summary_offsets, modified_summary.export_count, modified_summary.name_count)
    with original_encrypted_path.open("rb") as src:
        meta_offsets = upk._find_file_compression_metadata_offsets(src)
    upk.patch_i32_le(prefix, meta_offsets["compressed_chunks_offset_offset"], new_chunk_table_offset)
    if rebuilt_chunks:
        upk.patch_i32_le(prefix, meta_offsets["last_block_size_offset"], rebuilt_chunks[-1].uncompressed_size)

    print(f"[DEBUG] Re-encrypting: name_off={modified_summary.name_offset}, dep_off={modified_summary.depends_offset}, total_header={new_total_header_size}")
    if modified_summary.name_offset != summary.name_offset:
        print(f"[DEBUG] WARNING: name_offset SHIFTED from {summary.name_offset} to {modified_summary.name_offset}")

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

def build_output(upk, donor_path: Path, target_key_path: Path, modified, provider, output_path: Path, was_encrypted: bool, log: List[str]) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    if was_encrypted and provider is not None:
        override_key = None
        if target_key_path.exists() and hasattr(upk, "find_key_for_encrypted_upk"):
            override_key = upk.find_key_for_encrypted_upk(target_key_path, provider)
            log.append(f"Output key source:   {target_key_path}")
            log.append(f"Encrypting with key from target/original {target_key_path.name}: {base64.b64encode(override_key).decode()}")
        elif target_key_path.exists():
            log.append(f"Output key source exists but rl_upk_editor has no find_key_for_encrypted_upk: {target_key_path}")
        else:
            log.append(f"WARN: target key source missing, falling back to donor key: {target_key_path}")
        build_reencrypted_package_with_output_key(upk, donor_path, modified.file_bytes, provider, output_path, override_key) if override_key is not None else upk.build_reencrypted_package(donor_path, modified.file_bytes, provider, output_path)
        if override_key is not None:
            try:
                check_provider = upk.DecryptionProvider(None)
                check_provider.decryption_keys = [override_key]
                upk.find_valid_key(output_path, check_provider)
                log.append("Verified output decrypts with the target/original package key.")
            except Exception as exc:
                log.append(f"WARN: output key verification failed: {exc}")
        log.append("Saved encrypted/compressed output.")
    else:
        output_path.write_bytes(modified.file_bytes)
        log.append("Saved decrypted/decompressed output because input was not encrypted.")


def swap_one_package(upk, source_path: Path, output_path: Path, key_source_path: Path, pairs: Sequence[Tuple[str, str]], options: SwapOptions) -> Tuple[Path, List[str]]:
    log: List[str] = []
    cleanup_old_temp_files(options.output_dir, options.logger)
    if not source_path.exists():
        raise FileNotFoundError(f"Source package not found: {source_path}")
    if output_path.exists() and not options.overwrite:
        raise FileExistsError(f"Output already exists: {output_path}")

    temp_dir = script_dir() / "AssetSwapper_Decrypted"
    temp_dir.mkdir(exist_ok=True)

    log.append(f"Input source:        {source_path}")
    log.append(f"Output target:       {output_path}")
    log.append(f"Key source target:   {key_source_path}")

    resolved_path, package, provider, actual_keys_path, was_encrypted = resolve_with_optional_keys(upk, source_path, temp_dir, options.keys_path)
    log.append(f"Resolved package:    {resolved_path}")
    if actual_keys_path:
        log.append(f"Keys file:           {actual_keys_path}")
    log.append(f"Original offsets:    {summary_line(package)}")

    log.append("Name-table changes:")
    for old, new in pairs:
        log.append(f"  {old!r} -> {new!r}")

    modified, rename_log = apply_name_pairs(upk, package, pairs, options.preserve_header_offsets)
    log.extend(rename_log)
    log.append(f"Modified offsets:    {summary_line(modified)}")

    if output_path.exists() and options.overwrite:
        backup_path = output_path.with_suffix(output_path.suffix + ".bak")
        shutil.copy2(output_path, backup_path)
        log.append(f"Backup written:      {backup_path}")

    build_output(upk, source_path, key_source_path, modified, provider, output_path, was_encrypted, log)
    return output_path, log


def swap_asset(upk, target: Item, donor: Item, options: SwapOptions) -> Tuple[List[Path], List[str]]:
    if target.slot != donor.slot:
        raise ValueError(f"Slot mismatch: target={target.slot!r}, donor={donor.slot!r}")
    key_dir = options.key_source_dir or options.donor_dir
    all_paths: List[Path] = []
    all_log: List[str] = []
    all_log.append(f"Target/replaced item: {target.label}")
    all_log.append(f"Donor/visual item:    {donor.label}")
    main_path, main_log = swap_one_package(
        upk,
        options.donor_dir / donor.asset_package,
        options.output_dir / target.asset_package,
        key_dir / target.asset_package,
        infer_name_pairs(target, donor),
        options,
    )
    all_paths.append(main_path)
    all_log.extend(main_log)

    if options.include_thumbnails:
        donor_thumb = options.donor_dir / donor.thumbnail_package
        target_thumb = options.output_dir / target.thumbnail_package
        key_thumb = key_dir / target.thumbnail_package
        if donor_thumb.exists() and key_thumb.exists():
            all_log.append("")
            all_log.append("Thumbnail/_T_SF pass:")
            thumb_path, thumb_log = swap_one_package(upk, donor_thumb, target_thumb, key_thumb, infer_thumbnail_pairs(target, donor), options)
            all_paths.append(thumb_path)
            all_log.extend(thumb_log)
        else:
            all_log.append(f"SKIP thumbnails: missing {donor_thumb if not donor_thumb.exists() else key_thumb}")
    else:
        all_log.append("SKIP thumbnails: disabled.")

    return all_paths, all_log


def cleanup_old_temp_files(directory: Path, logger: Optional[Callable[[str], None]] = None) -> None:
    import time
    if not directory.exists():
        return
    now = time.time()
    cutoff = 24 * 3600
    for file in directory.glob("*"):
        if file.name.endswith(("_decrypted.upk", "_decompressed.upk")):
            try:
                mtime = file.stat().st_mtime
                if now - mtime > cutoff:
                    file.unlink()
                    if logger:
                        logger(f"CLEANUP: Removed old temp file {file.name}")
            except Exception:
                pass

def revert_item(target: Item, options: SwapOptions) -> Tuple[List[Path], List[str]]:
    src_dir = options.key_source_dir or options.donor_dir
    paths: List[Path] = []
    log: List[str] = []
    pairs = [(src_dir / target.asset_package, options.output_dir / target.asset_package)]
    if options.include_thumbnails:
        pairs.append((src_dir / target.thumbnail_package, options.output_dir / target.thumbnail_package))
    for src, dst in pairs:
        if not src.exists():
            log.append(f"MISS: revert source not found: {src}")
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        if dst.exists() and options.overwrite:
            backup_path = dst.with_suffix(dst.suffix + ".bak")
            shutil.copy2(dst, backup_path)
            log.append(f"Backup written: {backup_path}")
        shutil.copy2(src, dst)
        paths.append(dst)
        log.append(f"Reverted: {src} -> {dst}")
    return paths, log




def build_arg_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser()
    p.add_argument("--items", type=Path, default=default_path(("items.json", "items(4).json")))
    p.add_argument("--keys", type=Path, default=None)
    p.add_argument("--donor-dir", "--upk-dir", "--input-dir", dest="donor_dir", type=Path, default=None)
    p.add_argument("--output-dir", "--out-dir", dest="output_dir", type=Path, default=None)
    p.add_argument("--key-source-dir", type=Path, default=None)
    p.add_argument("--slot", default="")
    p.add_argument("--target", default="")
    p.add_argument("--donor", default="")
    p.add_argument("--auto-swap", action="store_true")
    p.add_argument("--no-gui", action="store_true")
    p.add_argument("--revert", action="store_true")
    p.add_argument("--fetch", action="store_true")
    p.add_argument("--token", default="")
    p.add_argument("--account", default="Unknown")
    thumbs = p.add_mutually_exclusive_group()
    thumbs.add_argument("--include-thumbnails", dest="include_thumbnails", action="store_true", default=False)
    thumbs.add_argument("--no-thumbnails", dest="include_thumbnails", action="store_false")
    preserve = p.add_mutually_exclusive_group()
    preserve.add_argument("--preserve-header-offsets", dest="preserve_header_offsets", action="store_true", default=True)
    preserve.add_argument("--no-preserve-header-offsets", dest="preserve_header_offsets", action="store_false")
    overwrite = p.add_mutually_exclusive_group()
    overwrite.add_argument("--overwrite", dest="overwrite", action="store_true", default=True)
    overwrite.add_argument("--no-overwrite", dest="overwrite", action="store_false")
    return p


def interactive_run(args: argparse.Namespace) -> int:
    print("\n=== VelocityRL Interactive CLI ===")
    
    if not args.donor_dir:
        val = input("Path to CookedPCConsole: ").strip().strip('"')
        if not val: raise SystemExit("Aborted")
        args.donor_dir = Path(val)
        
    if not args.output_dir:
        val = input("Path to Output Folder (Press Enter to use CookedPCConsole): ").strip().strip('"')
        args.output_dir = Path(val) if val else args.donor_dir

    items = load_items(args.items)
    
    if not args.slot:
        slots = sorted({i.slot for i in items if i.slot})
        print("\nAvailable Slots:")
        for idx, s in enumerate(slots):
            print(f"  {idx+1}. {s}")
        idx_str = input(f"Select slot (1-{len(slots)}): ").strip()
        if not idx_str: raise SystemExit("Aborted")
        args.slot = slots[int(idx_str)-1]

    def search_item(prompt: str):
        while True:
            query = input(prompt).strip().lower()
            if not query: return None
            matches = [i for i in items if i.slot == args.slot and (query in i.product.lower() or query == str(i.id))]
            if not matches:
                print("No matches found in this slot. Try again.")
                continue
            if len(matches) == 1:
                return matches[0]
            print("\nMultiple matches found:")
            for idx, m in enumerate(matches[:15]):
                print(f"  {idx+1}. {m.product} ({m.id})")
            if len(matches) > 15: print("  ...")
            idx_str = input(f"Select item (1-{min(len(matches), 15)}) or press Enter to refine search: ").strip()
            if not idx_str: continue
            try:
                return matches[int(idx_str)-1]
            except (ValueError, IndexError):
                continue

    if not args.target:
        target_item = search_item("\nSearch for target item (the one you own): ")
        if not target_item: raise SystemExit("Aborted")
        args.target = target_item.id

    if not args.donor and not args.revert:
        donor_item = search_item("\nSearch for donor item (the one you want): ")
        if not donor_item: raise SystemExit("Aborted")
        args.donor = donor_item.id

    # Now that we have the args, run the normal logic
    return cli_run(args)


def cli_run(args: argparse.Namespace) -> int:
    # If any required args are missing, try interactive mode if we're in a TTY
    if not args.donor_dir or not args.output_dir or (not args.revert and (not args.target or not args.donor)):
        if sys.stdin.isatty():
            return interactive_run(args)
        
    if not args.donor_dir or not args.output_dir:
        raise SystemExit("--donor-dir and --output-dir are required")
    if args.revert and not args.target:
        raise SystemExit("--target is required for --revert")
    if not args.revert and (not args.target or not args.donor):
        raise SystemExit("--target and --donor are required")
    upk = import_rl_upk_editor()
    items = load_items(args.items)
    target = find_item(items, str(args.target), args.slot)
    donor = find_item(items, str(args.donor), target.slot if not args.slot else args.slot) if args.donor else target
    keys = args.keys
    if keys is None:
        here = script_dir()
        candidates = [
            here / "keys.txt",
            here / "keys(1).txt",
            here.parent / "python" / "keys.txt",
            here.parent / "python" / "keys(1).txt",
            Path.cwd() / "keys.txt",
            Path.cwd() / "python" / "keys.txt",
            args.donor_dir / "keys.txt" if args.donor_dir else None,
        ]
        for candidate in candidates:
            if candidate is not None and candidate.exists():
                keys = candidate
                break
    options = SwapOptions(
        items_path=args.items,
        keys_path=keys,
        donor_dir=args.donor_dir,
        output_dir=args.output_dir,
        key_source_dir=args.key_source_dir,
        include_thumbnails=args.include_thumbnails,
        preserve_header_offsets=args.preserve_header_offsets,
        overwrite=args.overwrite,
    )
    if args.revert:
        _, log = revert_item(target, options)
    else:
        _, log = swap_asset(upk, target, donor, options)
    for line in log:
        print(line)
    return 0


def fetch_catalog(args: argparse.Namespace) -> int:
    if not args.token:
        print("Error: --token is required for --fetch")
        return 1
        
    REQUEST_KEY = bytes.fromhex("c338bd36fb8c42b1a431d30add939fc7")
    PSYNET_RPC_URL = "https://api.rlpp.psynet.gg/rpc/"

    def get_psysig(body: str, key: bytes) -> str:
        msg = f"-{body}".encode("utf-8")
        sig = hmac.new(key, msg, hashlib.sha256).digest()
        return base64.b64encode(sig).decode("utf-8")

    def call_rpc(service: str, body: dict, psy_token=None, session_id=None) -> dict:
        headers = {
            "PsyService": service,
            "PsyEnvironment": "Prod",
            "User-Agent": "RL Win/250811.43331.492665 gzip",
            "Content-Type": "application/json"
        }
        if psy_token: headers["PsyToken"] = psy_token
        if session_id: headers["PsySessionID"] = session_id
        
        json_body = json.dumps(body)
        headers["PsySig"] = get_psysig(json_body, REQUEST_KEY)
        
        import requests
        resp = requests.post(PSYNET_RPC_URL, headers=headers, data=json_body)
        if resp.status_code != 200:
            raise Exception(f"RPC failed: {resp.status_code} - {resp.text}")
        return resp.json()["Result"]

    try:
        print(f"Logging in for {args.account}...")
        login_body = {
            "Platform": "Epic",
            "PlayerName": args.account,
            "PlayerID": args.account,
            "Language": "INT",
            "AuthTicket": args.token,
            "FeatureSet": "PrimeUpdate55_1",
            "Device": "PC",
            "EpicAuthTicket": args.token,
            "EpicAccountID": args.account
        }
        res = call_rpc("Auth/Login v4", login_body)
        psy_token = res["PsyToken"]
        session_id = res["SessionID"]
        player_id = res["PlayerID"]
        print("Login successful. Fetching catalog...")

        catalog_body = {
            "PlayerID": player_id,
            "Category": "StarterPack" # Default category
        }
        catalog = call_rpc("Microtransaction/GetCatalog v1", catalog_body, psy_token, session_id)
        
        # In a real tool, we'd do more, but for now we output the catalog
        # The user's goal is to see it's working
        print(json.dumps(catalog, indent=4))
        
        # Also try to fetch shop
        shop_res = call_rpc("Shops/GetStandardShops v1", {}, psy_token, session_id)
        print("\n=== Available Shops ===")
        print(json.dumps(shop_res, indent=4))

        return 0
    except Exception as e:
        print(f"Fetch Error: {e}")
        return 1


def main() -> int:
    parser = build_arg_parser()
    args = parser.parse_args()
    
    if args.fetch:
        return fetch_catalog(args)

    try:
        return cli_run(args)
    except Exception as e:
        print(f"FATAL ERROR: {e}")
        traceback.print_exc()
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
