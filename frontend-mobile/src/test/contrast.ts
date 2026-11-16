// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone tone tests' grounds over the one WCAG calculator in @pierre/shared-constants — a class's colour, a fill composited on a ground
// ABOUTME: Lives outside __tests__ so jest does not run it as a suite; the ratio itself is shared-constants'

import { readFileSync } from 'fs';
import { join } from 'path';
import {
  BOREAL_DARK,
  BOREAL_LIGHT,
  GRAPHIC_FLOOR,
  TEXT_FLOOR,
  contrastHex,
  hexToRgb,
  over,
} from '@pierre/shared-constants';

export type Scheme = 'light' | 'dark';

export const SCHEMES: Scheme[] = ['light', 'dark'];

/** WCAG 1.4.3 AA for body-size text. */
export const AA_TEXT = TEXT_FLOOR;

/** WCAG 1.4.11 for a glyph or a component's edge. */
export const AA_GRAPHIC = GRAPHIC_FLOOR;

/**
 * The generated token block both schemes' classes resolve through. Read
 * because a tone is a class name while a runtime ink and the contrast grounds
 * are hexes, and nothing at runtime joins the two: a tone test is a parity
 * check between the generated CSS and the live palette.
 */
const TOKENS_CSS = readFileSync(join(__dirname, '../../boreal-tokens.generated.css'), 'utf8');

/** The `#rrggbb` a `text-<token>`, `bg-<token>[/NN]` or `border-<token>[/NN]` class names in one scheme. */
export function classColor(colorClass: string, scheme: Scheme): string {
  const darkStart = TOKENS_CSS.indexOf(':root.dark');
  const block = scheme === 'light' ? TOKENS_CSS.slice(0, darkStart) : TOKENS_CSS.slice(darkStart);
  const token = colorClass.replace(/^(text|bg|border)-/, '').replace(/\/\d+$/, '');
  const match = block.match(new RegExp(`--color-${token}:\\s*(\\d+) (\\d+) (\\d+);`));
  if (!match) throw new Error(`--color-${token} missing (${scheme})`);
  return `#${match
    .slice(1, 4)
    .map((v) => Number(v).toString(16).padStart(2, '0'))
    .join('')}`;
}

/** The WCAG 2.x contrast ratio between two `#rrggbb` strings — the shared calculator. */
export const contrast = contrastHex;

/** `#rrggbb` drawn at `alpha` over an opaque `#rrggbb` ground, as a renderer composites it — the shared `over`. */
export function hexOver(fg: string, alpha: number, ground: string): string {
  const seen = over(hexToRgb(fg), alpha, hexToRgb(ground));
  return `#${seen.map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/** The colour a `bg-<token>` or `bg-<token>/NN` fill resolves to over an opaque ground. */
export function fillOver(fillClass: string, scheme: Scheme, ground: string): string {
  const alphaMatch = fillClass.match(/\/(\d+)$/);
  const alpha = alphaMatch ? Number(alphaMatch[1]) / 100 : 1;
  return hexOver(classColor(fillClass, scheme), alpha, ground);
}

/**
 * The grounds a tinted surface can sit on: every tier from the canvas to
 * `surface-container-highest`, in both schemes — the card fill of each scheme
 * (`surface-container-lowest` in light, `surface-container-high` in dark), the
 * tiers a sheet or picker row uses, and the top tier for good measure.
 */
export const GROUNDS: Record<Scheme, ReadonlyArray<readonly [string, string]>> = {
  light: [
    ['surface', BOREAL_LIGHT.surface],
    ['surface-container-lowest', BOREAL_LIGHT.surfaceContainerLowest],
    ['surface-container-low', BOREAL_LIGHT.surfaceContainerLow],
    ['surface-container', BOREAL_LIGHT.surfaceContainer],
    ['surface-container-high', BOREAL_LIGHT.surfaceContainerHigh],
    ['surface-container-highest', BOREAL_LIGHT.surfaceContainerHighest],
  ],
  dark: [
    ['surface', BOREAL_DARK.surface],
    ['surface-container-lowest', BOREAL_DARK.surfaceContainerLowest],
    ['surface-container-low', BOREAL_DARK.surfaceContainerLow],
    ['surface-container', BOREAL_DARK.surfaceContainer],
    ['surface-container-high', BOREAL_DARK.surfaceContainerHigh],
    ['surface-container-highest', BOREAL_DARK.surfaceContainerHighest],
  ],
};
