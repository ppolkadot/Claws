"""Builds the web fonts Claws ships.

Two families: Salted Flutters Polaris, the main font (source .otf files in
Salted-Flutters-Polaris/), and Legacy UI Display as a fallback (source in
assets/fonts/Legacy UI Display/). A family whose source files are absent is skipped.
Polaris is derived from Legacy UI (same metrics and features), so neither the sources nor
the fonts built from them are committed; see README -> Fonts.

The four weights the interface uses are subset to Latin, Greek, Cyrillic and common
punctuation, keeping their OpenType features (tabular figures, kerning, ligatures) and
every name record, and written as WOFF2 to public/fonts/, which Vite copies into the
app as-is. Without any source files the app still builds and falls back to Segoe UI
Variable.

Usage (needs fontTools with WOFF2 support: pip install "fonttools[woff]"):
    python scripts/build-fonts.py
"""

import shutil
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "public" / "fonts"

WEIGHTS = {400: "Regular", 500: "Medium", 600: "Semibold", 700: "Bold"}

# (family, source folder, source file pattern, output file pattern)
FAMILIES = [
    ("Salted Flutters Polaris", ROOT / "Salted-Flutters-Polaris",
     "SaltedFluttersPolaris-{style}.otf", "salted-flutters-polaris-{weight}.woff2"),
    ("Legacy UI Display", ROOT / "assets" / "fonts" / "Legacy UI Display",
     "Legacy UI Display-{style}.otf", "Legacy UI Display-{weight}.woff2"),
]

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


def build(source: Path, target: Path) -> None:
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


def main() -> None:
    built = 0
    for family, folder, source_pattern, target_pattern in FAMILIES:
        sources = {w: folder / source_pattern.format(style=s) for w, s in WEIGHTS.items()}
        missing = [p.name for p in sources.values() if not p.is_file()]
        if missing:
            print(f"{family}: skipped, missing in {folder}: {', '.join(missing)}")
            continue
        OUTPUT.mkdir(parents=True, exist_ok=True)
        for weight, source in sources.items():
            target = OUTPUT / target_pattern.format(weight=weight)
            build(source, target)
            print(f"{source.name:36} {source.stat().st_size // 1024:5} KB -> {target.stat().st_size // 1024:4} KB  {target.relative_to(ROOT)}")
        licence = folder / "LICENSE.txt"
        if licence.is_file():
            shutil.copyfile(licence, OUTPUT / f"{target_pattern.split('-{')[0]}-LICENSE.txt")
        built += 1
    if not built:
        raise SystemExit("No font sources found; the app will fall back to Segoe UI Variable.")


if __name__ == "__main__":
    main()
