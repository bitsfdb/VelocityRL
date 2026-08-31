#!/usr/bin/env python3
"""Annotate items.json with Paintable based on painted UPK files on disk.

VelocityRL paint swaps need a dedicated painted package (e.g. Body_Octane_Black_SF.upk)
or matching name-table variants inside the base UPK. This tool only checks for
dedicated painted packages (fast, no decryption).

Usage:
  python tools/annotate_paintable.py "E:\\games\\rocketleague\\TAGame\\CookedPCConsole"
  python tools/annotate_paintable.py "E:\\games\\rocketleague\\TAGame\\CookedPCConsole" --in-place

Writes annotated items.json to Downloads by default.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

PAINT_NAMES = [
    "None",
    "Crimson",
    "Lime",
    "Black",
    "Orange",
    "Sky Blue",
    "Cobalt",
    "Saffron",
    "Grey",
    "Pink",
    "Forest Green",
    "Purple",
    "Titanium White",
]


def file_stem(path: str) -> str:
    return Path(path).stem


def package_base(stem: str) -> str:
    for suffix in ("_sf", "_SF", "_Sf"):
        if stem.endswith(suffix):
            return stem[: -len(suffix)]
    return stem


def paint_slugs(paint_id: int) -> list[str]:
    if paint_id < 1 or paint_id > 12:
        return []
    name = PAINT_NAMES[paint_id]
    nospace = name.replace(" ", "")
    underscored = name.replace(" ", "_")
    slugs = [
        nospace,
        underscored,
        name,
        f"P{paint_id}",
        f"P{paint_id:02d}",
        str(paint_id),
    ]
    if paint_id == 5:
        slugs.extend(["SB", "Sky_Blue"])
    elif paint_id == 8:
        slugs.append("Gray")
    elif paint_id == 10:
        slugs.extend(["FG", "Forest_Green"])
    elif paint_id == 12:
        slugs.extend(["TW", "Titanium_White"])
    return slugs


def painted_package_candidates(asset_package: str, paint_id: int) -> list[str]:
    stem = file_stem(asset_package)
    if not stem:
        return []
    base = package_base(stem)
    out: list[str] = []
    for slug in paint_slugs(paint_id):
        out.append(f"{base}_{slug}_SF.upk")
        out.append(f"{stem}_{slug}.upk")
        out.append(f"{base}_{slug}.upk")
    return out


def has_painted_upk(game_dir: Path, asset_package: str) -> bool:
    if not asset_package:
        return False
    for paint_id in range(1, 13):
        for cand in painted_package_candidates(asset_package, paint_id):
            if (game_dir / cand).is_file():
                return True
    return False


def load_items(path: Path) -> tuple[dict, list[dict]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(data, list):
        return {"Items": data}, data
    items = data.get("Items") or data.get("items")
    if not isinstance(items, list):
        raise SystemExit(f"{path} has no Items array")
    return data, items


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    default_items = repo / "python" / "items.json"
    default_out = Path(os.path.expanduser("~/Downloads/items.json"))

    ap = argparse.ArgumentParser(description="Annotate items.json with Paintable flags")
    ap.add_argument("game_dir", help="Rocket League CookedPCConsole folder")
    ap.add_argument("--items", type=Path, default=default_items, help="Input items.json")
    ap.add_argument(
        "--output",
        type=Path,
        default=default_out,
        help="Output path (default: ~/Downloads/items.json)",
    )
    ap.add_argument(
        "--in-place",
        action="store_true",
        help="Overwrite --items instead of writing to Downloads",
    )
    args = ap.parse_args()

    game_dir = Path(args.game_dir)
    if not game_dir.is_dir():
        raise SystemExit(f"game_dir not found: {game_dir}")

    items_path = args.items
    if not items_path.is_file():
        raise SystemExit(f"items.json not found: {items_path}")

    root, items = load_items(items_path)
    painted = 0
    for entry in items:
        pkg = entry.get("AssetPackage") or entry.get("asset_package") or ""
        flag = has_painted_upk(game_dir, pkg)
        entry["Paintable"] = flag
        if flag:
            painted += 1

    out_path = items_path if args.in_place else args.output
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(root, indent=4, ensure_ascii=False) + "\n", encoding="utf-8")

    print(f"Wrote {out_path}")
    print(f"Paintable: {painted} / {len(items)} items (dedicated painted UPK on disk)")
    print("Items without a painted UPK file are Paintable=false — RL inventory paint still works in-game.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
