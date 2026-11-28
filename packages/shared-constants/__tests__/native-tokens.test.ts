// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what the native token export carries: every scheme-paired colour, the hairline alphas, the scales, from the tokens
// ABOUTME: The Swift app generates its asset catalog from this file, so a value missing here is a colour it cannot draw

import { describe, expect, it } from 'vitest';
import { sharedPalettes } from '../scripts/generate-client-css';
import {
  nativeTokens,
  renderNativeTokens,
  shadowLayers,
  tokenColor,
  type SchemeColor,
} from '../scripts/generate-native-tokens';
import {
  BOREAL_DARK,
  BOREAL_LIGHT,
  BORDER_RADIUS,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  MARK_INK,
  PILLAR_COLORS,
  PILLAR_COLORS_DARK,
  PRIMARY_HOVER,
  PROVIDER_GLYPH_INK,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  SPACING,
} from '../src/design-system';

const opaque = (hex: string) => ({ hex: hex.toLowerCase(), alpha: 1 });

describe('native token export', () => {
  const tokens = nativeTokens();

  it('carries every key of the MD3 tree in both schemes', () => {
    const keys = Object.keys(BOREAL_LIGHT) as Array<keyof typeof BOREAL_LIGHT>;
    expect(Object.keys(tokens.colors.scheme)).toEqual(keys);
    expect(keys).toHaveLength(41);
    for (const key of keys) {
      expect(tokens.colors.scheme[key], key).toEqual({
        light: opaque(BOREAL_LIGHT[key]),
        dark: opaque(BOREAL_DARK[key]),
      });
    }
  });

  it('carries the pillars, the feedback hues and each bound ink under its stylesheet name', () => {
    expect(tokens.colors.pillars).toEqual({
      activity: { light: opaque(PILLAR_COLORS.activity), dark: opaque(PILLAR_COLORS_DARK.activity) },
      nutrition: { light: opaque(PILLAR_COLORS.nutrition), dark: opaque(PILLAR_COLORS_DARK.nutrition) },
      recovery: { light: opaque(PILLAR_COLORS.recovery), dark: opaque(PILLAR_COLORS_DARK.recovery) },
      mobility: { light: opaque(PILLAR_COLORS.mobility), dark: opaque(PILLAR_COLORS_DARK.mobility) },
    });
    expect(tokens.colors.feedback).toEqual({
      info: { light: opaque(SEMANTIC_COLORS.info), dark: opaque(SEMANTIC_COLORS_DARK.info) },
      success: { light: opaque(SEMANTIC_COLORS.success), dark: opaque(SEMANTIC_COLORS_DARK.success) },
      warning: { light: opaque(SEMANTIC_COLORS.warning), dark: opaque(SEMANTIC_COLORS_DARK.warning) },
    });
    // The fourth feedback hue is the MD3 error, carried once.
    expect(tokens.colors.scheme.error).toEqual({
      light: opaque(SEMANTIC_COLORS.error),
      dark: opaque(SEMANTIC_COLORS_DARK.error),
    });
    const inks = Object.keys(CONTAINER_INKS) as Array<keyof typeof CONTAINER_INKS>;
    expect(Object.keys(tokens.colors.boundInks)).toHaveLength(inks.length);
    for (const hue of inks) {
      const name = `on${hue.charAt(0).toUpperCase()}${hue.slice(1)}Container`;
      expect(tokens.colors.boundInks[name], name).toEqual({
        light: opaque(CONTAINER_INKS[hue]),
        dark: opaque(CONTAINER_INKS_DARK[hue]),
      });
    }
    expect(tokens.colors.brand).toEqual({
      primaryHover: { light: opaque(PRIMARY_HOVER.light), dark: opaque(PRIMARY_HOVER.dark) },
      markInk: { light: opaque(MARK_INK.light), dark: opaque(MARK_INK.dark) },
    });
  });

  it('carries the hairlines as each scheme’s own ink at each strength’s alpha', () => {
    expect(tokens.colors.hairlines).toEqual({
      ghostBorderFaint: { light: { hex: '#9ba59f', alpha: 0.26 }, dark: { hex: '#c0c8c3', alpha: 0.14 } },
      ghostBorder: { light: { hex: '#9ba59f', alpha: 0.4 }, dark: { hex: '#c0c8c3', alpha: 0.22 } },
      ghostBorderStrong: { light: { hex: '#9ba59f', alpha: 0.55 }, dark: { hex: '#c0c8c3', alpha: 0.34 } },
    });
  });

  it('names each scheme-paired colour once across the groups', () => {
    const groups: Array<Record<string, SchemeColor>> = Object.values(tokens.colors);
    const names = groups.flatMap((group) => Object.keys(group));
    expect(names).toHaveLength(60);
    expect(new Set(names).size).toBe(names.length);
    for (const group of groups) {
      for (const [name, color] of Object.entries(group)) {
        for (const scheme of ['light', 'dark'] as const) {
          expect(color[scheme].hex, `${name} ${scheme}`).toMatch(/^#[0-9a-f]{6}$/);
          expect(color[scheme].alpha, `${name} ${scheme}`).toBeGreaterThan(0);
          expect(color[scheme].alpha, `${name} ${scheme}`).toBeLessThanOrEqual(1);
        }
      }
    }
  });

  it('carries the scales and the type roles as numbers a native client can use', () => {
    expect(tokens.spacing).toEqual(SPACING);
    expect(tokens.radius).toEqual(BORDER_RADIUS);
    expect(tokens.typography.fontWeight).toEqual({ normal: 400, medium: 500, semibold: 600, bold: 700 });
    expect(tokens.typography.brandTrackingEm).toBe(0.15);
    expect(tokens.typography.families.mono).toBe('JetBrains Mono');
    expect(tokens.typography.families.display).toBe('Schibsted Grotesk');
  });

  it('carries the provider glyph inks, null where a brand yields to the body ink', () => {
    expect(Object.keys(tokens.providerGlyphInk)).toEqual(Object.keys(PROVIDER_GLYPH_INK));
    expect(tokens.providerGlyphInk.whoop).toEqual({ light: null, dark: { hex: '#00d46a', alpha: 1 } });
    expect(tokens.providerGlyphInk.trainingpeaks).toEqual({ light: { hex: '#005695', alpha: 1 }, dark: null });
    expect(tokens.providerColors.strava).toEqual({ hex: '#fc4c02', alpha: 1 });
    expect(Object.keys(tokens.primaryPalette)).toHaveLength(11);
  });

  it('carries the shadows and gradients as layers and stops, alpha kept', () => {
    expect(tokens.shadows.floating.light).toEqual([
      { x: 0, y: 12, blur: 24, spread: -6, color: { hex: '#1a1c1b', alpha: 0.12 } },
      { x: 0, y: 6, blur: 12, spread: -3, color: { hex: '#1a1c1b', alpha: 0.08 } },
    ]);
    expect(tokens.shadows.floating.dark.map((layer) => layer.color.alpha)).toEqual([0.55, 0.45]);
    expect(tokens.shadows.ambient.floating).toEqual({
      color: opaque(BOREAL_LIGHT.onSurface),
      opacity: 0.18,
      radius: 24,
      offset: { width: 0, height: 12 },
    });
    expect(tokens.shadows.buttonGlow).toEqual({ color: opaque(BOREAL_LIGHT.onSurface), opacity: 0.06, radius: 24 });
    expect(tokens.gradients.borealHero).toEqual({
      start: opaque(BOREAL_LIGHT.primary),
      end: opaque(BOREAL_LIGHT.primaryContainer),
      angle: 145,
    });
    expect(tokens.gradients.darkOverlay.end).toEqual({ hex: '#00241a', alpha: 0.85 });
  });

  it('refuses a value it cannot carry as data', () => {
    expect(() => tokenColor('forestgreen')).toThrow(/not a #rrggbb or rgba\(\) colour/);
    expect(() => tokenColor('rgba(300, 0, 0, 0.5)')).toThrow(/not three 8-bit channels/);
    expect(() => shadowLayers('inset 0 0 4px red')).toThrow(/not a box-shadow layer/);
  });

  it('refuses to write below the floors the client stylesheets are held to', () => {
    const palettes = sharedPalettes();
    const drifted = {
      ...palettes,
      dark: { ...palettes.dark, tokens: { ...palettes.dark.tokens, outline: '#8a9389' } },
    };
    expect(() => renderNativeTokens(drifted)).toThrow(/dark outline on surface-container-highest: 3\.91:1 < 4\.5:1/);
  });

  it('renders the same bytes on every run, as JSON that parses back to the export', () => {
    const rendered = renderNativeTokens();
    expect(renderNativeTokens()).toBe(rendered);
    expect(rendered.endsWith('}\n')).toBe(true);
    expect(JSON.parse(rendered)).toEqual(tokens);
  });
});
