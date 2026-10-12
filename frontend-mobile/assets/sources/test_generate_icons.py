# ABOUTME: Pins the adaptive-icon safe-circle guard in generate-icons.py and the committed asset.
# ABOUTME: Run: python3 -m unittest discover -s frontend-mobile/assets/sources (needs Pillow).
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from PIL import Image

SOURCES = Path(__file__).resolve().parent
ADAPTIVE_ICON = SOURCES.parent / "adaptive-icon.png"

# The generator's file name is not a valid module name, so it is loaded by path.
spec = importlib.util.spec_from_file_location("generate_icons", SOURCES / "generate-icons.py")
icons = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icons)


def canvas_with_pixel(x, y, alpha=255):
    """A transparent adaptive canvas carrying one inked pixel."""
    canvas = Image.new("RGBA", (icons.CANVAS, icons.CANVAS), (0, 0, 0, 0))
    canvas.putpixel((x, y), (0, 0, 0, alpha))
    return canvas


class SafeCircleGuard(unittest.TestCase):
    def test_safe_radius_is_the_66dp_circle_of_a_108dp_canvas(self):
        # Spec constant: Android fixes the adaptive-icon geometry (a 108dp canvas,
        # a 66dp safe circle), so this pins the platform's value, not ours.
        self.assertAlmostEqual(icons.SAFE_RADIUS, 1024 * 33 / 108, places=6)

    def test_ink_radius_measures_to_the_far_corner_of_the_pixel(self):
        # The pixel right of and below the centre point spans (0..1, 0..1) from it.
        centre = icons.CANVAS // 2
        self.assertAlmostEqual(
            icons.ink_radius(canvas_with_pixel(centre, centre)), 2 ** 0.5, places=6
        )

    def test_ink_radius_of_an_empty_canvas_is_zero(self):
        empty = Image.new("RGBA", (icons.CANVAS, icons.CANVAS), (0, 0, 0, 0))
        self.assertEqual(icons.ink_radius(empty), 0.0)

    def test_faint_alpha_is_not_ink(self):
        x, y = icons.CANVAS - 10, icons.CANVAS // 2
        faint = canvas_with_pixel(x, y, alpha=icons.INK_ALPHA_FLOOR)
        visible = canvas_with_pixel(x, y, alpha=icons.INK_ALPHA_FLOOR + 1)
        self.assertEqual(icons.ink_radius(faint), 0.0)
        self.assertGreater(icons.ink_radius(visible), icons.SAFE_RADIUS)

    def test_guard_accepts_ink_just_inside_the_circle(self):
        centre = icons.CANVAS // 2
        inside = canvas_with_pixel(centre + 311, centre)  # far corner 312.0px out
        icons.require_inside_safe_circle(inside)

    def test_guard_exits_on_ink_just_outside_the_circle(self):
        # The next pixel out from the accepted one above: the boundary itself.
        centre = icons.CANVAS // 2
        outside = canvas_with_pixel(centre + 312, centre)  # far corner 313.0px out
        with self.assertRaises(SystemExit) as raised:
            icons.require_inside_safe_circle(outside)
        self.assertIn("66dp safe circle", str(raised.exception))

    def test_guard_exits_on_a_diagonal_overrun_inside_the_bounding_square(self):
        # (260, 260) from the centre is inside the 312.9px bounding box half-width
        # but 367px from the centre: the circle matters, not the box.
        centre = icons.CANVAS // 2
        corner = canvas_with_pixel(centre + 260, centre + 260)
        with self.assertRaises(SystemExit):
            icons.require_inside_safe_circle(corner)


class AdaptiveIcon(unittest.TestCase):
    def test_committed_asset_keeps_every_inked_pixel_inside_the_safe_circle(self):
        # Read from disk on purpose: the committed PNG is what the app ships and no
        # build step regenerates it, so the file is the artefact under test.
        asset = Image.open(ADAPTIVE_ICON)
        self.assertEqual(asset.size, (icons.CANVAS, icons.CANVAS))
        self.assertEqual(asset.mode, "RGBA")
        self.assertLessEqual(icons.ink_radius(asset), icons.SAFE_RADIUS)

    def test_generator_writes_the_family_when_the_adaptive_fill_fits(self):
        # ASSETS is redirected to a scratch directory so a run never rewrites the
        # committed PNGs; everything else in main() runs for real.
        with tempfile.TemporaryDirectory() as out, mock.patch.object(icons, "ASSETS", Path(out)):
            icons.main()
            written = sorted(path.name for path in Path(out).iterdir())
            adaptive = Image.open(Path(out) / "adaptive-icon.png")
            self.assertLessEqual(icons.ink_radius(adaptive), icons.SAFE_RADIUS)
        self.assertEqual(
            written, ["adaptive-icon.png", "favicon.png", "icon.png", "splash-icon.png"]
        )

    def test_generator_writes_nothing_when_the_adaptive_fill_overruns(self):
        # A mark spanning the whole canvas puts ink at 512px along an axis.
        with tempfile.TemporaryDirectory() as out, mock.patch.object(
            icons, "ASSETS", Path(out)
        ), mock.patch.dict(icons.FILL, {"adaptive-icon": icons.CANVAS}):
            with self.assertRaises(SystemExit):
                icons.main()
            self.assertEqual(list(Path(out).iterdir()), [])


if __name__ == "__main__":
    unittest.main()
