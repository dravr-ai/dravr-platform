# Mobile icon sources

`boreal-ripple-mark.png` is the master for the mobile icon family — a boreal
forest and its reflection on the left, concentric ripple arcs on the right,
in forest ink on a near-white field.

| Generated asset | Fill | Alpha | Notes |
|---|---|---|---|
| `../icon.png` | 82% | **none** | iOS App Store + home screen. Apple rejects an icon with an alpha channel, so this is flattened onto the `#f9f9f6` surface token. |
| `../adaptive-icon.png` | 66% | yes | Android adaptive foreground. The reference launcher mask is a 72dp circle, and Android recommends keeping key elements inside a 66dp one; at this fill nothing is cropped by the mask but 16.7% of the ink falls outside that inner circle (registre#42). The `#f9f9f6` ground comes from `adaptiveIcon.backgroundColor` in `app.config.js`. |
| `../splash-icon.png` | 59% | yes | Launch screen, over the same `#f9f9f6`. |
| `../favicon.png` | — | none | 48px downsample of `icon.png`. |

Regenerate all four with:

```bash
python3 frontend-mobile/assets/sources/generate-icons.py   # needs Pillow
```

The script is dev-only — no build or CI step runs it, so the PNGs are committed.

## Why the master is a raster

The rest of the brand is vector (`dravr-logo.svg`, and the web marks under
`frontend/`). This mark arrived as a 1024×1024 generative render with no vector
original, so the pipeline reads the raster directly rather than pretending an
SVG master exists. Two consequences worth knowing before editing:

- The master's background is not a flat white (it measures 249–255), so the
  generator floors near-white to transparent instead of a plain inversion.
- The mark cannot be rescaled beyond 1024px without resampling. Any future
  vectorization should replace `boreal-ripple-mark.png` and keep this pipeline.

## Known: the mark degrades below ~60px

The dashed concentric arcs merge into a solid smudge at 48–60px, and the tree
silhouettes lose their separation. Rendered at real device sizes, it holds up at
120px and above. This was accepted deliberately so the favicon and notification
icon match the App Store icon rather than carrying a second mark; `BRAND.md`
records the same caution for the Momentum badge.

## Divergence from BRAND.md

`frontend/BRAND.md` documents the **Momentum** ribbon mark as the badge for web,
server, marketing and mobile alike. As of this change the mobile icon family
uses this mark instead, while every web and marketing surface still ships
Momentum. That split is deliberate and currently unreconciled — see the
"Framing variants" table in `BRAND.md`.
