// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures the Boreal surface ladder instead of trusting it — every ratio in DESIGN.md §2, computed
// ABOUTME: Also pins the three token mirrors and the two shell rules DESIGN.md §5 states for web and mobile

import fs from 'fs';
import path from 'path';
import { describe, it, expect } from 'vitest';
import { BOREAL_LIGHT, BOREAL_DARK } from '@pierre/shared-constants';

const REPO = path.join(__dirname, '..', '..', '..');
const DESIGN_MD = fs.readFileSync(path.join(REPO, 'frontend', 'DESIGN.md'), 'utf8');
const WEB_CSS = fs.readFileSync(path.join(REPO, 'frontend', 'src', 'index.css'), 'utf8');
const MOBILE_CSS = fs.readFileSync(path.join(REPO, 'frontend-mobile', 'global.css'), 'utf8');

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
/** WCAG 1.4.3 AA for body-size text. */
const AA_TEXT = 4.5;

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

/** Read a CSS custom property's RGB triplet as `#rrggbb`. `nth` 1 = light, 2 = dark. */
function cssToken(source: string, token: string, nth: 1 | 2): string {
  const matches = [...source.matchAll(new RegExp(`--color-${token}:\\s*(\\d+) (\\d+) (\\d+);`, 'g'))];
  const hit = matches[nth - 1];
  expect(hit, `--color-${token} occurrence ${nth}`).toBeDefined();
  return `#${[hit[1], hit[2], hit[3]].map((v) => Number(v).toString(16).padStart(2, '0')).join('')}`;
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
    // a card on the page canvas is lifted by the ghost border and the two-layer
    // shadow, so those two tones stay a half-step apart on purpose.
    const measured = LIGHT_LADDER.slice(1).map(([name, hex], index) => {
      const [aboveName, aboveHex] = LIGHT_LADDER[index];
      return { pair: `${aboveName} → ${name}`, ratio: contrast(aboveHex, hex) };
    });

    expect(measured[0].pair).toBe('surface-container-lowest → surface');
    expect(measured[0].ratio).toBeCloseTo(1.05, 2);

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

  it('keeps every text role at WCAG AA on every light tier', () => {
    const inks: ReadonlyArray<readonly [string, string]> = [
      ['on-surface', BOREAL_LIGHT.onSurface],
      ['on-surface-variant', BOREAL_LIGHT.onSurfaceVariant],
      ['outline', BOREAL_LIGHT.outline],
      ['brand', BOREAL_LIGHT.brand],
    ];

    for (const [inkName, ink] of inks) {
      for (const [tierName, tier] of LIGHT_LADDER) {
        const ratio = contrast(ink, tier);
        expect(ratio, `${inkName} on ${tierName} measured ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(
          AA_TEXT,
        );
      }
    }
  });
});

describe('the brand ink is a usable green in both schemes', () => {
  it('is not `primary` in light, where `primary` reads as black', () => {
    expect(BOREAL_LIGHT.brand).not.toBe(BOREAL_LIGHT.primary);
    expect(BOREAL_LIGHT.brand).toBe('#255f4d');
    // Its green channel dominates by a real margin — a colour, not an ink dot.
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(BOREAL_LIGHT.brand.slice(i, i + 2), 16));
    expect(g).toBeGreaterThan(r + 40);
    expect(g).toBeGreaterThan(b + 10);
  });

  it('is `primary` in dark, which was already legible there', () => {
    expect(BOREAL_DARK.brand).toBe(BOREAL_DARK.primary);
    for (const tier of [
      BOREAL_DARK.surface,
      BOREAL_DARK.surfaceContainerLow,
      BOREAL_DARK.surfaceContainer,
      BOREAL_DARK.surfaceContainerHigh,
    ]) {
      expect(contrast(BOREAL_DARK.brand, tier)).toBeGreaterThanOrEqual(AA_TEXT);
    }
  });
});

describe('the three token mirrors agree with DESIGN.md', () => {
  const mirrored: ReadonlyArray<readonly [string, keyof typeof BOREAL_LIGHT]> = [
    ['surface', 'surface'],
    ['surface-container-lowest', 'surfaceContainerLowest'],
    ['surface-container-low', 'surfaceContainerLow'],
    ['surface-container', 'surfaceContainer'],
    ['surface-container-high', 'surfaceContainerHigh'],
    ['surface-container-highest', 'surfaceContainerHighest'],
    ['outline', 'outline'],
    ['brand', 'brand'],
  ];

  it.each(mirrored)('%s matches across DESIGN.md, web CSS, mobile CSS and shared-constants', (cssName, tsName) => {
    const source = BOREAL_LIGHT[tsName];
    expect(designMdHex(cssName)).toBe(source);
    expect(cssToken(WEB_CSS, cssName, 1)).toBe(source);
    expect(cssToken(MOBILE_CSS, cssName, 1)).toBe(source);
  });

  it('mirrors the dark brand ink too', () => {
    expect(cssToken(WEB_CSS, 'brand', 2)).toBe(BOREAL_DARK.brand);
    expect(cssToken(MOBILE_CSS, 'brand', 2)).toBe(BOREAL_DARK.brand);
  });

  it('gives light the darker ghost-border ink and dark the pale one', () => {
    // A hairline has to contrast with what it sits on, and the two grounds are
    // opposite. Mobile shipped the dark ink in both schemes.
    expect(cssToken(MOBILE_CSS, 'border', 1)).toBe('#9ba59f');
    expect(cssToken(MOBILE_CSS, 'border', 2)).toBe('#c0c8c3');
    expect(DESIGN_MD).toContain('rgba(155, 165, 159, 0.40)');
  });
});

describe('DESIGN.md §5 states one shell rule per client', () => {
  const regionTable = DESIGN_MD.slice(
    DESIGN_MD.indexOf('### Chat surfaces — the messenger layout'),
    DESIGN_MD.indexOf('### Focus rings'),
  );

  it('keeps the web rail name-free', () => {
    const rail = regionTable.split('\n').find((line) => line.includes('Icon rail (72px)'));
    expect(rail, 'the icon rail row').toBeDefined();
    expect(rail).toContain('web only');
    expect(rail).toContain('No name or role text');
  });

  it('gives the mobile chat tab the mark and the wordmark', () => {
    const mobile = regionTable.split('\n').find((line) => line.includes('Chat-tab header'));
    expect(mobile, 'the mobile shell row').toBeDefined();
    expect(mobile).toContain('mobile only');
    expect(mobile).toMatch(/mark \*\*and\*\* the DRAVR wordmark/);
    expect(mobile).toContain('in place of the screen title');
    expect(mobile).toContain('The other tabs keep their own titles');
  });

  it('says why the two shells differ rather than leaving a contradiction', () => {
    expect(regionTable).toContain('Mark only on web, mark plus name on the phone');
    expect(regionTable).toContain('72px column');
    expect(regionTable).toContain('The phone has no rail');
  });
});
