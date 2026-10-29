# Mobile icon sources

`boreal-ripple-mark.png` is the master for the mobile icon family — a boreal
forest and its reflection on the left, concentric ripple arcs on the right,
in forest ink on a near-white field.

| Generated asset | Fill | Alpha | Notes |
|---|---|---|---|
| `../icon.png` | 82% | **none** | iOS App Store + home screen. Apple rejects an icon with an alpha channel, so this is flattened onto the `#f9f9f6` surface token. |
| `../adaptive-icon.png` | 61% | yes | Android adaptive foreground. Android draws the 108dp canvas through a launcher mask that shows 72dp of it and recommends keeping key elements inside a 66dp circle, so a tighter OEM mask or a launcher's parallax/zoom does not clip them. At this fill every inked pixel sits inside that circle: the farthest is 312.2px from the centre against the 312.9px allowed on the 1024px canvas, and `generate-icons.py` refuses to write the file when that stops being true. The `#f9f9f6` ground comes from `adaptiveIcon.backgroundColor` in `app.config.js`. |
| `../splash-icon.png` | 59% | yes | Launch screen, over the same `#f9f9f6`. |
| `../favicon.png` | — | none | 48px downsample of `icon.png`. |
| `../notification-icon.png` | 84px of 96px | yes | Android status-bar icon, referenced by the `expo-notifications` plugin in `app.config.js`. Android draws it from the alpha channel alone, so it is the mark as a white silhouette on transparency; the plugin cuts the lower densities from the 96px (xxxhdpi) source. |

Regenerate all five with:

```bash
python3 frontend-mobile/assets/sources/generate-icons.py   # needs Pillow
```

The script is dev-only — no build or CI step runs it, so the PNGs are committed.

The safe-circle guard and the committed `adaptive-icon.png` are pinned by a small
test, also run by hand:

```bash
python3 -m unittest discover -s frontend-mobile/assets/sources   # needs Pillow
```

## Why the master is a raster

This mark arrived as a 1024×1024 generative render with no vector original,
so the pipeline reads the raster directly rather than pretending an SVG master
exists — and so does the web's `frontend/scripts/generate-brand-assets.py`,
which cuts the in-app marks, the PWA icons and the Apple touch icon from this
same file. Two consequences worth knowing before editing:

- The master's background is not a flat white (it measures 249–255), so the
  generator floors near-white to transparent instead of a plain inversion.
- The mark cannot be rescaled beyond 1024px without resampling. Any future
  vectorization should replace `boreal-ripple-mark.png` and keep this pipeline.

`boreal-ripple-mark.svg` covers the sizes above 1024px (print, a large
marketing hero) until a hand-drawn master exists. It is a potrace trace of the
raster on the same 1024 canvas, in one forest `#05331f` fill. Rendered back at
1024px it matches the raster's ink to 98.4% IoU, and at 4096px its edges stay
crisp where an upsample of the PNG blurs. It is derived from the PNG, not a
master: neither generator reads it. A trace keeps the raster's generative
texture and does nothing for the small sizes, so it does not replace the
designer's vector or the reduced favicon mark. Regenerate it with:

```bash
python3 frontend-mobile/assets/sources/trace-mark.py   # needs Pillow and potrace
```

## Known: the mark degrades below ~60px

The dashed concentric arcs merge into a solid smudge at 48–60px, and the tree
silhouettes lose their separation. Rendered at real device sizes, it holds up at
120px and above. This was accepted deliberately so the favicon and notification
icon match the App Store icon rather than carrying a second mark; `BRAND.md`
records the same caution for the Momentum badge.
