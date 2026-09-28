// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Builds the Design System artifact's tokens.json from the Boreal sources and maps each card to its component source
// ABOUTME: Values come only from the sources; token-notes.json carries the usage prose, and a value without a note fails the build

import { readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  BORDER_INK,
  BORDER_RADIUS,
  BOREAL,
  BRAND_TRACKING,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  FLOATING_SHADOW,
  MARK_INK,
  PILLARS,
  PRIMARY_HOVER,
  PRIMARY_PALETTE,
  PROVIDER_COLORS,
  PROVIDER_GLYPH_INK,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  SPACING,
  ghostBorder,
  type ColorScheme,
  type HairlineStrength,
} from '../../../packages/shared-constants/src/design-system';

export const FRONTEND_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const CONTENT_ROOT = resolve(FRONTEND_ROOT, 'design-system');
export const BUNDLE_ENTRY = resolve(CONTENT_ROOT, 'bundle-entry.ts');
/** A card folder that is not a component: the artifact shows it above the brand book. */
export const COVER = 'Cover';

const require = createRequire(import.meta.url);
const SCHEMES: readonly ColorScheme[] = ['light', 'dark'];
/** The web scale is written in rem against the browser's 16px root. */
const ROOT_FONT_PX = 16;
/** Headings carry this tracking (DESIGN.md §3); it lives in index.css, not in the Tailwind theme. */
const HEADING_TRACKING = '-0.01em';

type Scheme = Record<ColorScheme, string>;
export interface ColorToken { name: string; value: Scheme | string; usage: string }
export interface ListToken { name: string; value: string | Scheme; usage: string }
interface TypeStyleNote { name: string; step: string; fontWeight: number; usage: string; tracking?: 'brand' | 'heading'; fontStyle?: string; sample?: string; family?: string }
interface TypeGroupNote { name: string; family: string; scale?: 'mobile'; styles: TypeStyleNote[] }
interface ListFamily { note?: string; tokens: ListToken[] }
export interface TokenNotes {
  colors: Record<string, string>;
  spacing: Record<string, string>;
  radius: Record<string, string>;
  shadow: Record<string, string>;
  layout: ListFamily;
  opacity: ListFamily;
  type: TypeGroupNote[];
}

interface TailwindTheme {
  theme: { extend: {
    fontSize: Record<string, [string, { lineHeight: string }]>;
    fontFamily: Record<string, string[]>;
    spacing: Record<string, string>;
  } };
}
type Step = { fontSize: number; lineHeight: number };

export function readNotes(): TokenNotes {
  return JSON.parse(readFileSync(resolve(CONTENT_ROOT, 'token-notes.json'), 'utf8')) as TokenNotes;
}

function tailwindTheme(): TailwindTheme['theme']['extend'] {
  return (require(resolve(FRONTEND_ROOT, 'tailwind.config.cjs')) as TailwindTheme).theme.extend;
}

function mobileScale(): Record<string, Step> {
  const path = resolve(FRONTEND_ROOT, '../frontend-mobile/src/constants/typeScale.js');
  return (require(path) as { TYPE_SCALE: Record<string, Step> }).TYPE_SCALE;
}

const kebab = (key: string): string => key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`).replace(/_/g, '-');
const lower = (hex: string): string => hex.toLowerCase();
const both = (light: string, dark: string): Scheme => ({ light: lower(light), dark: lower(dark) });

function remToPx(rem: string): string {
  const match = /^([\d.]+)rem$/.exec(rem);
  if (!match) throw new Error(`expected a rem length in the Tailwind scale, got "${rem}"`);
  return `${Number(match[1]) * ROOT_FONT_PX}px`;
}

/** Every colour value, named as the artifact names it, before notes are attached. */
export function colorValues(): Array<{ name: string; value: Scheme | string }> {
  const out: Array<{ name: string; value: Scheme | string }> = [];
  for (const key of Object.keys(BOREAL.light) as Array<keyof typeof BOREAL.light>) {
    out.push({ name: kebab(key), value: both(BOREAL.light[key], BOREAL.dark[key]) });
    if (key === 'onPrimaryContainer') out.push({ name: 'primary-hover', value: both(PRIMARY_HOVER.light, PRIMARY_HOVER.dark) });
  }
  for (const key of Object.keys(PILLARS.light) as Array<keyof typeof PILLARS.light>) {
    out.push({ name: key, value: both(PILLARS.light[key], PILLARS.dark[key]) });
  }
  for (const key of ['success', 'warning', 'info'] as const) {
    out.push({ name: key, value: both(SEMANTIC_COLORS[key], SEMANTIC_COLORS_DARK[key]) });
  }
  for (const key of Object.keys(CONTAINER_INKS) as Array<keyof typeof CONTAINER_INKS>) {
    out.push({ name: `on-${key}-container`, value: both(CONTAINER_INKS[key], CONTAINER_INKS_DARK[key]) });
  }
  const hairlines: Array<[string, HairlineStrength]> = [['ghost-border', 'default'], ['ghost-border-strong', 'strong'], ['ghost-border-faint', 'faint']];
  for (const [name, strength] of hairlines) {
    out.push({ name, value: { light: ghostBorder(BORDER_INK.light, strength), dark: ghostBorder(BORDER_INK.dark, strength) } });
  }
  out.push({ name: 'mark-ink', value: both(MARK_INK.light, MARK_INK.dark) });
  for (const [step, hex] of Object.entries(PRIMARY_PALETTE)) out.push({ name: `forest-${step}`, value: lower(hex) });
  // One swatch per provider the clients draw a glyph for; the sciotte_* rows repeat their provider's hex.
  for (const id of Object.keys(PROVIDER_GLYPH_INK).filter((p) => !p.startsWith('sciotte'))) {
    out.push({ name: `provider-${kebab(id)}`, value: lower(PROVIDER_COLORS[id as keyof typeof PROVIDER_COLORS]) });
  }
  return out;
}

/** Attach each family's notes by name, failing on a value with no note or a note with no value. */
export function annotate<T extends { name: string }>(family: string, values: T[], notes: Record<string, string>): Array<T & { usage: string }> {
  const names = new Set(values.map((v) => v.name));
  const orphans = Object.keys(notes).filter((n) => !names.has(n));
  const bare = values.filter((v) => !notes[v.name]).map((v) => v.name);
  if (orphans.length || bare.length) {
    throw new Error(`${family}: tokens without a note [${bare.join(', ')}]; notes without a token [${orphans.join(', ')}]`);
  }
  return values.map((v) => ({ ...v, usage: notes[v.name] }));
}

function stack(names: string[]): string {
  return names.map((n) => (/\s/.test(n) ? `"${n}"` : n)).join(', ');
}

function typeTokens(notes: TokenNotes) {
  const theme = tailwindTheme();
  const mobile = mobileScale();
  const families = Object.fromEntries(['display', 'sans', 'serif', 'mono'].map((k) => {
    const faces = theme.fontFamily[k];
    if (!faces) throw new Error(`tailwind.config.cjs has no fontFamily.${k}`);
    return [k, stack(faces)];
  }));
  const tracking = { brand: BRAND_TRACKING, heading: HEADING_TRACKING };
  const groups = notes.type.map((group) => ({
    name: group.name,
    family: group.family,
    styles: group.styles.map((s) => {
      let fontSize: string;
      let lineHeight: string;
      if (group.scale === 'mobile') {
        const step = mobile[s.step];
        if (!step) throw new Error(`typeScale.js has no step "${s.step}"`);
        fontSize = `${step.fontSize}px`;
        lineHeight = `${step.lineHeight}px`;
      } else {
        const step = theme.fontSize[s.step];
        if (!step) throw new Error(`tailwind.config.cjs has no fontSize "${s.step}"`);
        fontSize = remToPx(step[0]);
        lineHeight = remToPx(step[1].lineHeight);
      }
      return {
        name: s.name,
        ...(s.family ? { family: s.family } : {}),
        fontSize,
        lineHeight,
        fontWeight: s.fontWeight,
        ...(s.tracking ? { letterSpacing: tracking[s.tracking] } : {}),
        ...(s.fontStyle ? { fontStyle: s.fontStyle } : {}),
        ...(s.sample ? { sample: s.sample } : {}),
        usage: s.usage,
      };
    }),
  }));
  return { fonts: [], families, groups };
}

/** The athlete bubble's corner, which index.css sets as an arbitrary Tailwind value rather than a theme radius. */
function bubbleRadius(): string {
  const css = readFileSync(resolve(FRONTEND_ROOT, 'src/index.css'), 'utf8');
  const match = /\.chat-bubble-user\s*\{[^}]*rounded-\[(\d+px)\]/.exec(css);
  if (!match) throw new Error('index.css: .chat-bubble-user no longer sets rounded-[Npx]');
  return match[1];
}

export interface SourceRef { repo: string; ref: string }

export function buildTokens(source: SourceRef, components: Record<string, string>, synced: string) {
  const notes = readNotes();
  const theme = tailwindTheme();
  const section = theme.spacing.section;
  if (!section) throw new Error('tailwind.config.cjs has no spacing.section');
  const spacing = [
    ...Object.entries(SPACING).map(([k, px]) => ({ name: `space-${k === 'xxl' ? '2xl' : k}`, value: `${px}px` })),
    { name: 'space-section', value: remToPx(section) },
  ];
  const radius = [
    ...Object.entries(BORDER_RADIUS).filter(([k]) => k !== 'full').map(([k, px]) => ({ name: `radius-${k}`, value: `${px}px` })),
    { name: 'radius-bubble', value: bubbleRadius() },
    { name: 'radius-full', value: `${BORDER_RADIUS.full}px` },
  ];
  const shadow = [{ name: 'shadow-floating', value: { light: FLOATING_SHADOW.light, dark: FLOATING_SHADOW.dark } }];
  return {
    name: 'Dravr Boreal',
    version: 1,
    meta: {
      source: 'github',
      repo: source.repo,
      ref: source.ref,
      package: 'frontend',
      paths: {
        tokens: ['packages/shared-constants/src/design-system.ts', 'frontend/tailwind.config.cjs', 'frontend-mobile/src/constants/typeScale.js'],
        fonts: ['frontend/index.html'],
        assets: ['frontend/public/brand/', 'frontend/public/favicon.svg'],
        docs: ['frontend/DESIGN.md', 'frontend/BRAND.md', 'frontend/design-system/'],
      },
      components,
      synced,
    },
    color: {
      themes: SCHEMES.map((id) => ({ id, name: id === 'light' ? 'Light' : 'Dark' })),
      tokens: annotate('color', colorValues(), notes.colors),
    },
    type: typeTokens(notes),
    spacing: { tokens: annotate('spacing', spacing, notes.spacing) },
    radius: { tokens: annotate('radius', radius, notes.radius) },
    shadow: { note: 'Hairlines lift; shadows float. There is exactly one shadow.', tokens: annotate('shadow', shadow, notes.shadow) },
    layout: notes.layout,
    opacity: notes.opacity,
  };
}

/** Each card folder mapped to the source file its entry import names; a card the bundle does not export fails. */
export function componentSources(): Record<string, string> {
  const entry = readFileSync(BUNDLE_ENTRY, 'utf8');
  const imports = new Map<string, string>();
  for (const m of entry.matchAll(/^import\s+(?:\{([^}]+)\}|(\w+))\s+from\s+'([^']+)';$/gm)) {
    const names = m[1] ? m[1].split(',').map((n) => n.trim()) : [m[2]];
    const path = relative(resolve(FRONTEND_ROOT, '..'), resolve(CONTENT_ROOT, m[3]));
    for (const n of names) imports.set(n, `${path}.tsx`);
  }
  const cards = readdirSync(resolve(CONTENT_ROOT, 'components')).filter((d) => d !== COVER).sort();
  const missing = cards.filter((c) => !imports.has(c));
  if (missing.length) throw new Error(`bundle-entry.ts does not import the card component(s) ${missing.join(', ')}`);
  if (cards.length === 0) throw new Error('design-system/components holds no cards');
  return Object.fromEntries(cards.map((c) => [c, imports.get(c) as string]));
}
