# ABOUTME: Regenerates the mobile icon family from boreal-ripple-mark.png.
# ABOUTME: Dev-only asset tool — not wired into the build or CI. Needs Pillow.

import sys
from pathlib import Path

try:
    from PIL import Image, ImageChops
except ImportError:
    sys.exit("Pillow is required: pip install Pillow")

SOURCES = Path(__file__).resolve().parent
ASSETS = SOURCES.parent
MASTER = SOURCES / "boreal-ripple-mark.png"

# Surface token from DESIGN.md §2 / BRAND.md. The splash declares the same value
# as its backgroundColor, so an opaque icon on this ground has no visible seam.
SURFACE = (0xF9, 0xF9, 0xF6)
CANVAS = 1024

# The master is a generative render: its "white" field measures 249-255 rather
# than a flat 255, so a bare inversion carries speckle into the alpha. Anything
# below this darkness is treated as background.
SPECKLE_FLOOR = 14

# Share of the canvas the mark spans. The master itself only fills ~47%, which
# reads as a small mark once the OS mask is applied.
FILL = {
    "icon": 840,  # 82% — iOS masks the corners, so the mark can run wide
    # LIMITATION(registre#42): FILL["adaptive-icon"] spans 676px, putting 16.7% of
    # the mark's ink outside the 66dp circle Android recommends for key elements,
    # and leaving 1.7px against the 72dp reference mask.
    "adaptive-icon": 676,
    "splash-icon": 600,  # smaller presence behind the launch screen
}


def ink_mask(image):
    """Alpha for the mark: the master is dark ink on a near-white field."""
    mask = ImageChops.invert(image.convert("L"))
    span = 255 - SPECKLE_FLOOR
    return mask.point(
        lambda v: 0 if v < SPECKLE_FLOOR else min(255, int((v - SPECKLE_FLOOR) * 255 / span))
    )


def place(mark_rgb, mark_mask, fill_px, transparent):
    """Scale the mark to fill_px on its longest edge and centre it on the canvas."""
    width, height = mark_mask.size
    scale = fill_px / max(width, height)
    size = (max(1, round(width * scale)), max(1, round(height * scale)))
    resized_rgb = mark_rgb.resize(size, Image.LANCZOS)
    resized_mask = mark_mask.resize(size, Image.LANCZOS)
    origin = ((CANVAS - size[0]) // 2, (CANVAS - size[1]) // 2)

    if transparent:
        canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
        layer = resized_rgb.convert("RGBA")
        layer.putalpha(resized_mask)
        canvas.paste(layer, origin, layer)
        return canvas

    canvas = Image.new("RGB", (CANVAS, CANVAS), SURFACE)
    canvas.paste(resized_rgb, origin, resized_mask)
    return canvas


def main():
    master = Image.open(MASTER).convert("RGB")
    mask = ink_mask(master)
    bbox = mask.getbbox()
    mark_rgb, mark_mask = master.crop(bbox), mask.crop(bbox)

    # App Store and home screen. Apple rejects an icon carrying an alpha
    # channel, so this one is the only opaque member of the family.
    icon = place(mark_rgb, mark_mask, FILL["icon"], transparent=False)
    icon.save(ASSETS / "icon.png")

    for name in ("adaptive-icon", "splash-icon"):
        place(mark_rgb, mark_mask, FILL[name], transparent=True).save(ASSETS / f"{name}.png")

    icon.resize((48, 48), Image.LANCZOS).save(ASSETS / "favicon.png")
    print(f"regenerated icon family from {MASTER.name} (mark bbox {bbox})")


if __name__ == "__main__":
    main()
