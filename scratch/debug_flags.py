import sys
import os

# Add RLUPKTools to path
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')

import rl_upk_editor

file_path = 'E:\\games\\rocketleague\\TAGame\\CookedPCConsole\\explosion_badaboom_SF.upk'

with open(file_path, "rb") as f:
    summary = rl_upk_editor.parse_file_summary(f)
    print(f"Package Flags: {summary.package_flags:08X}")
    print(f"Is Encrypted (0x01000000): {(summary.package_flags & 0x01000000) != 0}")
    print(f"Is Compressed (0x02000000): {(summary.package_flags & 0x02000000) != 0}")
