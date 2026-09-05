# ABOUTME: Regenerates the web brand assets from the Boreal Ripple master the mobile app already ships.
# ABOUTME: Dev-only asset tool — not wired into the build or CI. Needs Pillow; the PNGs are committed.

import sys
from pathlib import Path

try:
    from PIL import Image, ImageChops
except ImportError:
    sys.exit("Pillow is required: pip install Pillow")

FRONTEND = Path(__file__).resolve().parent.parent
REPO = FRONTEND.parent
MASTER = REPO / "frontend-mobile" / "assets" / "sources" / "boreal-ripple-mark.png"
PUBLIC = FRONTEND / "public"
BRAND = PUBLIC / "brand"

# `surface` from DESIGN.md §2 — the paper the PWA and touch icons are flattened onto.
# Apple rejects an icon with an alpha channel and the maskable icon needs a full bleed,
# so those members of the family carry an opaque ground; the in-app marks are transparent.
SURFACE = (0xF7, 0xF6, 0xF2)

# Three inks and nothing else (DESIGN.md §1): forest on light surfaces, mint on the dark
# canvas. Sage exists for a hero mark on a coloured ground and has no web consumer yet.
INKS = {
    "ink": (0x05, 0x33, 0x1F),
    "mint": (0xA3, 0xD0, 0xBE),
}

# The in-app sizes `DravrLogo` picks from: each is the smallest asset at least twice the
# rendered size, so a 40px rail mark draws from the 96px file on a 2× display.
MARK_SIZES = (96, 192, 512)

# The master is a generative render whose "white" field measures 249-255 rather than a
# flat 255, so a bare inversion carries speckle into the alpha. Same floor as mobile.
SPECKLE_FLOOR = 14

# Share of the canvas the mark spans in each opaque icon — the mobile recipe.
FILL = {
    "pwa-192.png": (192, 0.82),
    "pwa-512.png": (512, 0.82),
    # Android's maskable safe zone is a 66dp circle inside a 72dp reference mask
    # (registre#42 on the mobile side); the same fill keeps the treeline inside it.
    "pwa-maskable-512.png": (512, 0.66),
    "apple-touch-icon.png": (180, 0.82),
}


def ink_mask(image: Image.Image) -> Image.Image:
    """Alpha for the mark: the master is dark ink on a near-white field."""
    mask = ImageChops.invert(image.convert("L"))
    span = 255 - SPECKLE_FLOOR
    return mask.point(lambda v: 0 if v < SPECKLE_FLOOR else min(255, int((v - SPECKLE_FLOOR) * 255 / span)))


def tinted(mask: Image.Image, rgb: tuple[int, int, int], edge: int) -> Image.Image:
    """The mark in one flat ink on a transparent square of `edge` px, centred."""
    width, height = mask.size
    scale = edge / max(width, height)
    size = (max(1, round(width * scale)), max(1, round(height * scale)))
    resized = mask.resize(size, Image.LANCZOS)
    canvas = Image.new("RGBA", (edge, edge), rgb + (0,))
    layer = Image.new("RGBA", size, rgb + (255,))
    layer.putalpha(resized)
    canvas.paste(layer, ((edge - size[0]) // 2, (edge - size[1]) // 2), layer)
    return canvas


def on_paper(mask: Image.Image, edge: int, fill: float) -> Image.Image:
    """The forest mark flattened onto the surface token, at `fill` of the edge."""
    mark = tinted(mask, INKS["ink"], round(edge * fill))
    canvas = Image.new("RGB", (edge, edge), SURFACE)
    offset = ((edge - mark.size[0]) // 2, (edge - mark.size[1]) // 2)
    canvas.paste(mark, offset, mark)
    return canvas


def main() -> None:
    master = Image.open(MASTER).convert("RGB")
    mask = ink_mask(master)
    mask = mask.crop(mask.getbbox())
    BRAND.mkdir(parents=True, exist_ok=True)

    for name, rgb in INKS.items():
        for edge in MARK_SIZES:
            tinted(mask, rgb, edge).save(BRAND / f"mark-{name}-{edge}.png", optimize=True)

    for filename, (edge, fill) in FILL.items():
        on_paper(mask, edge, fill).save(PUBLIC / filename, optimize=True)

    print(f"regenerated {len(INKS) * len(MARK_SIZES)} marks and {len(FILL)} icons from {MASTER.name}")


if __name__ == "__main__":
    main()
