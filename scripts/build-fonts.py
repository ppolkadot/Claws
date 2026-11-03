"""Builds the web fonts Claws ships from the Legacy UI Display source files.

The source .otf files live in assets/fonts/Legacy UI Display/ (not committed: Apple's
licence does not allow redistributing them). This script subsets the four weights
the interface uses to Latin, Greek, Cyrillic and common punctuation, keeps their
OpenType features (tabular figures, kerning, ligatures), and writes WOFF2 files to
public/fonts/, which Vite copies into the app as-is.

Without the source files the app still builds and falls back to Segoe UI Variable.

Usage (needs fontTools with WOFF2 support: pip install "fonttools[woff]"):
    python scripts/build-fonts.py
"""

from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets" / "fonts" / "Legacy UI Display"
OUTPUT = ROOT / "public" / "fonts"

WEIGHTS = {400: "Regular", 500: "Medium", 600: "Semibold", 700: "Bold"}

# Titles come from sites all over the world, so the subset keeps every script Legacy UI
# itself covers well. Anything outside it falls back to the system font per glyph.
UNICODES = [
    *range(0x0020, 0x0250),  # Basic Latin, Latin-1, Latin Extended-A and -B
    *range(0x0250, 0x02B0),  # IPA (used in some names)
    *range(0x02B0, 0x0370),  # spacing modifiers, combining diacritics
    *range(0x0370, 0x0400),  # Greek
    *range(0x0400, 0x0530),  # Cyrillic and supplement
    *range(0x1E00, 0x1F00),  # Latin Extended Additional (Vietnamese and more)
    *range(0x2000, 0x2070),  # general punctuation: dashes, quotes, ellipsis
    *range(0x2070, 0x20A0),  # super- and subscripts
    *range(0x20A0, 0x20D0),  # currency
    *range(0x2100, 0x2150),  # letterlike symbols
    *range(0x2150, 0x2190),  # number forms
    *range(0x2190, 0x2200),  # arrows
    *range(0x2200, 0x2300),  # math operators (×, −, ≈)
    0x25CF, 0x2713, 0x2715, 0xFEFF, 0xFFFD,
]


def build(weight: int, style: str) -> Path:
    source = SOURCE / f"Legacy UI Display-{style}.otf"
    target = OUTPUT / f"Legacy UI Display-{weight}.woff2"
    options = subset.Options()
    options.layout_features = ["*"]  # keep tnum, kern, liga, case, ss0x...
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.flavor = "woff2"
    options.desubroutinize = False
    font = TTFont(source)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=UNICODES)
    subsetter.subset(font)
    font.flavor = "woff2"
    font.save(target)
    return target


def main() -> None:
    missing = [s for s in WEIGHTS.values() if not (SOURCE / f"Legacy UI Display-{s}.otf").is_file()]
    if missing:
        raise SystemExit(f"Missing source fonts in {SOURCE}: {', '.join(missing)}")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for weight, style in WEIGHTS.items():
        target = build(weight, style)
        source_kb = (SOURCE / f"Legacy UI Display-{style}.otf").stat().st_size // 1024
        print(f"{style:9} {source_kb:5} KB -> {target.stat().st_size // 1024:4} KB  {target.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
