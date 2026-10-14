#!/usr/bin/env python3
"""Exercise replacement, override persistence, reset and ICO frames in an isolated repo."""
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class IconPipelineTests(unittest.TestCase):
    def test_replacements_and_overrides(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            (root / 'assets').mkdir()
            shutil.copy(ROOT / 'scripts/generate-icons.py', root / 'scripts')
            shutil.copy(ROOT / 'assets/jianxin-icon.svg', root / 'assets')
            alternative = root / 'alternative.svg'
            alternative.write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" data-n="test-logo"><circle cx="50" cy="50" r="40" fill="#ff0000" data-n="test-circle"/></svg>')

            def run(*args, success=True):
                result = subprocess.run(['python3', str(root / 'scripts/generate-icons.py'), *map(str, args)], capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)

            run('--tray-source', alternative, '--web-source', alternative)
            run('--check')
            tray = (root / 'assets/jianxin-tray.png').read_bytes()
            main_source = root / 'replacement.svg'
            main_source.write_text(alternative.read_text().replace('#ff0000', '#00ff00'))
            template = (root / 'assets/jianxin-tray-template.png').read_bytes()
            android_monochrome = (root / 'android/app/src/main/res/drawable-nodpi/ic_launcher_monochrome.png').read_bytes()
            run('--windows-tray-source', main_source)
            windows_tray = (root / 'assets/jianxin-tray-windows.png').read_bytes()
            self.assertNotEqual(windows_tray, tray)
            self.assertEqual((root / 'assets/jianxin-tray-template.png').read_bytes(), template)
            self.assertEqual((root / 'android/app/src/main/res/drawable-nodpi/ic_launcher_monochrome.png').read_bytes(), android_monochrome)
            run('--source', main_source)
            self.assertEqual((root / 'assets/jianxin-tray-windows.png').read_bytes(), windows_tray)
            self.assertEqual((root / 'assets/jianxin-web.svg').read_bytes(), alternative.read_bytes())
            self.assertEqual((root / 'assets/jianxin-tray.png').read_bytes(), tray)
            run('--reset', 'web')
            self.assertFalse((root / 'assets/logo-overrides/web.svg').exists())
            self.assertEqual((root / 'assets/jianxin-web.svg').read_bytes(), main_source.read_bytes())
            run('--check')
            run('--reset', 'windows-tray')
            self.assertEqual((root / 'assets/jianxin-tray-windows.png').read_bytes(), tray)
            before = {path: path.read_bytes() for path in root.rglob('*') if path.is_file()}
            run('--check', '--source', alternative, success=False)
            run('--reset', 'tray', '--tray-source', alternative, success=False)
            alternative.write_text('<svg/>')
            run('--source', alternative, success=False)
            for path, data in before.items():
                if path != alternative:
                    self.assertEqual(path.read_bytes(), data)
            (root / 'assets/jianxin-web.svg').write_bytes(b'stale')
            run('--check', success=False)

            ico = (root / 'assets/jianxin.ico').read_bytes()
            self.assertEqual(struct.unpack_from('<HHH', ico), (0, 1, 7))
            for index, size in enumerate((16, 24, 32, 48, 64, 128, 256)):
                width, height, _, _, planes, bits, length, offset = struct.unpack_from('<BBBBHHII', ico, 6 + 16 * index)
                self.assertEqual((width or 256, height or 256, planes, bits), (size, size, 1, 32))
                frame = ico[offset:offset + length]
                self.assertEqual(frame[:8], b'\x89PNG\r\n\x1a\n')
                self.assertEqual(struct.unpack('>II', frame[16:24]), (size, size))


if __name__ == '__main__':
    unittest.main()
