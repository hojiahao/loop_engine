"""Render Loop Engine's README artwork with pinned Google Sans Code outlines.

This documentation-only tool requires fonttools==4.60.1. It performs no network
requests and does not participate in the application build.
"""

from __future__ import annotations

import argparse
import hashlib
from html import escape
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent
FONT_HASH = "c9649573afcd966f61096b1e9c3c3115e7b54315ca2adf9839c6f8b17d5f8e1f"
THEMES = {
    "dark": {
        "bg": "#0c1218",
        "panel": "#121d26",
        "line": "#263640",
        "muted": "#94aab5",
        "text": "#edf5f2",
        "accent": "#8ce0c1",
        "soft": "#27453f",
    },
    "light": {
        "bg": "#f3f7f5",
        "panel": "#e9f0ec",
        "line": "#cfdfd7",
        "muted": "#557268",
        "text": "#173d32",
        "accent": "#17785d",
        "soft": "#c4e3d4",
    },
}


def number(value: float) -> str:
    """Use bounded precision so generated files are stable and compact."""
    return f"{value:.2f}".rstrip("0").rstrip(".") or "0"


def outlines(
    font: TTFont,
    label: str,
    x: float,
    y: float,
    size: float,
    color: str,
    weight: int = 400,
) -> str:
    """Shape fixed-width Latin labels as paths without external font loading."""
    glyphs = font.getGlyphSet(location={"wght": weight})
    cmap = font.getBestCmap()
    scale = size / font["head"].unitsPerEm
    pen = SVGPathPen(glyphs, ntos=number)
    for character in label:
        glyph = glyphs[cmap[ord(character)]]
        glyph.draw(TransformPen(pen, (scale, 0, 0, -scale, x, y)))
        x += glyph.width * scale
    return f'<path fill="{color}" d="{pen.getCommands()}"/>'


def svg_open(width: int, height: int, title: str, description: str) -> str:
    """Include accessible text while keeping visible lettering in outlines."""
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" '
        f'width="{width}" height="{height}" role="img" aria-labelledby="title desc">\n'
        f'<title id="title">{escape(title)}</title>\n'
        f'<desc id="desc">{escape(description)}</desc>\n'
        "<!-- Lettering: Google Sans Code, SIL OFL 1.1. See vendor/OFL.txt. -->\n"
    )


def loop_mark(x: int, y: int, scale: float, palette: dict[str, str]) -> str:
    """Draw an original loop symbol with a restrained horizontal light trail."""
    return f'''<g transform="translate({x} {y}) scale({scale})" fill="none">
<path d="M-72 46 H18 M-102 66 H2 M-66 86 H18" stroke="{palette["soft"]}" stroke-width="2"/>
<path d="M-38 152 H22 M-66 172 H42 M-16 192 H62" stroke="{palette["soft"]}" stroke-width="2"/>
<rect x="32" y="18" width="148" height="148" rx="47" stroke="{palette["soft"]}" stroke-width="18"/>
<rect x="90" y="76" width="148" height="148" rx="47" stroke="{palette["accent"]}" stroke-width="18"/>
<path d="M90 116 V123 A43 43 0 0 1 43 166" stroke="{palette["bg"]}" stroke-width="26"/>
<path d="M90 116 V119 A47 47 0 0 1 43 166" stroke="{palette["soft"]}" stroke-width="18"/>
<circle cx="238" cy="149" r="5" fill="{palette["bg"]}" stroke="{palette["accent"]}" stroke-width="3"/>
</g>'''


def banner(font: TTFont, theme: str, compact: bool) -> str:
    """Generate wide and narrow layouts; neither depicts an application screen."""
    p = THEMES[theme]
    width, height = (600, 400) if compact else (1280, 400)
    parts = [
        svg_open(
            width,
            height,
            "Loop Engine",
            "US equity factor research. Provider neutral. Evidence first.",
        ),
        (
            f'<rect x="1" y="1" width="{width - 2}" height="{height - 2}" '
            f'rx="18" fill="{p["bg"]}" stroke="{p["line"]}"/>'
        ),
        f'<path d="M1 54 H{width - 1}" stroke="{p["line"]}"/>',
        f'<circle cx="28" cy="28" r="4" fill="{p["accent"]}"/>',
        outlines(font, "hojiahao / loop_engine", 45, 33, 14, p["muted"]),
    ]
    if compact:
        parts.extend(
            [
                outlines(font, "RESEARCH / VERIFY / REPEAT", 32, 111, 14, p["accent"]),
                outlines(font, "Loop Engine", 28, 185, 68, p["text"], 650),
                outlines(font, "US EQUITY", 32, 232, 18, p["muted"]),
                outlines(font, "FACTOR RESEARCH", 32, 261, 18, p["muted"]),
                f'<path d="M32 305 H568" stroke="{p["line"]}"/>',
                outlines(font, "PROVIDER NEUTRAL", 32, 350, 14, p["accent"], 500),
                outlines(font, "EVIDENCE FIRST", 337, 350, 14, p["muted"], 500),
                loop_mark(457, 214, 0.33, p),
            ]
        )
    else:
        parts.extend(
            [
                outlines(font, "RESEARCH / VERIFY / REPEAT", 64, 116, 16, p["accent"]),
                outlines(font, "Loop Engine", 56, 232, 106, p["text"], 650),
                outlines(font, "US EQUITY FACTOR RESEARCH", 64, 282, 20, p["muted"]),
                f'<path d="M64 319 H765" stroke="{p["line"]}"/>',
                outlines(font, "PROVIDER NEUTRAL", 64, 356, 14, p["accent"], 500),
                outlines(font, "EVIDENCE FIRST", 302, 356, 14, p["muted"], 500),
                loop_mark(912, 98, 1.0, p),
                outlines(font, "[ US EQUITIES ]", 989, 356, 14, p["muted"]),
            ]
        )
    return "\n".join(parts) + "\n</svg>\n"


def badge(font: TTFont, label: str, value: str) -> str:
    """Generate a static factual badge; CI links do not claim a passing build."""
    size = 12
    glyphs = font.getGlyphSet()
    cell = glyphs[font.getBestCmap()[ord("M")]].width * size / font["head"].unitsPerEm
    split = round(len(label) * cell + 24)
    width = round(split + len(value) * cell + 25)
    return "\n".join(
        [
            svg_open(
                width, 30, f"{label}: {value}", "Project technology or workflow link."
            ),
            f'<rect x="0.5" y="0.5" width="{width - 1}" height="29" rx="6" fill="#17222b" stroke="#34464f"/>',
            f'<path d="M{split} 6 V24" stroke="#34464f"/>',
            outlines(font, label, 12, 19, size, "#a6bac3", 450),
            outlines(font, value, split + 12, 19, size, "#a0e4ca", 550),
            "</svg>\n",
        ]
    )


def main() -> None:
    """Build or compare the checked-in assets against the pinned font."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Compare without writing")
    arguments = parser.parse_args()
    source = ROOT / "vendor" / "GoogleSansCode.ttf"
    if hashlib.sha256(source.read_bytes()).hexdigest() != FONT_HASH:
        parser.error("Google Sans Code checksum does not match the pinned source")
    with TTFont(source) as font:
        assets = {
            f"banner-{theme}{'-compact' if compact else ''}.svg": banner(
                font, theme, compact
            )
            for theme in THEMES
            for compact in (False, True)
        }
        for name, label, value in (
            ("market", "Market", "US equities"),
            ("interface", "Interface", "CLI"),
            ("providers", "Models", "pluggable"),
            ("ci", "CI", "checks"),
            ("rust", "Rust", "control"),
            ("typescript", "TypeScript", "providers"),
            ("python", "Python", "research"),
            ("postgresql", "PostgreSQL", "state"),
        ):
            assets[f"badge-{name}.svg"] = badge(font, label, value)
    for name, content in assets.items():
        path = ROOT / name
        if arguments.check:
            if not path.exists() or path.read_text() != content:
                parser.error(f"Asset differs from generation: {name}")
        else:
            path.write_text(content)
    print(
        f"{'Verified' if arguments.check else 'Generated'} {len(assets)} brand assets"
    )


if __name__ == "__main__":
    main()
