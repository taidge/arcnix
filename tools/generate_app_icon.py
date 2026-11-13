"""Regenerate Windows/preview assets from the 512x512 SVG (CairoSVG + Pillow)."""
from io import BytesIO
from pathlib import Path

import cairosvg
from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/arcnix-desktop/assets"


def main():
    source = (ASSETS / "arcnix.svg").read_text(encoding="utf-8")
    for name, palette in {
        "dark": {"#538DD0": "#E0F2FC", "#3269AC": "#BCE6F0", "#A7F0DC": "#168C80", "#EAF5FF": "#244E7B", "#36AE99": "#6FE0C1"},
        "light": {"#538DD0": "#32659C", "#3269AC": "#244E7B", "#A7F0DC": "#96E5D2", "#EAF5FF": "#EAF5FF", "#36AE99": "#168C80"},
    }.items():
        variant = source
        for old, new in palette.items():
            variant = variant.replace(old, new)
        (ASSETS / f"arcnix-{name}.svg").write_text(variant, encoding="utf-8")
    png = cairosvg.svg2png(url=str(ASSETS / "arcnix.svg"), output_width=512, output_height=512)
    (ASSETS / "arcnix.png").write_bytes(png)
    image = Image.open(BytesIO(png)).convert("RGBA")
    image.save(
        ASSETS / "arcnix.ico",
        sizes=[(size, size) for size in (16, 20, 24, 32, 40, 48, 64, 128, 256)],
    )
    print("Generated theme SVGs, arcnix.png (512x512) and arcnix.ico (16–256px) from arcnix.svg")


if __name__ == "__main__":
    main()
