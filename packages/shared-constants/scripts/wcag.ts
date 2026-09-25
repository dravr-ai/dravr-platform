// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The WCAG 2.x colour math both stylesheet generators measure their pairings with
// ABOUTME: Hex parsing, alpha compositing over a ground, relative luminance, contrast ratio and the two floors

/** WCAG 1.4.3 for text, 1.4.11 for a graphic or a component's edge. */
export const TEXT_FLOOR = 4.5;
export const GRAPHIC_FLOOR = 3;

export type Rgb = readonly [number, number, number];

/** A `#rrggbb` token as a channel triple. */
export function hexToRgb(hex: string): Rgb {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!match) {
    throw new Error(`not a #rrggbb token: ${hex}`);
  }
  return [parseInt(match[1], 16), parseInt(match[2], 16), parseInt(match[3], 16)];
}

/** `#rrggbb` as the bare `r g b` triple every `--color-*` variable holds (DESIGN.md §2). */
export function triple(hex: string): string {
  return hexToRgb(hex).join(' ');
}

/** A colour drawn at `alpha` over an opaque ground, as the eye receives it. */
export function over(top: Rgb, alpha: number, ground: Rgb): Rgb {
  return [0, 1, 2].map((i) => Math.round(top[i] * alpha + ground[i] * (1 - alpha))) as unknown as Rgb;
}

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function luminance([r, g, b]: Rgb): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** The WCAG contrast ratio between two opaque colours. */
export function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}
