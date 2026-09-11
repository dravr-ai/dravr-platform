// ABOUTME: Pins the phone's type ladder, faces, radii, hairlines and scrim (Boreal v2.2 Phase 2)
// ABOUTME: The hairline strengths are read from the web stylesheet so the two clients cannot drift apart

import { readFileSync } from 'fs';
import { join } from 'path';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';
import { BORDER_INK } from '../src/contexts/ThemeContext';

// eslint-disable-next-line @typescript-eslint/no-require-imports
const tailwind = require('../tailwind.config.js') as {
  theme: {
    extend: {
      fontSize: Record<string, [string, { lineHeight: string }]>;
      fontFamily: Record<string, string[]>;
      borderRadius: Record<string, string>;
      boxShadow: Record<string, string>;
      colors: Record<string, unknown>;
    };
  };
};

const mobileCss = readFileSync(join(__dirname, '..', 'global.css'), 'utf8');
const webCss = readFileSync(join(__dirname, '..', '..', 'frontend', 'src', 'index.css'), 'utf8');

/** `--name: value;` inside the block that starts at `selector {`. */
function cssVar(css: string, selector: string, name: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start).toBeGreaterThanOrEqual(0);
  const block = css.slice(start, css.indexOf('\n}', start));
  const match = block.match(new RegExp(`${name}:\\s*([^;]+);`));
  expect(match).not.toBeNull();
  return (match as RegExpMatchArray)[1].trim();
}

const px = (value: string) => Number.parseInt(value, 10);

describe('the platform ladder in the system face (D4)', () => {
  const ladder = tailwind.theme.extend.fontSize;

  it.each([
    ['xs', 12, 16],
    ['sm', 13, 18],
    ['base', 16, 22],
    ['lg', 17, 22],
    ['xl', 20, 25],
    ['2xl', 22, 28],
    ['3xl', 26, 32],
  ])('%s is %i / %i', (step, size, lineHeight) => {
    const [fontSize, options] = ladder[step];
    expect(px(fontSize)).toBe(size);
    expect(px(options.lineHeight)).toBe(lineHeight);
  });

  it('has a 12 px floor and stops at the auth headline', () => {
    const sizes = Object.values(ladder).map(([size]) => px(size));
    expect(Math.min(...sizes)).toBe(12);
    expect(Math.max(...sizes)).toBe(26);
    expect(Object.keys(ladder)).toEqual(['xs', 'sm', 'base', 'lg', 'xl', '2xl', '3xl']);
  });

  it('loads two faces beside the system one: the display face and the mono', () => {
    expect(tailwind.theme.extend.fontFamily).toEqual({
      display: ['SchibstedGrotesk'],
      mono: ['JetBrainsMono'],
    });
  });
});

describe('the radius ladder', () => {
  it('is 4 chips · 8 buttons · 12 cards · 20 sheets · full', () => {
    const radii = tailwind.theme.extend.borderRadius;
    expect(px(radii.DEFAULT)).toBe(4);
    expect(px(radii.lg)).toBe(8);
    expect(px(radii.xl)).toBe(12);
    // Nothing sits between a card and a sheet.
    expect(radii['2xl']).toBe(radii.xl);
    expect(px(radii['3xl'])).toBe(20);
    expect(px(radii.full)).toBeGreaterThan(1000);
  });

  it('keeps one shadow, for what floats', () => {
    expect(Object.keys(tailwind.theme.extend.boxShadow)).toEqual(['floating']);
  });
});

describe('hairlines agree with the web (E2)', () => {
  const strengths = [
    ['--ghost-border-faint', 'faint'],
    ['--ghost-border', 'default'],
    ['--ghost-border-strong', 'strong'],
  ] as const;

  it.each(strengths)('%s matches frontend/src/index.css in light', (name) => {
    expect(cssVar(mobileCss, ':root', name)).toBe(cssVar(webCss, ':root', name));
  });

  it.each(strengths)('%s matches frontend/src/index.css in dark', (name) => {
    expect(cssVar(mobileCss, ':root.dark, .dark', name)).toBe(cssVar(webCss, 'html.dark', name));
  });

  it.each(strengths)('%s reaches inline styles at the same alpha (%s)', (name, key) => {
    for (const [scheme, selector] of [
      ['light', ':root'],
      ['dark', ':root.dark, .dark'],
    ] as const) {
      const ink = BORDER_INK[scheme];
      expect(cssVar(mobileCss, selector, name)).toBe(`rgba(${ink.rgb}, ${ink[key].toFixed(2)})`);
    }
  });

  it('the class path reads the variables, not a fixed alpha', () => {
    expect(tailwind.theme.extend.colors.border).toEqual({
      faint: 'var(--ghost-border-faint)',
      DEFAULT: 'var(--ghost-border)',
      strong: 'var(--ghost-border-strong)',
    });
  });
});

describe('one scrim', () => {
  it('is the web value in both schemes, on the class path and the token tree', () => {
    expect(cssVar(mobileCss, ':root', '--color-scrim')).toBe(cssVar(webCss, ':root', '--color-scrim'));
    expect(cssVar(mobileCss, ':root.dark, .dark', '--color-scrim')).toBe(cssVar(webCss, 'html.dark', '--color-scrim'));
    expect(BOREAL_LIGHT.scrim).toBe('#1a1c1b');
    expect(BOREAL_DARK.scrim).toBe('#000000');
    expect(tailwind.theme.extend.colors.scrim).toBe('rgb(var(--color-scrim) / <alpha-value>)');
  });
});

describe('what the token map no longer carries', () => {
  it('has no legacy namespace, no frozen scale and no component layer', () => {
    const colors = tailwind.theme.extend.colors;
    expect(colors).not.toHaveProperty('pierre');
    expect(colors).not.toHaveProperty('primary_scale');
    expect(mobileCss).not.toMatch(/@layer components/);
    expect(mobileCss).not.toMatch(/--color-gray-/);
  });
});
