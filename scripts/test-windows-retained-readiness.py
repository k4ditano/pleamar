"""A bright desktop must never stand in for the fixture's first text frame."""
from pathlib import Path
import importlib.util
import unittest

spec = importlib.util.spec_from_file_location('retained', Path(__file__).with_name('windows-retained-ci.py'))
retained = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retained)

class Readiness(unittest.TestCase):
    def test_desktop_then_panel_then_glyphs(self):
        width, height = 640, 480
        pixels = bytearray(b'\xff\xff\xff\xff' * width * height)
        self.assertFalse(retained.title_ready((width, height, pixels)))
        for y in range(28, 73):
            for x in range(32, 612):
                offset = (y * width + x) * 4
                pixels[offset:offset + 4] = b'\x27\x21\x17\xff'
        self.assertFalse(retained.title_ready((width, height, pixels)))
        for y in range(40, 50):
            for x in range(50, 75):
                offset = (y * width + x) * 4
                pixels[offset:offset + 4] = b'\xff\xff\xff\xff'
        self.assertTrue(retained.title_ready((width, height, pixels)))

if __name__ == '__main__':
    unittest.main()
