#!/usr/bin/env python3
"""
VelocityRL CLI — standalone interactive item swap tool.
Run 'velocityrl' to enter interactive mode.
Run 'velocityrl --config' to update your game directory.
"""
import json
import sys
from pathlib import Path

# ── Resource paths ────────────────────────────────────────────────────────────
# PyInstaller bundles files into sys._MEIPASS; when running from source,
# look in the same directory as this script.
RESOURCE_DIR = Path(getattr(sys, '_MEIPASS', Path(__file__).parent))
if str(RESOURCE_DIR) not in sys.path:
    sys.path.insert(0, str(RESOURCE_DIR))

import rl_asset_swapper as engine  # noqa: E402

# ── Config ────────────────────────────────────────────────────────────────────
CONFIG_FILE = Path.home() / '.velocityrl.json'


def load_config() -> dict:
    try:
        return json.loads(CONFIG_FILE.read_text())
    except Exception:
        return {}


def save_config(cfg: dict):
    CONFIG_FILE.write_text(json.dumps(cfg, indent=2))


# ── Windows game-dir auto-detection ──────────────────────────────────────────
_CANDIDATE_DIRS = [
    'C:/Program Files (x86)/Steam/steamapps/common/rocketleague/TAGame/CookedPCConsole',
    'C:/Program Files/Steam/steamapps/common/rocketleague/TAGame/CookedPCConsole',
    'D:/SteamLibrary/steamapps/common/rocketleague/TAGame/CookedPCConsole',
    'E:/SteamLibrary/steamapps/common/rocketleague/TAGame/CookedPCConsole',
    'D:/Games/rocketleague/TAGame/CookedPCConsole',
    'C:/Games/rocketleague/TAGame/CookedPCConsole',
]


def detect_game_dir():
    for p in _CANDIDATE_DIRS:
        pp = Path(p)
        if pp.exists():
            return pp
    if sys.platform == 'win32':
        try:
            import winreg
            for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
                for sub in (
                    r'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 252950',
                    r'SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 252950',
                ):
                    try:
                        with winreg.OpenKey(hive, sub) as k:
                            loc = winreg.QueryValueEx(k, 'InstallLocation')[0]
                            pp = Path(loc) / 'TAGame' / 'CookedPCConsole'
                            if pp.exists():
                                return pp
                    except (FileNotFoundError, OSError):
                        pass
        except ImportError:
            pass
    return None


# ── Main ──────────────────────────────────────────────────────────────────────
def main():
    cfg = load_config()

    if '--config' in sys.argv:
        cur = cfg.get('game_dir', 'not set')
        print(f'Current game directory: {cur}')
        path = input('New CookedPCConsole path (Enter to keep): ').strip().strip('"')
        if path:
            cfg['game_dir'] = path
            save_config(cfg)
            print(f'Saved to {CONFIG_FILE}')
        sys.exit(0)

    # First-run: detect or ask for game directory
    if 'game_dir' not in cfg:
        print('VelocityRL — first run setup')
        print('─' * 50)
        found = detect_game_dir()
        if found:
            print(f'Found Rocket League: {found}')
            answer = input('Use this directory? [Y/n]: ').strip().lower()
            if answer in ('', 'y', 'yes'):
                cfg['game_dir'] = str(found)
                save_config(cfg)
        if 'game_dir' not in cfg:
            path = input('Enter CookedPCConsole path: ').strip().strip('"')
            if not path:
                sys.exit('Aborted.')
            cfg['game_dir'] = path
            save_config(cfg)
        print()

    game_dir = Path(cfg['game_dir'])
    parser = engine.build_arg_parser()
    args = parser.parse_args()
    if not args.donor_dir:
        args.donor_dir = game_dir
    if not args.output_dir:
        args.output_dir = game_dir
    sys.exit(engine.cli_run(args))


if __name__ == '__main__':
    main()
