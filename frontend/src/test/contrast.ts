// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web tone tests' grounds and rules over the one WCAG calculator in @pierre/shared-constants — token triples, index.css tone rules
// ABOUTME: Reads what a table's classes resolve to and composites a fill over a ground; the ratio itself is shared-constants'

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { GRAPHIC_FLOOR, TEXT_FLOOR, contrast, over, type Rgb } from '@pierre/shared-constants';

export { contrast };

const SRC = path.resolve(__dirname, '..');
/*
 * Read because nothing at runtime joins the three: a tone is a class name in
 * a table, its fill and ink are declared in index.css, and the colours those
 * tokens resolve to live in the generated token block. jsdom computes no
 * styles, so a tone test is the parity check between its table and the CSS
 * rules, and the contrast measurement over the generated tokens.
 */
const INDEX_CSS = readFileSync(path.join(SRC, 'index.css'), 'utf8');
const TOKENS_CSS = readFileSync(path.join(SRC, 'boreal-tokens.generated.css'), 'utf8');

export type Scheme = 'light' | 'dark';
export type { Rgb };

export const SCHEMES: Scheme[] = ['light', 'dark'];

/** WCAG 1.4.3 AA for body-size text. */
export const AA_TEXT = TEXT_FLOOR;

/** WCAG 1.4.11 for a glyph or a component's edge. */
export const AA_GRAPHIC = GRAPHIC_FLOOR;

/** The `--color-*` triples of one scheme: light under `:root`, dark under `html.dark`. */
function tokens(scheme: Scheme): Record<string, Rgb> {
  const darkStart = TOKENS_CSS.indexOf('html.dark {');
  if (darkStart < 0) throw new Error('no html.dark block in the generated tokens');
  const block = scheme === 'light' ? TOKENS_CSS.slice(0, darkStart) : TOKENS_CSS.slice(darkStart);
  const out: Record<string, Rgb> = {};
  for (const match of block.matchAll(/--color-([a-z-]+):\s*(\d+) (\d+) (\d+);/g)) {
    out[match[1]] = [Number(match[2]), Number(match[3]), Number(match[4])] as const;
  }
  return out;
}

const TOKENS: Record<Scheme, Record<string, Rgb>> = { light: tokens('light'), dark: tokens('dark') };

export function token(scheme: Scheme, name: string): Rgb {
  const value = TOKENS[scheme][name];
  if (!value) throw new Error(`no --color-${name} in the ${scheme} tokens`);
  return value;
}

/** A fill as a token and an alpha. */
export interface Fill {
  token: string;
  alpha: number;
}

/** The background and ink a tone class (`.badge-*`, `.alert-*`) declares in index.css. */
export function toneRule(className: string): { fill: Fill; ink: string } {
  const rule = INDEX_CSS.match(new RegExp(`\\.${className}\\s*\\{([^}]*)\\}`));
  if (!rule) throw new Error(`no .${className} rule in index.css`);
  const background = rule[1].match(/background:\s*rgb\(var\(--color-([a-z-]+)\)(?:\s*\/\s*([\d.]+))?\)/);
  const color = rule[1].match(/(?:^|[\s;])color:\s*rgb\(var\(--color-([a-z-]+)\)\)/);
  if (!background || !color) throw new Error(`.${className} does not declare a token background and ink`);
  return { fill: { token: background[1], alpha: background[2] ? Number(background[2]) : 1 }, ink: color[1] };
}

/** The fill a `bg-<token>[/NN]` utility paints, with or without a `hover:` prefix. */
export function utilityFill(bgClass: string): Fill {
  const match = bgClass.match(/^(?:hover:)?bg-([a-z-]+?)(?:\/(\d+))?$/);
  if (!match) throw new Error(`${bgClass} is not a background utility`);
  return { token: match[1], alpha: match[2] ? Number(match[2]) / 100 : 1 };
}

/** The token a `text-<token>` utility names. */
export function utilityInk(textClass: string): string {
  const match = textClass.match(/^text-([a-z-]+)$/);
  if (!match) throw new Error(`${textClass} is not a text-colour utility`);
  return match[1];
}

export function composite(fill: Fill, scheme: Scheme, ground: Rgb): Rgb {
  return over(token(scheme, fill.token), fill.alpha, ground);
}

/** The contrast of an ink token on a fill composited over a ground token, in one scheme. */
export function inkOnFill(scheme: Scheme, ink: string, fill: Fill, ground: string): number {
  return contrast(token(scheme, ink), composite(fill, scheme, token(scheme, ground)));
}

/** Every ground a tinted surface can sit on, canvas to the highest container tier. */
export const GROUNDS = [
  'surface',
  'surface-container-lowest',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-highest',
] as const;
