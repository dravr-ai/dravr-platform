# ABOUTME: Traces boreal-ripple-mark.png into boreal-ripple-mark.svg, a vector for sizes above 1024px.
# ABOUTME: Dev-only asset tool — not wired into the build or CI. Needs Pillow and potrace; the SVG is committed.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    sys.exit("Pillow is required: pip install Pillow")

SOURCES = Path(__file__).resolve().parent
MASTER = SOURCES / "boreal-ripple-mark.png"
TRACE = SOURCES / "boreal-ripple-mark.svg"

# Forest, the mark's ink on light surfaces (DESIGN.md §1). The SVG carries one
# fill, so a consumer recolours it to mint or sage by replacing this value.
INK = "#05331f"
CANVAS = 1024

# potrace fits curves to pixel edges, so it traces a 4× LANCZOS upsample: the
# outline follows the master's anti-aliasing instead of its 1px staircase.
UPSAMPLE = 4

# The master is dark ink on a near-white field; mid-grey splits the two and puts
# the outline on the anti-aliased edge's midpoint.
INK_THRESHOLD = 128

# Specks smaller than this many upsampled pixels (1px of the master) are render
# noise, not ink.
TURDSIZE = 16

# Curve-fitting tolerance: larger merges more segments into one Bézier. 0.4
# halves the path data against potrace's 0.2 with no visible loss at 4096px.
OPTTOLERANCE = 0.4

HEADER = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS} {CANVAS}" width="{CANVAS}" height="{CANVAS}"><title>Dravr</title><desc>Boreal Ripple mark, traced by potrace from the 1024px raster master (trace-mark.py). Not a hand-drawn vector: carnet#339 tracks that.</desc>"""


def ink_bitmap(path):
    """The master upsampled and split into ink (black) and field (white) for potrace."""
    grey = Image.open(path).convert("L")
    grey = grey.resize((grey.width * UPSAMPLE, grey.height * UPSAMPLE), Image.LANCZOS)
    return grey.point(lambda v: 0 if v < INK_THRESHOLD else 255).convert("1")


def trace(bitmap):
    """potrace's SVG body: the path group, with its coordinates kept as potrace writes them."""
    with tempfile.TemporaryDirectory() as work:
        pbm = Path(work) / "ink.pbm"
        svg = Path(work) / "ink.svg"
        bitmap.save(pbm)
        subprocess.run(
            [
                "potrace", str(pbm), "--svg", "--output", str(svg),
                "--turdsize", str(TURDSIZE), "--opttolerance", str(OPTTOLERANCE),
            ],
            check=True,
        )
        raw = svg.read_text()
    paths = re.findall(r'<path d="[^"]*"/>', raw)
    if not paths:
        sys.exit("potrace produced no paths")
    return paths


def main():
    if shutil.which("potrace") is None:
        sys.exit("potrace is required: brew install potrace")

    paths = trace(ink_bitmap(MASTER))
    # potrace writes integer coordinates in tenths of an upsampled pixel with the
    # y axis pointing up; this transform maps them back onto the 1024px canvas.
    scale = 1 / (10 * UPSAMPLE)
    group = f'<g transform="translate(0,{CANVAS}) scale({scale},-{scale})" fill="{INK}">'
    body = "".join(" ".join(p.split()) for p in paths)
    TRACE.write_text(f"{HEADER}{group}{body}</g></svg>\n")
    print(f"{TRACE.name}: {len(paths)} paths, {TRACE.stat().st_size} bytes")


if __name__ == "__main__":
    main()
