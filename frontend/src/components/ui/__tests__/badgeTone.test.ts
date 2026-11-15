// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures every badge tone's ink against its tint, composited over each ground, in both schemes
// ABOUTME: Joins the TS tone table to its index.css rule and the generated token triples, so a missing rule or a failing pair is caught

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { BADGE_TONE, type BadgeTone } from '../badgeTone';

const SRC = path.resolve(__dirname, '../../..');
/*
 * Read because nothing at runtime joins the three: a tone is a class name in
 * `BADGE_TONE`, its fill and ink are declared in index.css, and the colours
 * those tokens resolve to live in the generated token block. jsdom computes
 * no styles, so this is the parity check between the table and its CSS rules,
 * and the contrast measurement over the generated tokens.
 */
const INDEX_CSS = readFileSync(path.join(SRC, 'index.css'), 'utf8');
const TOKENS_CSS = readFileSync(path.join(SRC, 'boreal-tokens.generated.css'), 'utf8');

type Scheme = 'light' | 'dark';
type Rgb = [number, number, number];

/** WCAG 1.4.3 AA for body-size text. */
const AA_TEXT = 4.5;

/** The `--color-*` triples of one scheme: light under `:root`, dark under `html.dark`. */
function tokens(scheme: Scheme): Record<string, Rgb> {
  const darkStart = TOKENS_CSS.indexOf('html.dark {');
  if (darkStart < 0) throw new Error('no html.dark block in the generated tokens');
  const block = scheme === 'light' ? TOKENS_CSS.slice(0, darkStart) : TOKENS_CSS.slice(darkStart);
  const out: Record<string, Rgb> = {};
  for (const match of block.matchAll(/--color-([a-z-]+):\s*(\d+) (\d+) (\d+);/g)) {
    out[match[1]] = [Number(match[2]), Number(match[3]), Number(match[4])];
  }
  return out;
}

const TOKENS: Record<Scheme, Record<string, Rgb>> = { light: tokens('light'), dark: tokens('dark') };

function token(scheme: Scheme, name: string): Rgb {
  const value = TOKENS[scheme][name];
  if (!value) throw new Error(`no --color-${name} in the ${scheme} tokens`);
  return value;
}

/** A fill as a token and an alpha. */
interface Fill {
  token: string;
  alpha: number;
}

/** The background and ink a `.badge-*` class declares in index.css. */
function toneRule(className: string): { fill: Fill; ink: string } {
  const rule = INDEX_CSS.match(new RegExp(`\\.${className}\\s*\\{([^}]*)\\}`));
  if (!rule) throw new Error(`no .${className} rule in index.css`);
  const background = rule[1].match(/background:\s*rgb\(var\(--color-([a-z-]+)\)(?:\s*\/\s*([\d.]+))?\)/);
  const color = rule[1].match(/color:\s*rgb\(var\(--color-([a-z-]+)\)\)/);
  if (!background || !color) throw new Error(`.${className} does not declare a token background and ink`);
  return { fill: { token: background[1], alpha: background[2] ? Number(background[2]) : 1 }, ink: color[1] };
}

/** The fill a `hover:bg-<token>[/NN]` utility paints. */
function hoverFill(hoverClass: string): Fill {
  const match = hoverClass.match(/^hover:bg-([a-z-]+?)(?:\/(\d+))?$/);
  if (!match) throw new Error(`${hoverClass} is not a hover background utility`);
  return { token: match[1], alpha: match[2] ? Number(match[2]) / 100 : 1 };
}

function composite(fill: Fill, scheme: Scheme, ground: Rgb): Rgb {
  const front = token(scheme, fill.token);
  return front.map((f, i) => Math.round(fill.alpha * f + (1 - fill.alpha) * ground[i])) as Rgb;
}

function luminance([r, g, b]: Rgb): number {
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Every ground a badge can sit on, canvas to the highest container tier. */
const GROUNDS = [
  'surface',
  'surface-container-lowest',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-highest',
] as const;

/**
 * The grounds a pressable chip sits on — a reply on the paper canvas, a card,
 * a sheet — where its deeper /25 hover tint must still hold AA. The two high
 * tiers are left out because the /25 tint falls under AA there in light
 * (error reads 4.34:1 over `surface-container-high`); the one reader of
 * `hover`, the verdict chip, sits on a chat reply.
 */
const CHIP_GROUNDS = ['surface', 'surface-container-lowest', 'surface-container-low', 'surface-container'] as const;

const TONES = Object.keys(BADGE_TONE) as BadgeTone[];
const SCHEMES: Scheme[] = ['light', 'dark'];

describe('badge tones', () => {
  it('has an index.css rule for every tone in the table', () => {
    for (const tone of TONES) {
      expect(() => toneRule(BADGE_TONE[tone].tone)).not.toThrow();
    }
  });

  it.each(SCHEMES)('every ink clears AA on its tint over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { fill, ink } = toneRule(BADGE_TONE[tone].tone);
      for (const ground of GROUNDS) {
        const ratio = contrast(token(scheme, ink), composite(fill, scheme, token(scheme, ground)));
        if (ratio < AA_TEXT) failures.push(`${tone} over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it.each(SCHEMES)('every ink clears AA on its hover tint over a chip ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { ink } = toneRule(BADGE_TONE[tone].tone);
      const fill = hoverFill(BADGE_TONE[tone].hover);
      for (const ground of CHIP_GROUNDS) {
        const ratio = contrast(token(scheme, ink), composite(fill, scheme, token(scheme, ground)));
        if (ratio < AA_TEXT) failures.push(`${tone} hover over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('labels error and primary in an ink that clears 6:1 where each bare hue falls under AA on its own tint', () => {
    const error = toneRule(BADGE_TONE.error.tone);
    const primary = toneRule(BADGE_TONE.primary.tone);
    const errorTint = composite(error.fill, 'light', token('light', 'surface-container-low'));
    expect(contrast(token('light', 'error'), errorTint)).toBeLessThan(AA_TEXT);
    expect(contrast(token('light', error.ink), errorTint)).toBeGreaterThanOrEqual(6);
    const primaryTint = composite(primary.fill, 'light', token('light', 'surface-container-high'));
    expect(contrast(token('light', 'primary'), primaryTint)).toBeLessThan(AA_TEXT);
    expect(contrast(token('light', primary.ink), primaryTint)).toBeGreaterThanOrEqual(6);
  });
});
