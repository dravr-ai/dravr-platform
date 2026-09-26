// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what the client stylesheet generator emits for web and mobile: both schemes, from the tokens, above every floor
// ABOUTME: The two stylesheets used to hand-copy the token tree; this proves the generated blocks carry the source values

import { describe, expect, it } from 'vitest';
import {
  clientPalette,
  measureClientPairings,
  renderClientCss,
  sharedPalettes,
  type Client,
} from '../scripts/generate-client-css';
import {
  BOREAL_DARK,
  BOREAL_LIGHT,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  PRIMARY_HOVER,
} from '../src/design-system';

/** `#rrggbb` as the bare triple a `--color-*` variable holds. */
function triple(hex: string): string {
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(' ');
}

/** The declarations of the block that opens with `selector {`, as a name → value map. */
function block(css: string, selector: string): Map<string, string> {
  const start = css.indexOf(`${selector} {\n`);
  expect(start, `a block opening with ${selector}`).toBeGreaterThan(-1);
  const body = css.slice(start, css.indexOf('}', start));
  const entries = [...body.matchAll(/^\s*(--[a-z0-9-]+): ([^;]+);$/gm)].map((m) => [m[1], m[2]] as const);
  return new Map(entries);
}

const SELECTORS: Record<Client, { light: string; dark: string }> = {
  web: { light: ':root', dark: 'html.dark' },
  mobile: { light: '  :root', dark: '  :root.dark, .dark' },
};

describe('client stylesheet blocks', () => {
  const web = renderClientCss('web');
  const mobile = renderClientCss('mobile');

  it('scopes light under :root and dark under each client’s dark class', () => {
    expect(web.startsWith('/* ABOUTME: ')).toBe(true);
    expect(web).toContain('\n:root {\n');
    expect(web).toContain('\nhtml.dark {\n');
    expect(web).not.toContain('@layer');
    expect(mobile).toContain('\n@layer base {\n  :root {\n');
    expect(mobile).toContain('\n  :root.dark, .dark {\n');
  });

  it('carries the token values in each scheme, the dark outline raised for AA included', () => {
    for (const [client, css] of [
      ['web', web],
      ['mobile', mobile],
    ] as const) {
      const light = block(css, SELECTORS[client].light);
      const dark = block(css, SELECTORS[client].dark);
      expect(light.get('--color-primary')).toBe('37 95 77');
      expect(light.get('--color-surface-container-lowest')).toBe(triple(BOREAL_LIGHT.surfaceContainerLowest));
      expect(light.get('--color-outline')).toBe('82 90 85');
      expect(dark.get('--color-outline')).toBe('176 182 175');
      expect(dark.get('--color-surface-container-lowest')).toBe('11 14 11');
      expect(dark.get('--color-primary')).toBe(triple(BOREAL_DARK.primary));
      expect(light.get('--color-on-warning-container')).toBe(triple(CONTAINER_INKS.warning));
      expect(dark.get('--color-on-mobility-container')).toBe(triple(CONTAINER_INKS_DARK.mobility));
      expect(light.get('--color-mobility')).toBe('155 70 102');
      expect(dark.get('--color-success')).toBe('121 166 148');
      expect(light.get('--ghost-border')).toBe('rgba(155, 165, 159, 0.40)');
      expect(light.get('--ghost-border-faint')).toBe('rgba(155, 165, 159, 0.26)');
      expect(dark.get('--ghost-border-strong')).toBe('rgba(192, 200, 195, 0.34)');
      expect(dark.get('--color-scrim')).toBe('0 0 0');
    }
  });

  it('declares what each client’s Tailwind config reads: 47 properties per scheme', () => {
    const counts = (css: string, client: Client) => [
      block(css, SELECTORS[client].light).size,
      block(css, SELECTORS[client].dark).size,
    ];
    expect(counts(web, 'web')).toEqual([47, 47]);
    expect(counts(mobile, 'mobile')).toEqual([47, 47]);

    const webLight = block(web, ':root');
    expect(webLight.get('--color-primary-hover')).toBe(triple(PRIMARY_HOVER.light));
    expect(block(web, 'html.dark').get('--color-primary-hover')).toBe('140 186 168');
    expect(webLight.get('--shadow-floating')).toBe(
      '0 12px 24px -6px rgba(26, 28, 27, 0.12), 0 6px 12px -3px rgba(26, 28, 27, 0.08)',
    );
    expect(webLight.has('--color-primary-fixed')).toBe(false);

    const mobileLight = block(mobile, '  :root');
    expect(mobileLight.get('--color-primary-fixed')).toBe('190 237 217');
    expect(mobileLight.get('--color-tertiary-fixed-dim')).toBe('173 205 195');
    expect(mobileLight.has('--color-primary-hover')).toBe(false);
    expect(mobileLight.has('--shadow-floating')).toBe(false);
  });

  it('measures every pairing the clients draw above 4.5:1, in both schemes', () => {
    const palettes = sharedPalettes();
    const light = measureClientPairings('light', palettes.light);
    const dark = measureClientPairings('dark', palettes.dark);
    // 4 text roles × 6 tiers, 7 bound inks × 6 tiers, 5 filled pairs.
    expect(light).toHaveLength(71);
    expect(dark).toHaveLength(71);
    for (const row of [...light, ...dark]) {
      expect(row.ratio, `${row.scheme} ${row.what}`).toBeGreaterThanOrEqual(4.5);
    }
    const ratio = (rows: typeof light, what: string) => rows.find((r) => r.what === what)?.ratio ?? 0;
    expect(ratio(light, 'outline on surface-container-highest')).toBeCloseTo(4.51, 2);
    expect(ratio(dark, 'outline on surface-container-highest')).toBeCloseTo(6.01, 2);
    expect(ratio(light, 'on-warning-container on warning/15 over surface-container-highest')).toBeCloseTo(4.58, 2);
    expect(web).toContain('142 pairings measured');
    expect(web).toContain('closest to its floor: 4.51:1 (light outline on surface-container-highest)');
  });

  it('refuses to write a dark outline below AA — the value the website still carries', () => {
    const palettes = sharedPalettes();
    const drifted = {
      ...palettes,
      dark: { ...palettes.dark, tokens: { ...palettes.dark.tokens, outline: '#8a9389' } },
    };
    expect(() => renderClientCss('web', drifted)).toThrow(/dark outline on surface-container-highest: 3\.91:1 < 4\.5:1/);
  });

  it('refuses a bound ink that no longer clears its tint', () => {
    const light = clientPalette('light');
    const drifted = {
      light: { ...light, inks: { ...light.inks, nutrition: light.hues.nutrition } },
      dark: clientPalette('dark'),
    };
    expect(() => renderClientCss('mobile', drifted)).toThrow(/light on-nutrition-container on nutrition\/15 over surface/);
  });
});
