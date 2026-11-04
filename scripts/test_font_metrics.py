"""Regression checks for the web fonts used by Chromium/WebView2.

Run with: python -m unittest discover -s scripts -p "test_font_metrics.py"
Requires the same fonttools[woff] dependency as build-fonts.py.
"""
import unittest
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.pens.recordingPen import RecordingPen

ROOT = Path(__file__).resolve().parent.parent


class FontMetricsTest(unittest.TestCase):
    def test_outline_advances_match_layout_advances(self):
        for weight in (400, 500, 600, 700):
            with self.subTest(weight=weight):
                font = TTFont(ROOT / "public" / "fonts" / f"salted-flutters-polaris-{weight}.woff2")
                charstrings = font["CFF "].cff.topDictIndex[0].CharStrings
                mismatches = []
                for name in font.getGlyphOrder():
                    charstring = charstrings[name]
                    charstring.draw(RecordingPen())
                    expected = font["hmtx"][name][0]
                    if charstring.width != expected:
                        mismatches.append((name, charstring.width, expected))
                self.assertEqual(len(mismatches), 0, f"{len(mismatches)} glyphs have incorrect outline advances: {mismatches[:8]}")


    def test_glyph_shapes_and_attribution_are_preserved(self):
        for weight, style in ((400, "Regular"), (500, "Medium"), (600, "Semibold"), (700, "Bold")):
            with self.subTest(weight=weight):
                source = TTFont(ROOT / "assets" / "fonts" / "polaris" / f"SaltedFluttersPolaris-{style}.otf")
                bundled = TTFont(ROOT / "public" / "fonts" / f"salted-flutters-polaris-{weight}.woff2")
                self.assertEqual([n.toUnicode() for n in source["name"].names if n.nameID < 256],
                                 [n.toUnicode() for n in bundled["name"].names if n.nameID < 256])
                for character in set("Settings Downloads Quick Access AV fi 0123456789+-"):
                    original = source.getBestCmap()[ord(character)]
                    rebuilt = bundled.getBestCmap()[ord(character)]
                    before, after = RecordingPen(), RecordingPen()
                    source.getGlyphSet()[original].draw(before)
                    bundled.getGlyphSet()[rebuilt].draw(after)
                    self.assertEqual(before.value, after.value, (weight, character))


if __name__ == "__main__":
    unittest.main()
