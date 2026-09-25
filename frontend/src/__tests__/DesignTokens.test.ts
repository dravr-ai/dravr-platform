// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures the Boreal surface ladder instead of trusting it — the separation ratios of DESIGN.md §2, computed
// ABOUTME: Also pins DESIGN.md's token table to the shared tokens and the two shell rules DESIGN.md §5 states

import fs from 'fs';
import path from 'path';
import { describe, it, expect } from 'vitest';
import { BOREAL_LIGHT, BOREAL_DARK, BORDER_INK, PRIMARY_HOVER, ghostBorder } from '@pierre/shared-constants';

// The WCAG 4.5:1 floors — every text role on every tier, every bound ink on
// its tint, every filled pair — are measured by the generator that writes the
// client stylesheets from these tokens, which refuses to write below them
// (packages/shared-constants/__tests__/client-css.test.ts). This file holds
// the design's own targets above that floor.

const REPO = path.join(__dirname, '..', '..', '..');
const DESIGN_MD = fs.readFileSync(path.join(REPO, 'frontend', 'DESIGN.md'), 'utf8');

/**
 * Thresholds from DESIGN.md §2 "Light tier separation".
 *
 * Light separates on fill because fill is all it has: the pale ghost-border
 * hairline that gives the dark scheme a second channel is invisible on white,
 * and the shadow recipe is faint. The raised floor is the separation dark
 * already carried for the same pair (1.20:1) and the one the messengers this
 * layout follows sit at (WhatsApp 1.20:1, Telegram 1.24:1).
 */
const TIER_STEP_MIN = 1.06;
const RAISED_STEP_MIN = 1.18;

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance of a `#rrggbb` string. */
function luminance(hex: string): number {
  const n = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => channel(parseInt(n.slice(i, i + 2), 16)));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2.x contrast ratio between two `#rrggbb` strings. */
function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Pull `| \`token\` | \`#hex\` |` style rows out of a DESIGN.md table. */
function designMdHex(token: string): string {
  const row = new RegExp(`\\|\\s*\`${token}\`\\s*\\|\\s*\`(#[0-9a-f]{6})\``).exec(DESIGN_MD);
  expect(row, `DESIGN.md row for \`${token}\``).not.toBeNull();
  return (row as RegExpExecArray)[1];
}

/** The light ladder, lightest first — the order a surface stacks in. */
const LIGHT_LADDER: ReadonlyArray<readonly [string, string]> = [
  ['surface-container-lowest', BOREAL_LIGHT.surfaceContainerLowest],
  ['surface', BOREAL_LIGHT.surface],
  ['surface-container-low', BOREAL_LIGHT.surfaceContainerLow],
  ['surface-container', BOREAL_LIGHT.surfaceContainer],
  ['surface-container-high', BOREAL_LIGHT.surfaceContainerHigh],
  ['surface-container-highest', BOREAL_LIGHT.surfaceContainerHighest],
];

describe('the contrast helper agrees with WCAG', () => {
  it('reproduces the reference ratios', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrast('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
    // The canonical AA example: #767676 is the darkest grey that passes on white.
    expect(contrast('#767676', '#ffffff')).toBeGreaterThanOrEqual(4.5);
    expect(contrast('#777777', '#ffffff')).toBeLessThan(4.5);
  });
});

describe('light surface ladder — DESIGN.md §2 "Light tier separation"', () => {
  it('separates every adjacent tier on fill alone', () => {
    // `surface` against `surface-container-lowest` is the documented exemption:
    // a card on the page canvas is lifted by the ghost-border hairline, so
    // those two tones stay a half-step apart on purpose.
    const measured = LIGHT_LADDER.slice(1).map(([name, hex], index) => {
      const [aboveName, aboveHex] = LIGHT_LADDER[index];
      return { pair: `${aboveName} → ${name}`, ratio: contrast(aboveHex, hex) };
    });

    expect(measured[0].pair).toBe('surface-container-lowest → surface');
    expect(measured[0].ratio).toBeCloseTo(1.08, 2);

    for (const step of measured.slice(1)) {
      expect(step.ratio, `${step.pair} measured ${step.ratio.toFixed(3)}:1`).toBeGreaterThanOrEqual(
        TIER_STEP_MIN,
      );
    }
  });

  it('lifts a raised surface off the canvas under it in both schemes', () => {
    // Light: a white bubble/card on the thread canvas. This measured 1.05:1
    // before the tiers were re-valued, which is what "no coach bubble at all"
    // looked like on a light phone.
    expect(
      contrast(BOREAL_LIGHT.surfaceContainerLowest, BOREAL_LIGHT.surfaceContainerLow),
    ).toBeGreaterThanOrEqual(RAISED_STEP_MIN);

    // Dark: `surface-container-high` on the same canvas, where "lowest" would
    // sink below the page. This is the pair light was measured against.
    expect(
      contrast(BOREAL_DARK.surfaceContainerHigh, BOREAL_DARK.surfaceContainerLow),
    ).toBeGreaterThanOrEqual(RAISED_STEP_MIN);
  });
});

describe('primary is a usable green in both schemes', () => {
  it('reads as a colour in light, not as black', () => {
    // The v1 primary (#00241a) was so deep it needed a separate `brand` ink
    // to put any green on screen. One token now carries both roles.
    expect(BOREAL_LIGHT.primary).toBe('#255f4d');
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(BOREAL_LIGHT.primary.slice(i, i + 2), 16));
    expect(g).toBeGreaterThan(r + 40);
    expect(g).toBeGreaterThan(b + 10);
    // White on the filled primary, and the filled hover under the same white.
    expect(contrast('#ffffff', BOREAL_LIGHT.primary)).toBeGreaterThanOrEqual(7);
    expect(contrast('#ffffff', PRIMARY_HOVER.light)).toBeGreaterThanOrEqual(7);
  });

  it('keeps the athlete bubble ink well clear of its tint in both schemes', () => {
    expect(contrast(BOREAL_LIGHT.onPrimaryContainer, BOREAL_LIGHT.primaryContainer)).toBeGreaterThanOrEqual(7);
    expect(contrast(BOREAL_DARK.onPrimaryContainer, BOREAL_DARK.primaryContainer)).toBeGreaterThanOrEqual(7);
  });
});

describe('DESIGN.md states the shipped token values', () => {
  const documented: ReadonlyArray<readonly [string, keyof typeof BOREAL_LIGHT]> = [
    ['surface', 'surface'],
    ['surface-container-lowest', 'surfaceContainerLowest'],
    ['surface-container-low', 'surfaceContainerLow'],
    ['surface-container', 'surfaceContainer'],
    ['surface-container-high', 'surfaceContainerHigh'],
    ['surface-container-highest', 'surfaceContainerHighest'],
    ['outline', 'outline'],
    ['primary', 'primary'],
    ['primary-container', 'primaryContainer'],
    ['on-primary-container', 'onPrimaryContainer'],
  ];

  it.each(documented)('the %s row carries the shared-constants value', (cssName, tsName) => {
    expect(designMdHex(cssName)).toBe(BOREAL_LIGHT[tsName]);
  });

  it('gives light the darker ghost-border ink and dark the pale one', () => {
    // A hairline has to contrast with what it sits on, and the two grounds are
    // opposite. Mobile once shipped the dark ink in both schemes.
    const ink = (rgb: string) => `#${rgb.split(', ').map((v) => Number(v).toString(16).padStart(2, '0')).join('')}`;
    expect(luminance(ink(BORDER_INK.light.rgb))).toBeLessThan(luminance(ink(BORDER_INK.dark.rgb)));
    expect(DESIGN_MD).toContain(ghostBorder(BORDER_INK.light, 'default'));
  });
});

describe('DESIGN.md §5 states one shell rule per client', () => {
  const regionTable = DESIGN_MD.slice(
    DESIGN_MD.indexOf('### Chat surfaces — the messenger layout'),
    DESIGN_MD.indexOf('### Focus rings'),
  );

  it('keeps the web rail name-free, with the mark as the way Home', () => {
    const rail = regionTable.split('\n').find((line) => line.includes('Icon rail (72px)'));
    expect(rail, 'the icon rail row').toBeDefined();
    expect(rail).toContain('web only');
    expect(rail).toContain('No name or role text');
    expect(rail).toContain('a button to Home');
  });

  it('gives the phone the mark and the wordmark on its Home and Chat tabs only', () => {
    const mobile = regionTable.split('\n').find((line) => line.includes('Home and Chat tab headers'));
    expect(mobile, 'the mobile shell row').toBeDefined();
    expect(mobile).toContain('mobile only');
    expect(mobile).toMatch(/mark \*\*and\*\* the DRAVR wordmark/);
    expect(mobile).toContain('in place of the screen title');
    expect(mobile).toContain('keep their own titles');
  });

  it('says why the two shells differ rather than leaving a contradiction', () => {
    expect(regionTable).toContain('Mark only on web, mark plus name on the phone');
    expect(regionTable).toContain('72px column');
    expect(regionTable).toContain('The phone has no rail');
  });
});
