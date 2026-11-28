// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Generates boreal-tokens.json, the data export of design-system.ts a client with no JS runtime reads (the Swift app)
// ABOUTME: Every value comes from the shared tokens; it refuses to write below the WCAG floors the client stylesheets are held to

import { writeFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  AMBIENT_SHADOW,
  BORDER_INK,
  BORDER_RADIUS,
  BRAND_TRACKING,
  BUTTON_GLOW,
  FLOATING_SHADOW,
  FONT_WEIGHT,
  GRADIENT_COLORS,
  MARK_INK,
  PRIMARY_PALETTE,
  PROVIDER_COLORS,
  PROVIDER_GLYPH_INK,
  SPACING,
  TYPOGRAPHY,
  type ColorScheme,
  type HairlineStrength,
} from '../src/design-system';
import { pairingsAboveFloor, sharedPalettes, type ClientPalette } from './generate-client-css';

const PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/**
 * Where the export is committed: beside the package manifest, so the path holds in every checkout
 * of this package. A consumer pins a commit and vendors `packages/shared-constants/boreal-tokens.json`.
 */
export const NATIVE_TOKENS_PATH = resolve(PACKAGE_ROOT, 'boreal-tokens.json');

/** One colour as data: an opaque `#rrggbb` and the alpha it is drawn at. */
export interface TokenColor {
  hex: string;
  alpha: number;
}

/** A colour that has a value in each scheme. */
export type SchemeColor = Record<ColorScheme, TokenColor>;

/** One layer of a CSS box shadow, in pixels. */
export interface ShadowLayer {
  x: number;
  y: number;
  blur: number;
  spread: number;
  color: TokenColor;
}

/** The single shadow a native view takes. */
export interface NativeShadow {
  color: TokenColor;
  opacity: number;
  radius: number;
  offset?: { width: number; height: number };
}

/**
 * The scheme-paired colours, grouped as DESIGN.md names them. A key is unique
 * across the groups, so a consumer may flatten them into one namespace: the
 * names are the camelCase of the `--color-*` and `--ghost-border*` custom
 * properties the client stylesheets declare.
 */
export interface NativeColorGroups {
  /** The MD3 token tree, every key of `BOREAL_LIGHT` / `BOREAL_DARK`. */
  scheme: Record<string, SchemeColor>;
  /** `PRIMARY_HOVER` and `MARK_INK`. */
  brand: Record<string, SchemeColor>;
  /** `PILLARS`. */
  pillars: Record<string, SchemeColor>;
  /** `SEMANTIC_COLORS` / `SEMANTIC_COLORS_DARK`; `error` is the MD3 `error` in `scheme`. */
  feedback: Record<string, SchemeColor>;
  /** `CONTAINER_INKS` / `CONTAINER_INKS_DARK`, as `on<Hue>Container`. */
  boundInks: Record<string, SchemeColor>;
  /** `BORDER_INK`: the hairline ink of each scheme at each strength's alpha. */
  hairlines: Record<string, SchemeColor>;
}

/** The whole export. */
export interface NativeTokens {
  source: string;
  generator: string;
  colors: NativeColorGroups;
  primaryPalette: Record<string, TokenColor>;
  providerColors: Record<string, TokenColor>;
  /** `null` is the body ink (`onSurface`) in that scheme. */
  providerGlyphInk: Record<string, Record<ColorScheme, TokenColor | null>>;
  gradients: Record<string, { start: TokenColor; end: TokenColor; angle?: number }>;
  shadows: {
    floating: Record<ColorScheme, ShadowLayer[]>;
    ambient: Record<string, NativeShadow>;
    buttonGlow: NativeShadow;
  };
  typography: {
    families: Record<string, string>;
    fontWeight: Record<string, number>;
    /** Letter-spacing of the wordmark, in em. */
    brandTrackingEm: number;
  };
  spacing: Record<string, number>;
  radius: Record<string, number>;
}

const HEX = /^#[0-9a-f]{6}$/i;
const RGBA = /^rgba\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(0|1|0?\.\d+)\s*\)$/;
const SHADOW_LAYER = /^(-?\d+)(?:px)?\s+(-?\d+)(?:px)?\s+(-?\d+)(?:px)?\s+(-?\d+)(?:px)?\s+(rgba\([^)]*\))$/;

function channelsToHex(channels: readonly number[]): string {
  if (channels.length !== 3 || channels.some((c) => !Number.isInteger(c) || c < 0 || c > 255)) {
    throw new Error(`not three 8-bit channels: ${channels.join(', ')}`);
  }
  return `#${channels.map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}

/** A `#rrggbb` or `rgba(r, g, b, a)` token as data. Anything else is a token this export cannot carry. */
export function tokenColor(value: string): TokenColor {
  if (HEX.test(value)) {
    return { hex: value.toLowerCase(), alpha: 1 };
  }
  const rgba = RGBA.exec(value);
  if (rgba) {
    return { hex: channelsToHex([rgba[1], rgba[2], rgba[3]].map(Number)), alpha: Number(rgba[4]) };
  }
  throw new Error(`not a #rrggbb or rgba() colour: ${value}`);
}

/** `0 12px 24px -6px rgba(…), 0 6px 12px -3px rgba(…)` as its layers. */
export function shadowLayers(css: string): ShadowLayer[] {
  // A comma separates layers and also sits inside rgba(); split on the one that follows a `)`.
  return css.split(/(?<=\))\s*,\s*/).map((layer) => {
    const match = SHADOW_LAYER.exec(layer.trim());
    if (!match) {
      throw new Error(`not a box-shadow layer this export can carry: ${layer}`);
    }
    const [x, y, blur, spread] = match.slice(1, 5).map(Number);
    return { x, y, blur, spread, color: tokenColor(match[5]) };
  });
}

function capitalise(word: string): string {
  return word.charAt(0).toUpperCase() + word.slice(1);
}

function mapValues<V, R>(record: Readonly<Record<string, V>>, map: (value: V, key: string) => R): Record<string, R> {
  return Object.fromEntries(Object.entries(record).map(([key, value]) => [key, map(value, key)]));
}

/** One name's value in each scheme. */
function paired(pick: (scheme: ColorScheme) => TokenColor): SchemeColor {
  return { light: pick('light'), dark: pick('dark') };
}

const HAIRLINE_NAMES: Record<HairlineStrength, string> = {
  faint: 'ghostBorderFaint',
  default: 'ghostBorder',
  strong: 'ghostBorderStrong',
};

function colorGroups(palettes: Record<ColorScheme, ClientPalette>): NativeColorGroups {
  const tokenKeys = Object.keys(palettes.light.tokens) as Array<keyof ClientPalette['tokens']>;
  const hueKeys = Object.keys(palettes.light.hues) as Array<keyof ClientPalette['hues']>;
  const pillarKeys = ['activity', 'nutrition', 'recovery', 'mobility'] as const;
  const feedbackKeys = hueKeys.filter((hue) => !pillarKeys.some((pillar) => pillar === hue));
  const hairlineInk = (scheme: ColorScheme) => channelsToHex(BORDER_INK[scheme].rgb.split(',').map(Number));

  const groups: NativeColorGroups = {
    scheme: Object.fromEntries(
      tokenKeys.map((key) => [key, paired((scheme) => tokenColor(palettes[scheme].tokens[key]))]),
    ),
    brand: {
      primaryHover: paired((scheme) => tokenColor(palettes[scheme].primaryHover)),
      markInk: paired((scheme) => tokenColor(MARK_INK[scheme])),
    },
    pillars: Object.fromEntries(
      pillarKeys.map((key) => [key, paired((scheme) => tokenColor(palettes[scheme].hues[key]))]),
    ),
    feedback: Object.fromEntries(
      feedbackKeys.map((key) => [key, paired((scheme) => tokenColor(palettes[scheme].hues[key]))]),
    ),
    boundInks: Object.fromEntries(
      hueKeys.map((key) => [
        `on${capitalise(key)}Container`,
        paired((scheme) => tokenColor(palettes[scheme].inks[key])),
      ]),
    ),
    hairlines: Object.fromEntries(
      (Object.keys(HAIRLINE_NAMES) as HairlineStrength[]).map((strength) => [
        HAIRLINE_NAMES[strength],
        paired((scheme) => ({ hex: hairlineInk(scheme), alpha: BORDER_INK[scheme][strength] })),
      ]),
    ),
  };

  // The groups share one namespace on the consumer's side; a name declared twice would let one
  // colour silently replace another there.
  const seen = new Map<string, string>();
  for (const [group, colors] of Object.entries(groups)) {
    for (const name of Object.keys(colors)) {
      const earlier = seen.get(name);
      if (earlier !== undefined) {
        throw new Error(`colour name ${name} is declared by both ${earlier} and ${group}`);
      }
      seen.set(name, group);
    }
  }
  return groups;
}

function nativeShadow(shadow: {
  shadowColor: string;
  shadowOpacity: number;
  shadowRadius: number;
  shadowOffset?: { width: number; height: number };
}): NativeShadow {
  return {
    color: tokenColor(shadow.shadowColor),
    opacity: shadow.shadowOpacity,
    radius: shadow.shadowRadius,
    ...(shadow.shadowOffset ? { offset: { ...shadow.shadowOffset } } : {}),
  };
}

function brandTrackingEm(): number {
  const match = /^(\d*\.?\d+)em$/.exec(BRAND_TRACKING);
  if (!match) {
    throw new Error(`BRAND_TRACKING is not an em length: ${BRAND_TRACKING}`);
  }
  return Number(match[1]);
}

/** The export, from the palettes. Throws below a contrast floor, like the client stylesheets. */
export function nativeTokens(palettes: Record<ColorScheme, ClientPalette> = sharedPalettes()): NativeTokens {
  pairingsAboveFloor(palettes);
  return {
    source: 'packages/shared-constants/src/design-system.ts',
    generator: 'packages/shared-constants/scripts/generate-native-tokens.ts',
    colors: colorGroups(palettes),
    primaryPalette: mapValues(PRIMARY_PALETTE, tokenColor),
    providerColors: mapValues(PROVIDER_COLORS, tokenColor),
    providerGlyphInk: mapValues(PROVIDER_GLYPH_INK, (ink) => ({
      light: ink.light === null ? null : tokenColor(ink.light),
      dark: ink.dark === null ? null : tokenColor(ink.dark),
    })),
    gradients: mapValues(
      GRADIENT_COLORS as Readonly<Record<string, { start: string; end: string; angle?: number }>>,
      (gradient) => ({
        start: tokenColor(gradient.start),
        end: tokenColor(gradient.end),
        ...(gradient.angle === undefined ? {} : { angle: gradient.angle }),
      }),
    ),
    shadows: {
      floating: {
        light: shadowLayers(FLOATING_SHADOW.light),
        dark: shadowLayers(FLOATING_SHADOW.dark),
      },
      ambient: mapValues(AMBIENT_SHADOW, nativeShadow),
      buttonGlow: nativeShadow(BUTTON_GLOW),
    },
    typography: {
      families: { ...TYPOGRAPHY },
      fontWeight: mapValues(FONT_WEIGHT, Number),
      brandTrackingEm: brandTrackingEm(),
    },
    spacing: { ...SPACING },
    radius: { ...BORDER_RADIUS },
  };
}

/** The committed file's bytes: two-space JSON in the tokens' own order, one trailing newline. */
export function renderNativeTokens(palettes: Record<ColorScheme, ClientPalette> = sharedPalettes()): string {
  return `${JSON.stringify(nativeTokens(palettes), null, 2)}\n`;
}

/** `--out <path>` writes somewhere else — the staleness check renders into a temp file. */
function outputPath(argv: readonly string[]): string {
  const at = argv.indexOf('--out');
  if (at === -1) {
    return NATIVE_TOKENS_PATH;
  }
  const target = argv[at + 1];
  if (!target) {
    throw new Error('--out needs a path');
  }
  return resolve(process.cwd(), target);
}

if (import.meta.main) {
  const out = outputPath(process.argv.slice(2));
  const tokens = nativeTokens();
  const json = renderNativeTokens();
  writeFileSync(out, json, 'utf8');
  const groups: Array<Record<string, SchemeColor>> = Object.values(tokens.colors);
  const paired = groups.reduce((count, group) => count + Object.keys(group).length, 0);
  console.log(
    `native tokens: ${json.length} bytes, ${paired} light/dark colours ` +
      `-> ${relative(process.cwd(), out) || out}`,
  );
}
