"""Generates the SJTU Canvas Downloader icons for Windows (.ico/.png) and macOS (.iconset).

Run from anywhere: python apps/shared/make_icons.py   (requires Pillow)

The artwork is drawn from shapes: a red plate with a white video frame and
a download arrow into a tray.
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
WINDOWS_ASSETS = ROOT / "windows" / "SJTUCanvasDownloader" / "Assets"
MAC_ICONSET = ROOT / "macos" / "Resources" / "AppIcon.iconset"

TOP = (218, 54, 51)
BOTTOM = (148, 16, 38)
SUPERSAMPLE = 2


def gradient(size: int) -> Image.Image:
    image = Image.new("RGB", (size, size))
    pixels = image.load()
    for y in range(size):
        for x in range(size):
            t = (x + y) / (2 * (size - 1))
            pixels[x, y] = tuple(int(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3))
    return image


def master(size: int = 1024) -> Image.Image:
    """macOS proportions: an 824 px plate inside a 1024 px canvas."""
    big = size * SUPERSAMPLE
    canvas = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    plate = int(big * 0.805)
    offset = (big - plate) // 2
    radius = int(plate * 0.225)

    shadow = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (offset, offset + big * 0.012, offset + plate, offset + plate + big * 0.012),
        radius,
        fill=(0, 0, 0, 90),
    )
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(big * 0.018)))

    mask = Image.new("L", (plate, plate), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, plate - 1, plate - 1), radius, fill=255)
    canvas.paste(gradient(plate), (offset, offset), mask)

    def p(x: float, y: float) -> tuple[float, float]:
        return offset + x * plate, offset + y * plate

    white = (255, 255, 255, 255)
    glyphs = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(glyphs)
    # Lecture screen (an outline) with a play triangle.
    draw.rounded_rectangle(
        (*p(0.20, 0.17), *p(0.80, 0.55)), radius=plate * 0.07, outline=white, width=int(plate * 0.05)
    )
    draw.polygon([p(0.445, 0.275), p(0.445, 0.445), p(0.59, 0.36)], fill=white)
    # Download arrow into a tray.
    draw.rounded_rectangle((*p(0.455, 0.585), *p(0.545, 0.69)), radius=plate * 0.012, fill=white)
    draw.polygon([p(0.355, 0.655), p(0.645, 0.655), p(0.50, 0.795)], fill=white)
    draw.rounded_rectangle((*p(0.27, 0.825), *p(0.73, 0.868)), radius=plate * 0.021, fill=white)

    glyph_shadow = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    glyph_shadow.paste((0, 0, 0, 60), (0, int(big * 0.008)), glyphs.split()[3])
    canvas.alpha_composite(glyph_shadow.filter(ImageFilter.GaussianBlur(big * 0.01)))
    canvas.alpha_composite(glyphs)
    return canvas.resize((size, size), Image.LANCZOS)


def main() -> None:
    icon = master()
    WINDOWS_ASSETS.mkdir(parents=True, exist_ok=True)
    # Windows: trim the macOS safe area so the plate fills the tile.
    trimmed = icon.crop((80, 80, 944, 944)).resize((256, 256), Image.LANCZOS)
    trimmed.save(
        WINDOWS_ASSETS / "AppIcon.ico",
        sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (96, 96), (128, 128), (256, 256)],
    )
    trimmed.save(WINDOWS_ASSETS / "AppIcon.png")
    MAC_ICONSET.mkdir(parents=True, exist_ok=True)
    for points in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            pixels = points * scale
            name = f"icon_{points}x{points}{'@2x' if scale == 2 else ''}.png"
            icon.resize((pixels, pixels), Image.LANCZOS).save(MAC_ICONSET / name)
    icon.save(ROOT / "shared" / "AppIcon-1024.png")
    print("icons written")


if __name__ == "__main__":
    main()
