// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Generates the Boreal custom-property blocks the web and mobile stylesheets import, light and dark
// ABOUTME: Reads the shared tokens, measures the pairings the clients draw, and fails closed below a WCAG floor

import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  BOREAL,
  BORDER_INK,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  FLOATING_SHADOW,
  PILLARS,
  PRIMARY_HOVER,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  ghostBorder,
  type BorealTokens,
  type ColorScheme,
  type HairlineInk,
} from '../src/design-system';
import { TEXT_FLOOR, contrast, hexToRgb, over, triple } from './wcag';

const PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO_ROOT = resolve(PACKAGE_ROOT, '../..');

/** The two stylesheets that draw with Tailwind classes over `--color-*` variables. */
export type Client = 'web' | 'mobile';

/**
 * Where each client's stylesheet imports its block from, repo-relative. Each
 * sits beside the stylesheet that imports it (`frontend/src/index.css`,
 * `frontend-mobile/global.css`), because both build tools resolve `@import`
 * relative to the importing file.
 */
export const CLIENT_CSS_PATHS: Record<Client, string> = {
  web: 'frontend/src/boreal-tokens.generated.css',
  mobile: 'frontend-mobile/boreal-tokens.generated.css',
};

const SCHEMES: readonly ColorScheme[] = ['light', 'dark'];

/** The pillar and feedback hues that bind an ink when drawn as a tint (DESIGN.md §2 "Bound ink"). */
type TintHue = keyof typeof CONTAINER_INKS;

/** Everything one scheme of the client stylesheets declares, each value read from the shared tokens. */
export interface ClientPalette {
  tokens: BorealTokens;
  primaryHover: string;
  /** The hue each bound ink sits on as a tint: a pillar, or a feedback colour. */
  hues: Record<TintHue, string>;
  inks: Record<TintHue, string>;
  hairline: HairlineInk;
  floatingShadow: string;
}

/** One scheme's palette from the shared constants. */
export function clientPalette(scheme: ColorScheme): ClientPalette {
  const semantic = scheme === 'light' ? SEMANTIC_COLORS : SEMANTIC_COLORS_DARK;
  const pillars = PILLARS[scheme];
  return {
    tokens: BOREAL[scheme],
    primaryHover: PRIMARY_HOVER[scheme],
    hues: {
      activity: pillars.activity,
      nutrition: pillars.nutrition,
      recovery: pillars.recovery,
      mobility: pillars.mobility,
      info: semantic.info,
      success: semantic.success,
      warning: semantic.warning,
    },
    inks: scheme === 'light' ? CONTAINER_INKS : CONTAINER_INKS_DARK,
    hairline: BORDER_INK[scheme],
    floatingShadow: FLOATING_SHADOW[scheme],
  };
}

/** Both schemes, as the committed stylesheets declare them. */
export function sharedPalettes(): Record<ColorScheme, ClientPalette> {
  return { light: clientPalette('light'), dark: clientPalette('dark') };
}

/** `surfaceContainerLow` → `surface-container-low`, the CSS name of an MD3 token. */
function kebab(key: string): string {
  return key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
}

/**
 * One custom property. `only` names the one client whose Tailwind config reads
 * it: the phone maps the two `*-fixed` tones, the web the hover fill and the
 * CSS shadow list (a native view takes `AMBIENT_SHADOW` instead).
 */
interface Declaration {
  name: string;
  value: string;
  only?: Client;
}

const BOREAL_GROUPS: ReadonlyArray<ReadonlyArray<keyof BorealTokens>> = [
  ['primary', 'onPrimary', 'primaryContainer', 'onPrimaryContainer', 'primaryFixed', 'primaryFixedDim'],
  ['tertiary', 'onTertiary', 'tertiaryContainer', 'onTertiaryContainer', 'tertiaryFixedDim'],
  ['error', 'onError', 'errorContainer', 'onErrorContainer'],
  [
    'surface',
    'surfaceDim',
    'surfaceBright',
    'surfaceContainerLowest',
    'surfaceContainerLow',
    'surfaceContainer',
    'surfaceContainerHigh',
    'surfaceContainerHighest',
    'surfaceVariant',
    'surfaceTint',
    'scrim',
    'onSurface',
    'onSurfaceVariant',
  ],
  ['outline', 'outlineVariant'],
];

const MOBILE_ONLY_TOKENS: ReadonlySet<keyof BorealTokens> = new Set(['primaryFixed', 'tertiaryFixedDim']);

/** A Boreal colour as the `--color-*` triple both Tailwind configs compose an alpha onto. */
function color(p: ClientPalette, key: keyof BorealTokens): Declaration {
  return {
    name: `--color-${kebab(key)}`,
    value: triple(p.tokens[key]),
    only: MOBILE_ONLY_TOKENS.has(key) ? 'mobile' : undefined,
  };
}

/** One scheme's declarations, grouped as the blocks print them. */
function declarationGroups(p: ClientPalette): Declaration[][] {
  const [primary, tertiary, error, surfaces, outlines] = BOREAL_GROUPS.map((group) =>
    group.map((key) => color(p, key)),
  );
  const hues = Object.keys(p.hues) as TintHue[];
  return [
    [...primary, { name: '--color-primary-hover', value: triple(p.primaryHover), only: 'web' }],
    tertiary,
    [
      ...error,
      ...(['success', 'warning', 'info', 'activity', 'nutrition', 'recovery', 'mobility'] as const).map((hue) => ({
        name: `--color-${hue}`,
        value: triple(p.hues[hue]),
      })),
    ],
    surfaces,
    hues.map((hue) => ({ name: `--color-on-${hue}-container`, value: triple(p.inks[hue]) })),
    outlines,
    [
      { name: '--ghost-border', value: ghostBorder(p.hairline, 'default') },
      { name: '--ghost-border-strong', value: ghostBorder(p.hairline, 'strong') },
      { name: '--ghost-border-faint', value: ghostBorder(p.hairline, 'faint') },
    ],
    [{ name: '--shadow-floating', value: p.floatingShadow, only: 'web' }],
  ];
}

/** One pairing a client draws, measured. */
export interface Measurement {
  scheme: ColorScheme;
  what: string;
  ratio: number;
  floor: number;
}

/**
 * The tint a bound ink is measured on. DESIGN.md §2 gives the envelope as
 * /10 through /15 over the whole ladder, and the denser tint is the binding
 * case in both schemes: it pulls the ground toward the hue and away from the
 * ink.
 */
const TINT_ALPHA = 0.15;

/** The ladder a text role or a tinted chip can sit on, lightest to darkest in light. */
const LADDER: ReadonlyArray<keyof BorealTokens> = [
  'surfaceContainerLowest',
  'surface',
  'surfaceContainerLow',
  'surfaceContainer',
  'surfaceContainerHigh',
  'surfaceContainerHighest',
];

/** The roles drawn as text on the ladder: body, secondary, helper labels, links. */
const TEXT_ROLES: ReadonlyArray<keyof BorealTokens> = ['onSurface', 'onSurfaceVariant', 'outline', 'primary'];

/** Every pairing one scheme is held to, measured from its palette. */
export function measureClientPairings(scheme: ColorScheme, p: ClientPalette): Measurement[] {
  const t = p.tokens;
  const row = (what: string, fg: string, bg: ReturnType<typeof hexToRgb>): Measurement => ({
    scheme,
    what,
    ratio: contrast(hexToRgb(fg), bg),
    floor: TEXT_FLOOR,
  });
  const rows: Measurement[] = [];
  for (const role of TEXT_ROLES) {
    for (const tier of LADDER) {
      rows.push(row(`${kebab(role)} on ${kebab(tier)}`, t[role], hexToRgb(t[tier])));
    }
  }
  for (const hue of Object.keys(p.hues) as TintHue[]) {
    for (const tier of LADDER) {
      const ground = over(hexToRgb(p.hues[hue]), TINT_ALPHA, hexToRgb(t[tier]));
      rows.push(row(`on-${hue}-container on ${hue}/15 over ${kebab(tier)}`, p.inks[hue], ground));
    }
  }
  rows.push(row('on-primary on primary', t.onPrimary, hexToRgb(t.primary)));
  rows.push(row('on-primary on primary-hover', t.onPrimary, hexToRgb(p.primaryHover)));
  rows.push(row('on-primary-container on primary-container', t.onPrimaryContainer, hexToRgb(t.primaryContainer)));
  rows.push(row('on-error on error', t.onError, hexToRgb(t.error)));
  rows.push(row('on-error-container on error-container', t.onErrorContainer, hexToRgb(t.errorContainer)));
  return rows;
}

/** The selectors each client scopes a scheme under, and the block that wraps them. */
const LAYOUT: Record<Client, { light: string; dark: string; layer: string | null; about: string }> = {
  web: {
    light: ':root',
    dark: 'html.dark',
    layer: null,
    about: 'The Boreal custom properties the web app draws with: light under :root, dark under html.dark',
  },
  mobile: {
    light: ':root',
    dark: ':root.dark, .dark',
    layer: 'base',
    about: 'The Boreal custom properties the phone draws with (NativeWind): light under :root, dark under .dark',
  },
};

function block(client: Client, selector: string, p: ClientPalette, indent: string): string {
  const groups = declarationGroups(p)
    .map((group) => group.filter((d) => d.only === undefined || d.only === client))
    .filter((group) => group.length > 0)
    .map((group) => group.map((d) => `${indent}  ${d.name}: ${d.value};`).join('\n'));
  return `${indent}${selector} {\n${groups.join('\n\n')}\n${indent}}`;
}

/** One client's stylesheet block, from the palettes. Throws below a contrast floor. */
export function renderClientCss(
  client: Client,
  palettes: Record<ColorScheme, ClientPalette> = sharedPalettes(),
): string {
  const rows = SCHEMES.flatMap((scheme) => measureClientPairings(scheme, palettes[scheme]));
  const failing = rows.filter((r) => r.ratio < r.floor);
  if (failing.length > 0) {
    const detail = failing
      .map((r) => `${r.scheme} ${r.what}: ${r.ratio.toFixed(2)}:1 < ${r.floor}:1`)
      .join('\n  ');
    throw new Error(`the client stylesheets would ship pairings under their WCAG floor:\n  ${detail}`);
  }
  const weakest = rows.reduce((min, r) => (r.ratio / r.floor < min.ratio / min.floor ? r : min));

  const layout = LAYOUT[client];
  const indent = layout.layer === null ? '' : '  ';
  const blocks = [
    block(client, layout.light, palettes.light, indent),
    block(client, layout.dark, palettes.dark, indent),
  ].join('\n\n');
  const body = layout.layer === null ? blocks : `@layer ${layout.layer} {\n${blocks}\n}`;

  return `/* ABOUTME: ${layout.about} */
/* ABOUTME: Generated by packages/shared-constants/scripts/generate-client-css.ts from design-system.ts - DO NOT EDIT */
/*
 * Regenerate from the shared tokens with:
 *   cd packages/shared-constants && bun run generate:client-css
 * scripts/ci/check-hosted-css.sh regenerates it and fails a push that left it behind.
 *
 * Every --color-* holds a bare \`r g b\` triple (no commas), so a Tailwind config
 * composes an alpha onto it: rgb(var(--color-x) / <alpha-value>). One dark class
 * on the root swaps the whole set.
 *
 * ${rows.length} pairings measured from the token values (WCAG 2.x, text ${TEXT_FLOOR}:1),
 * closest to its floor: ${weakest.ratio.toFixed(2)}:1 (${weakest.scheme} ${weakest.what}).
 */
${body}
`;
}

/** `--out-dir <dir>` writes each block under `<dir>/<repo path>` — the staleness check renders into a temp dir. */
function outputRoot(argv: readonly string[]): string {
  const at = argv.indexOf('--out-dir');
  if (at === -1) {
    return REPO_ROOT;
  }
  const target = argv[at + 1];
  if (!target) {
    throw new Error('--out-dir needs a path');
  }
  return resolve(process.cwd(), target);
}

if (import.meta.main) {
  const root = outputRoot(process.argv.slice(2));
  for (const client of Object.keys(CLIENT_CSS_PATHS) as Client[]) {
    const out = resolve(root, CLIENT_CSS_PATHS[client]);
    const css = renderClientCss(client);
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, css, 'utf8');
    console.log(`client css (${client}): ${css.length} bytes -> ${relative(process.cwd(), out) || out}`);
  }
}
