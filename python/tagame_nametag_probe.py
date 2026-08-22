#!/usr/bin/env python3
"""Probe Rocket League TAGame.upk for player-title / nametag data and diff edits."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from rl_upk_editor import (  # noqa: E402
    ExportEntry,
    ParsedPackage,
    parse_serialized_properties,
    resolve_input_package,
)

PLAYER_TITLE_SLOT_EXPORT = 92758
DEFAULT_WORK_DIR = SCRIPT_DIR / "_probe_out"
HEX_PREVIEW_BYTES = 128


@dataclass
class NametagRecord:
    export_index: int
    name: str
    class_name: str
    asset_path: str
    asset_package: str
    quality: str
    slot: str
    trade_restriction_ids: List[int]
    file_offset: int
    serial_size: int
    serial_sha256: str
    properties: Dict[str, str]


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def hex_preview(data: bytes, limit: int = HEX_PREVIEW_BYTES) -> str:
    clipped = data[:limit]
    text = clipped.hex(" ").upper()
    if len(data) > limit:
        text += f" ... (+{len(data) - limit} bytes)"
    return text


def parse_trade_restriction_ids(value: Any) -> List[int]:
    text = str(value)
    match = re.search(r"data=([0-9A-Fa-f ]+)", text)
    if not match:
        return []
    raw = bytes.fromhex(match.group(1).replace(" ", ""))
    ids: List[int] = []
    for offset in range(0, len(raw), 8):
        chunk = raw[offset : offset + 8]
        if len(chunk) == 8:
            ids.append(int.from_bytes(chunk, "little"))
    return ids


def prop_map(props) -> Dict[str, str]:
    return {prop.name: str(prop.value) for prop in props}


def is_player_title_product(slot_value: Any) -> bool:
    text = str(slot_value)
    return "PlayerTitle" in text


def load_items_player_titles() -> List[Dict[str, Any]]:
    items_path = SCRIPT_DIR / "items.json"
    if not items_path.exists():
        return []
    with items_path.open("r", encoding="utf-8") as fh:
        payload = json.load(fh)
    return [item for item in payload.get("Items", []) if item.get("Slot") == "Player Title"]


def load_package(upk_path: Path, work_dir: Path) -> Tuple[ParsedPackage, Path, bool]:
    work_dir.mkdir(parents=True, exist_ok=True)
    _decrypted_path, package, _provider, _keys_path, was_encrypted = resolve_input_package(
        upk_path, work_dir, SCRIPT_DIR
    )
    return package, str(upk_path.resolve()), was_encrypted


def collect_player_title_products(package: ParsedPackage) -> List[NametagRecord]:
    records: List[NametagRecord] = []
    for export in package.exports:
        if package.export_class_name(export) != "Product_TA":
            continue
        props = parse_serialized_properties(package, export, None)
        slot = next((prop.value for prop in props if prop.name == "Slot"), None)
        if not is_player_title_product(slot):
            continue
        raw = package.object_data(export) or b""
        records.append(
            NametagRecord(
                export_index=export.table_index,
                name=package.resolve_name(export.object_name),
                class_name=package.export_class_name(export),
                asset_path=str(next((prop.value for prop in props if prop.name == "AssetPath"), "")),
                asset_package=str(next((prop.value for prop in props if prop.name == "AssetPackageName"), "")),
                quality=str(next((prop.value for prop in props if prop.name == "Quality"), "")),
                slot=str(slot),
                trade_restriction_ids=parse_trade_restriction_ids(
                    next((prop.value for prop in props if prop.name == "TradeRestrictions"), "")
                ),
                file_offset=export.serial_offset,
                serial_size=export.serial_size,
                serial_sha256=sha256_bytes(raw),
                properties=prop_map(props),
            )
        )
    records.sort(key=lambda row: row.export_index)
    return records


def collect_title_name_table(package: ParsedPackage) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for index, entry in enumerate(package.names):
        if not re.search(r"title|nametag", entry.name, re.IGNORECASE):
            continue
        rows.append({"name_index": index, "name": entry.name, "flags": entry.flags})
    return rows


def collect_title_related_exports(package: ParsedPackage) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    patterns = (
        "PlayerTitle",
        "GFxData_PlayerTitles",
        "TitleIDs",
        "title_generic",
        "ProductAsset_PlayerTitle",
    )
    for export in package.exports:
        name = package.resolve_name(export.object_name)
        class_name = package.export_class_name(export)
        if not any(token in name or token in class_name for token in patterns):
            continue
        raw = package.object_data(export) or b""
        rows.append(
            {
                "export_index": export.table_index,
                "name": name,
                "class_name": class_name,
                "serial_offset": export.serial_offset,
                "serial_size": export.serial_size,
                "serial_sha256": sha256_bytes(raw),
            }
        )
    rows.sort(key=lambda row: row["export_index"])
    return rows


def build_export_manifest(package: ParsedPackage) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for export in package.exports:
        raw = package.object_data(export) or b""
        rows.append(
            {
                "export_index": export.table_index,
                "name": package.resolve_name(export.object_name),
                "class_name": package.export_class_name(export),
                "serial_offset": export.serial_offset,
                "serial_size": export.serial_size,
                "serial_sha256": sha256_bytes(raw),
            }
        )
    return rows


def build_name_manifest(package: ParsedPackage) -> List[Dict[str, Any]]:
    return [{"name_index": index, "name": entry.name} for index, entry in enumerate(package.names)]


def dump_tagame(upk_path: Path, work_dir: Path) -> Dict[str, Any]:
    package, source_path, was_encrypted = load_package(upk_path, work_dir)
    player_titles = collect_player_title_products(package)
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_upk": str(source_path.resolve()),
        "source_sha256": sha256_file(source_path),
        "was_encrypted": was_encrypted,
        "export_count": len(package.exports),
        "name_count": len(package.names),
        "player_title_slot_export": PLAYER_TITLE_SLOT_EXPORT,
        "player_title_products": [asdict(row) for row in player_titles],
        "items_json_player_titles": load_items_player_titles(),
        "title_name_table": collect_title_name_table(package),
        "title_related_exports": collect_title_related_exports(package),
        "notes": [
            "PlayerTitle Product_TA rows are the embedded nametag product definitions.",
            "Most RL titles are synced online; TAGame often only ships title_generic locally.",
            "TradeRestrictions u64 values are the in-package restriction IDs for a product.",
        ],
    }


def snapshot_tagame(upk_path: Path, work_dir: Path) -> Dict[str, Any]:
    package, source_path, was_encrypted = load_package(upk_path, work_dir)
    player_titles = collect_player_title_products(package)
    return {
        "kind": "tagame_snapshot",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_upk": str(source_path.resolve()),
        "source_sha256": sha256_file(source_path),
        "was_encrypted": was_encrypted,
        "export_count": len(package.exports),
        "name_count": len(package.names),
        "player_title_products": [asdict(row) for row in player_titles],
        "title_name_table": build_name_manifest(package),
        "exports": build_export_manifest(package),
    }


def _changed_rows(before_rows: Sequence[Dict[str, Any]], after_rows: Sequence[Dict[str, Any]], key: str):
    before_map = {row[key]: row for row in before_rows}
    after_map = {row[key]: row for row in after_rows}
    changed = []
    for index, before_row in before_map.items():
        after_row = after_map.get(index)
        if after_row is None:
            changed.append({"kind": "removed", key: index, "before": before_row})
            continue
        if before_row.get("serial_sha256") != after_row.get("serial_sha256"):
            changed.append({"kind": "modified", key: index, "before": before_row, "after": after_row})
    for index, after_row in after_map.items():
        if index not in before_map:
            changed.append({"kind": "added", key: index, "after": after_row})
    return changed


def diff_tagame(before_path: Path, after_path: Path, work_dir: Path, before_snapshot: Optional[Path] = None) -> Dict[str, Any]:
    before_payload: Optional[Dict[str, Any]] = None
    if before_snapshot is not None:
        with before_snapshot.open("r", encoding="utf-8") as fh:
            before_payload = json.load(fh)
        before_source = before_payload.get("source_upk", str(before_path))
        before_sha = before_payload.get("source_sha256")
        before_path = Path(before_source)
        before_exports = before_payload.get("exports", [])
        before_names = before_payload.get("title_name_table", before_payload.get("names", []))
        before_player_titles = before_payload.get("player_title_products", [])
    else:
        before_source = str(before_path.resolve())
        before_sha = sha256_file(before_path)

    before_package, _, _ = load_package(before_path, work_dir)
    after_package, after_source, _after_encrypted = load_package(after_path, work_dir)

    if before_payload is None:
        before_exports = build_export_manifest(before_package)
        before_names = build_name_manifest(before_package)
        before_player_titles = [asdict(row) for row in collect_player_title_products(before_package)]

    after_exports = build_export_manifest(after_package)
    after_names = build_name_manifest(after_package)
    after_player_titles = [asdict(row) for row in collect_player_title_products(after_package)]

    changed_exports = _changed_rows(before_exports, after_exports, "export_index")
    changed_names = _changed_rows(before_names, after_names, "name_index")
    changed_player_titles = _changed_rows(before_player_titles, after_player_titles, "export_index")

    detailed_changes: List[Dict[str, Any]] = []
    for row in changed_exports:
        if row["kind"] != "modified":
            detailed_changes.append(row)
            continue
        export_index = row["export_index"]
        before_hex = hex_preview(before_package.object_data(before_package.exports[export_index]) or b"")
        after_hex = hex_preview(after_package.object_data(after_package.exports[export_index]) or b"")
        enriched = dict(row)
        enriched["before_serial_hex"] = before_hex
        enriched["after_serial_hex"] = after_hex
        detailed_changes.append(enriched)

    title_related = [
        row for row in detailed_changes if row.get("kind") == "modified" and _is_title_related_change(row)
    ]

    return {
        "kind": "tagame_diff",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "before_upk": before_source,
        "after_upk": after_source,
        "before_sha256": before_sha,
        "after_sha256": sha256_file(after_path),
        "file_hash_changed": before_sha != sha256_file(after_path),
        "snapshot_used": str(before_snapshot.resolve()) if before_snapshot is not None else None,
        "changed_export_count": len(changed_exports),
        "changed_name_count": len(changed_names),
        "changed_player_title_count": len(changed_player_titles),
        "changed_player_titles": changed_player_titles,
        "changed_title_related_exports": title_related,
        "changed_exports": detailed_changes[:200],
        "changed_names": changed_names[:200],
        "truncated": {
            "exports": len(changed_exports) > 200,
            "names": len(changed_names) > 200,
        },
    }


def _load_for_diff(upk_path: Path, work_dir: Path) -> Tuple[ParsedPackage, str, bool]:
    package, source_path, was_encrypted = load_package(upk_path, work_dir)
    return package, str(source_path.resolve()), was_encrypted


def _is_title_related_change(row: Dict[str, Any]) -> bool:
    before = row.get("before") or {}
    after = row.get("after") or {}
    haystack = " ".join(
        str(part)
        for part in (
            before.get("name"),
            after.get("name"),
            before.get("class_name"),
            after.get("class_name"),
        )
    )
    return bool(re.search(r"title|nametag|PlayerTitle|title_generic", haystack, re.IGNORECASE))


def write_json(payload: Dict[str, Any], out_path: Path) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", encoding="utf-8") as fh:
        json.dump(payload, fh, indent=2)
        fh.write("\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    dump_cmd = sub.add_parser("dump", help="Dump nametag / player-title probe data")
    dump_cmd.add_argument("upk", type=Path)
    dump_cmd.add_argument("-o", "--output", type=Path, default=DEFAULT_WORK_DIR / "nametags_dump.json")
    dump_cmd.add_argument("--work-dir", type=Path, default=DEFAULT_WORK_DIR)

    snap_cmd = sub.add_parser("snapshot", help="Save a full export/name snapshot for later diffing")
    snap_cmd.add_argument("upk", type=Path)
    snap_cmd.add_argument("-o", "--output", type=Path, default=DEFAULT_WORK_DIR / "tagame_snapshot.json")
    snap_cmd.add_argument("--work-dir", type=Path, default=DEFAULT_WORK_DIR)

    diff_cmd = sub.add_parser("diff", help="Diff two TAGame.upk files or a snapshot vs edited file")
    diff_cmd.add_argument("after", type=Path, help="Edited TAGame.upk")
    diff_cmd.add_argument("before", nargs="?", type=Path, help="Original TAGame.upk")
    diff_cmd.add_argument("--snapshot", type=Path, help="Use a snapshot JSON instead of a before UPK")
    diff_cmd.add_argument("-o", "--output", type=Path, default=DEFAULT_WORK_DIR / "tagame_diff.json")
    diff_cmd.add_argument("--work-dir", type=Path, default=DEFAULT_WORK_DIR)
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        if args.command == "dump":
            payload = dump_tagame(args.upk, args.work_dir)
        elif args.command == "snapshot":
            payload = snapshot_tagame(args.upk, args.work_dir)
        elif args.command == "diff":
            if args.snapshot is None and args.before is None:
                parser.error("diff requires either BEFORE upk or --snapshot")
            payload = diff_tagame(args.before or Path(""), args.after, args.work_dir, args.snapshot)
        else:
            parser.error(f"unknown command: {args.command}")
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    write_json(payload, args.output)
    print(json.dumps({"ok": True, "output": str(args.output.resolve()), "summary": _summarize(payload)}, indent=2))
    return 0


def _summarize(payload: Dict[str, Any]) -> Dict[str, Any]:
    if payload.get("kind") == "tagame_diff":
        return {
            "changed_exports": payload.get("changed_export_count"),
            "changed_names": payload.get("changed_name_count"),
            "changed_player_titles": payload.get("changed_player_title_count"),
        }
    if payload.get("kind") == "tagame_snapshot":
        return {
            "exports": payload.get("export_count"),
            "player_title_products": len(payload.get("player_title_products", [])),
        }
    return {
        "player_title_products": len(payload.get("player_title_products", [])),
        "title_name_table": len(payload.get("title_name_table", [])),
        "title_related_exports": len(payload.get("title_related_exports", [])),
    }


if __name__ == "__main__":
    raise SystemExit(main())
