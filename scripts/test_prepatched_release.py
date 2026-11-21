"""Offline checks: publishing must never label a partial/unknown build as a release."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent


class ReleaseTests(unittest.TestCase):
    def test_vps_dry_run_requires_complete_pair_and_exact_version(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            base = [sys.executable, str(SCRIPTS / 'update_prepatched_vps.py'),
                    '--incoming', folder, '--dry-run', '--version', '261031.63733.533021']
            (root / 'TAGame.upk').write_bytes(b'a' * 100_001)
            result = subprocess.run(base, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('complete', result.stderr)
            (root / 'ProjectX.upk').write_bytes(b'b' * 100_001)
            result = subprocess.run(base, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('DRY RUN', result.stdout)
            self.assertIn('"ProjectX.upk"', result.stdout)
            result = subprocess.run(base[:-1] + ['unknown'], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)

    def test_server_does_not_inherit_old_manifest_version(self):
        spec = importlib.util.spec_from_file_location('server_updater', SCRIPTS / 'server_update_prepatched.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            incoming = root / 'incoming'
            incoming.mkdir()
            for name in ['TAGame.upk', 'ProjectX.upk']:
                (incoming / name).write_bytes(b'new')
            output = root / 'output'
            output.mkdir()
            manifest = output / 'manifest.json'
            old = json.dumps({'game_version': '261031.63733.533021', 'files': {}})
            manifest.write_text(old)
            with patch.object(module, 'PREPATCHED_DIR', output), patch.object(module, 'MANIFEST_FILE', manifest), \
                 patch.object(sys, 'argv', ['updater', '--incoming', str(incoming)]), \
                 patch.object(module.subprocess, 'run') as command:
                with self.assertRaises(SystemExit):
                    module.main()
                command.assert_not_called()
            self.assertEqual(manifest.read_text(), old)


if __name__ == '__main__':
    unittest.main()
