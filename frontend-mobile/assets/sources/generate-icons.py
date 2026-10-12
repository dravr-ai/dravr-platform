# ABOUTME: Regenerates the mobile icon family from boreal-ripple-mark.png.
# ABOUTME: Dev-only asset tool — not wired into the build or CI. Needs Pillow.

import math
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

# Android draws a notification's status-bar icon from its alpha channel alone, so
# it is a white silhouette of the mark. 96px is the xxxhdpi size of the 24dp icon;
# the expo-notifications plugin cuts the lower densities from it.
NOTIFICATION_CANVAS = 96
NOTIFICATION_FILL = 84

# Android draws an adaptive icon's 108dp canvas through a launcher mask that shows
# 72dp of it, and recommends keeping key elements inside a centred 66dp circle so a
# tighter OEM mask or a launcher's parallax/zoom does not clip them.
ADAPTIVE_CANVAS_DP = 108
ADAPTIVE_SAFE_DP = 66
SAFE_RADIUS = CANVAS * ADAPTIVE_SAFE_DP / ADAPTIVE_CANVAS_DP / 2  # px, 312.9 at 1024

# Alpha at or below this (8/255, about 3% opacity) is invisible, so it is not ink.
INK_ALPHA_FLOOR = 8

# Share of the canvas the mark spans. The master itself only fills ~47%, which
# reads as a small mark once the OS mask is applied.
FILL = {
    "icon": 840,  # 82% — iOS masks the corners, so the mark can run wide
    # 61% — the largest whole-pixel fill whose ink stays inside SAFE_RADIUS.
    # require_inside_safe_circle() refuses to write the file when a fill or a
    # replacement master breaks that.
    "adaptive-icon": 620,
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


def silhouette(mark_mask, canvas_px, fill_px):
    """The mark as white ink on transparency, scaled to fill_px and centred on a canvas_px square."""
    scale = fill_px / max(mark_mask.size)
    size = (max(1, round(mark_mask.width * scale)), max(1, round(mark_mask.height * scale)))
    ink = Image.new("RGBA", size, (255, 255, 255, 255))
    ink.putalpha(mark_mask.resize(size, Image.LANCZOS))
    canvas = Image.new("RGBA", (canvas_px, canvas_px), (255, 255, 255, 0))
    canvas.paste(ink, ((canvas_px - size[0]) // 2, (canvas_px - size[1]) // 2), ink)
    return canvas


def ink_radius(canvas):
    """Distance from the canvas centre to the far corner of the outermost inked pixel."""
    alpha = canvas.getchannel("A")
    width, height = alpha.size
    centre_x, centre_y = width / 2, height / 2
    data = alpha.tobytes()
    farthest = 0.0
    for y in range(height):
        row = data[y * width : (y + 1) * width]
        inked = [x for x, value in enumerate(row) if value > INK_ALPHA_FLOOR]
        if not inked:
            continue
        # Distance only grows with |dx| on a row, so the row's extremes decide it.
        dx = max(abs(inked[0] + 0.5 - centre_x), abs(inked[-1] + 0.5 - centre_x)) + 0.5
        dy = abs(y + 0.5 - centre_y) + 0.5
        farthest = max(farthest, math.hypot(dx, dy))
    return farthest


def require_inside_safe_circle(canvas):
    """Exit when any ink of the adaptive foreground falls outside the 66dp circle."""
    radius = ink_radius(canvas)
    if radius > SAFE_RADIUS:
        sys.exit(
            f"adaptive-icon: ink reaches {radius:.1f}px from the centre, outside the "
            f"{ADAPTIVE_SAFE_DP}dp safe circle ({SAFE_RADIUS:.1f}px); "
            "lower FILL['adaptive-icon'] or fix the master"
        )


def main():
    master = Image.open(MASTER).convert("RGB")
    mask = ink_mask(master)
    bbox = mask.getbbox()
    mark_rgb, mark_mask = master.crop(bbox), mask.crop(bbox)

    # Checked before anything is written, so a failing run leaves the family untouched.
    adaptive = place(mark_rgb, mark_mask, FILL["adaptive-icon"], transparent=True)
    require_inside_safe_circle(adaptive)

    # App Store and home screen. Apple rejects an icon carrying an alpha
    # channel, so this one is the only opaque member of the family.
    icon = place(mark_rgb, mark_mask, FILL["icon"], transparent=False)
    icon.save(ASSETS / "icon.png")

    adaptive.save(ASSETS / "adaptive-icon.png")
    splash = place(mark_rgb, mark_mask, FILL["splash-icon"], transparent=True)
    splash.save(ASSETS / "splash-icon.png")

    silhouette(mark_mask, NOTIFICATION_CANVAS, NOTIFICATION_FILL).save(ASSETS / "notification-icon.png")

    icon.resize((48, 48), Image.LANCZOS).save(ASSETS / "favicon.png")
    print(f"regenerated icon family from {MASTER.name} (mark bbox {bbox})")


if __name__ == "__main__":
    main()
