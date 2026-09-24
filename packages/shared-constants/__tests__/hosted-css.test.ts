// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what the hosted-page stylesheet generator emits: both schemes, from the tokens, above every floor
// ABOUTME: The server-rendered pages cannot run a token test of their own, so the sheet they embed is measured here

import { describe, expect, it } from 'vitest';
import {
  generateHostedCss,
  hostedPalette,
  measurePairings,
  renderHostedCss,
} from '../scripts/generate-hosted-css';
import { BOREAL_DARK, BOREAL_LIGHT, CONTAINER_INKS, CONTAINER_INKS_DARK } from '../src/design-system';

/** `#rrggbb` as the bare triple the sheet's `--color-*` variables hold. */
function triple(hex: string): string {
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(' ');
}

/** The body of the dark-scheme block, so a light value cannot satisfy a dark assertion. */
function darkBlock(css: string): string {
  const start = css.indexOf('@media (prefers-color-scheme: dark) {');
  const end = css.indexOf('\n}\n', start);
  expect(start).toBeGreaterThan(-1);
  return css.slice(start, end);
}

describe('hosted page stylesheet', () => {
  const css = generateHostedCss();

  it('declares both schemes, light by default and dark under the media query', () => {
    expect(css).toContain('  color-scheme: light dark;');
    const light = css.slice(0, css.indexOf('@media (prefers-color-scheme: dark)'));
    expect(light).toContain(`--color-primary: ${triple(BOREAL_LIGHT.primary)};`);
    expect(light).toContain(`--color-surface: ${triple(BOREAL_LIGHT.surface)};`);
    expect(light).toContain(`--color-card: ${triple(BOREAL_LIGHT.surfaceContainerLowest)};`);
    const dark = darkBlock(css);
    expect(dark).toContain(`--color-primary: ${triple(BOREAL_DARK.primary)};`);
    expect(dark).toContain(`--color-surface: ${triple(BOREAL_DARK.surface)};`);
    expect(dark).toContain(`--color-card: ${triple(BOREAL_DARK.surfaceContainerHigh)};`);
  });

  it('binds the notice to the warning tint and its own ink in each scheme', () => {
    expect(css).toContain(`--color-on-warning-container: ${triple(CONTAINER_INKS.warning)};`);
    expect(darkBlock(css)).toContain(`--color-on-warning-container: ${triple(CONTAINER_INKS_DARK.warning)};`);
    expect(css).toContain('  background: rgb(var(--color-warning) / 0.1);');
    expect(css).toContain('  color: rgb(var(--color-on-warning-container));');
  });

  it('draws the checkbox edge in the outline token and the checked fill in primary', () => {
    const rule = css.slice(css.indexOf('.check input[type="checkbox"] {'));
    expect(rule).toContain('  border: 1px solid rgb(var(--color-outline));');
    expect(css).toContain('  background-image: var(--check-glyph);');
  });

  it('sets the wordmark from the shared constant and loads only Google Fonts', () => {
    expect(css).toContain(".lockup::after {\n  content: 'DRAVR';\n}");
    const imports = css.match(/@import url\('([^']+)'\);/g) ?? [];
    expect(imports).toHaveLength(1);
    expect(imports[0]).toContain('https://fonts.googleapis.com/css2?family=Schibsted+Grotesk');
    expect(imports[0]).toContain('family=Plus+Jakarta+Sans');
  });

  it('carries no template brace pair and none of the retired Pierre palette', () => {
    expect(css).not.toContain('{{');
    expect(css).not.toContain('}}');
    expect(css.toLowerCase()).not.toContain('#7c3aed');
    expect(css).not.toContain('--pierre-');
    expect(css).not.toContain('linear-gradient');
  });

  it('measures every pairing it draws above its WCAG floor, in both schemes', () => {
    const light = measurePairings('light');
    const dark = measurePairings('dark');
    // 18 fixed pairings plus one glyph per PROVIDER_GLYPH_INK row (10).
    expect(light).toHaveLength(28);
    expect(dark).toHaveLength(28);
    for (const row of [...light, ...dark]) {
      expect(row.ratio, `${row.scheme} ${row.what}`).toBeGreaterThanOrEqual(row.floor);
    }
    const notice = (rows: typeof light) =>
      rows.find((r) => r.what.startsWith('notice ink'))?.ratio ?? 0;
    expect(notice(light)).toBeCloseTo(7.26, 2);
    expect(notice(dark)).toBeCloseTo(6.35, 2);
    const edge = (rows: typeof light) =>
      rows.find((r) => r.what.startsWith('checkbox edge'))?.ratio ?? 0;
    expect(edge(light)).toBeCloseTo(6.42, 2);
    expect(edge(dark)).toBeCloseTo(5.64, 2);
  });

  it('writes the measured table into the sheet header', () => {
    expect(css).toMatch(/light {2}notice ink \(title, body, checkbox label\) on the notice\.+ +7\.26:1/);
    expect(css).toMatch(/dark {3}checkbox edge \(outline\) on the notice\.+ +5\.64:1/);
  });

  it('lifts the card from a different tier per scheme', () => {
    expect(hostedPalette('light').card).toBe(BOREAL_LIGHT.surfaceContainerLowest);
    expect(hostedPalette('dark').card).toBe(BOREAL_DARK.surfaceContainerHigh);
  });

  it('refuses to render a lockup with no mark', () => {
    expect(() => renderHostedCss(new Uint8Array())).toThrow('the mark asset is empty');
  });
});
