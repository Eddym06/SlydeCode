"""Builds assets/brand/slydecode.ico from the 512 px mark.

Dev helper, not part of the build: the .ico is committed, so a normal build needs
neither Python nor Pillow. Run it again only when the mark changes:

    python scripts/make-icon.py

The icon is cropped to the mark plus a small margin so it fills the taskbar tile,
then resampled to every size Windows asks for. Each entry is stored as PNG, which
every Windows since Vista reads.
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "ui" / "static" / "logo-icon.png"
TARGET = ROOT / "assets" / "brand" / "slydecode.ico"
SIZES = [16, 24, 32, 48, 64, 128, 256]
MARGIN = 0.03  # fraction of the mark's size kept around it


def squared_mark(image: Image.Image) -> Image.Image:
    """Crops to the visible mark and pads it to a square with a small margin."""
    box = image.getchannel("A").getbbox()
    if box is None:
        raise SystemExit(f"{SOURCE} has no visible pixels")
    mark = image.crop(box)
    side = round(max(mark.size) * (1 + 2 * MARGIN))
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(mark, ((side - mark.width) // 2, (side - mark.height) // 2), mark)
    return square


def main() -> None:
    source = Image.open(SOURCE).convert("RGBA")
    base = squared_mark(source)
    # Pillow resamples the base image to each size itself; start from the largest.
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    base.resize((256, 256), Image.LANCZOS).save(
        TARGET, format="ICO", sizes=[(size, size) for size in SIZES]
    )
    print(f"wrote {TARGET.relative_to(ROOT)} ({TARGET.stat().st_size} bytes, sizes {SIZES})")


if __name__ == "__main__":
    main()
