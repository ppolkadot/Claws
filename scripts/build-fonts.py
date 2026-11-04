"""Rebuild the bundled Salted Flutters Polaris web fonts from committed sources.

The four source weights and LICENSE.txt live in assets/fonts/polaris/. Outputs
are committed in public/fonts/ so normal app builds do not require Python.
Usage: pip install "fonttools[woff]" && python scripts/build-fonts.py
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
    ("Salted Flutters Polaris", ROOT / "assets" / "fonts" / "polaris",
     "SaltedFluttersPolaris-{style}.otf", "salted-flutters-polaris-{weight}.woff2"),
]

# Keep the scripts used by titles from sites around the world.
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


def repair_outline_advances(font: TTFont) -> None:
    """Keep CFF outline widths consistent with hmtx for Chromium's font sanitizer.

    The supplied outlines often omit their advance and inherit one oversized
    default. hmtx has the intended proportional widths. Expand subroutines first
    so each initial width can be replaced without changing drawing instructions.
    """
    cff = font["CFF "].cff
    cff.desubroutinize()
    charstrings = cff.topDictIndex[0].CharStrings
    width_parity = {"rmoveto": 0, "hmoveto": 1, "vmoveto": 1,
                    "hstem": 0, "vstem": 0, "hstemhm": 0, "vstemhm": 0,
                    "endchar": 0}
    for name in font.getGlyphOrder():
        charstring = charstrings[name]
        charstring.decompile()
        program = list(charstring.program)
        operator_index = next(i for i, value in enumerate(program) if isinstance(value, str))
        operator = program[operator_index]
        if operator not in width_parity:
            raise ValueError(f"Unsupported initial CFF operator {operator!r} in {name}")
        if (operator_index % 2) ^ width_parity[operator]:
            program.pop(0)  # Remove an explicitly encoded advance, if present.
        advance = font["hmtx"][name][0]
        if advance != charstring.private.defaultWidthX:
            program.insert(0, advance - charstring.private.nominalWidthX)
        charstring.program = program


def build(source: Path, target: Path) -> None:
    options = subset.Options()
    options.layout_features = ["*"]  # keep tnum, kern, liga, case, ss0x...
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.flavor = "woff2"
    options.desubroutinize = False
    font = TTFont(source, recalcTimestamp=False)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=UNICODES)
    subsetter.subset(font)
    repair_outline_advances(font)
    font.flavor = "woff2"
    font.save(target)


def main() -> None:
    built = 0
    for family, folder, source_pattern, target_pattern in FAMILIES:
        sources = {w: folder / source_pattern.format(style=s) for w, s in WEIGHTS.items()}
        missing = [p.name for p in sources.values() if not p.is_file()]
        if not (folder / "LICENSE.txt").is_file():
            missing.append("LICENSE.txt")
        if missing:
            raise SystemExit(f"{family}: missing sources in {folder}: {', '.join(missing)}")
        OUTPUT.mkdir(parents=True, exist_ok=True)
        for weight, source in sources.items():
            target = OUTPUT / target_pattern.format(weight=weight)
            build(source, target)
            print(f"{source.name:36} {source.stat().st_size // 1024:5} KB -> {target.stat().st_size // 1024:4} KB  {target.relative_to(ROOT)}")
        licence = folder / "LICENSE.txt"
        if licence.is_file():
            shutil.copyfile(licence, OUTPUT / "LICENSE.txt")
        built += 1
    if not built:
        raise SystemExit("No Polaris font sources found.")


if __name__ == "__main__":
    main()
