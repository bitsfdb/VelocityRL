#!/usr/bin/env python3
"""
Restores all UPK backup files (*.upk.bak) in the Rocket League CookedPCConsole directory.
"""

import os
import sys
import shutil

CANDIDATE_DIRS = [
    r"E:\games\rocketleague\TAGame\CookedPCConsole",
    r"C:\Program Files\Epic Games\rocketleague\TAGame\CookedPCConsole",
    r"C:\Program Files (x86)\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole",
]

def restore_backups(cooked_dir: str):
    if not os.path.isdir(cooked_dir):
        print(f"[-] Directory not found: {cooked_dir}")
        return False

    print(f"[*] Scanning for .bak files in: {cooked_dir}")
    restored_count = 0

    for filename in os.listdir(cooked_dir):
        if filename.lower().endswith(".upk.bak"):
            bak_path = os.path.join(cooked_dir, filename)
            # Remove the .bak extension to get the original UPK filename
            original_filename = filename[:-4]  # removes '.bak'
            original_path = os.path.join(cooked_dir, original_filename)

            try:
                # Copy backup over original
                shutil.copy2(bak_path, original_path)
                print(f"  [+] Restored: {original_filename} from {filename}")
                restored_count += 1
            except Exception as e:
                print(f"  [!] Failed to restore {filename}: {e}")

    if restored_count == 0:
        print("[-] No .upk.bak backup files found.")
    else:
        print(f"\n[+] Successfully restored {restored_count} file(s) to original vanilla state!")

    return True

def main():
    target_dir = sys.argv[1] if len(sys.argv) > 1 else None

    if not target_dir:
        for d in CANDIDATE_DIRS:
            if os.path.isdir(d):
                target_dir = d
                break

    if not target_dir or not os.path.isdir(target_dir):
        print("Error: Could not locate Rocket League CookedPCConsole folder.")
        print("Usage: python restore_backups.py \"E:\\games\\rocketleague\\TAGame\\CookedPCConsole\"")
        sys.exit(1)

    restore_backups(target_dir)

if __name__ == "__main__":
    main()
