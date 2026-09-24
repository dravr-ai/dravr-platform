// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Generates the one Boreal stylesheet every server-rendered hosted page embeds, in light and dark
// ABOUTME: Reads the shared tokens, measures every pairing the sheet draws, and fails closed below a WCAG floor

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { PRODUCT_WORDMARK } from '../src/brands';
import {
  BOREAL,
  BRAND_TRACKING,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  GHOST_BORDER,
  MARK_INK,
  PRIMARY_HOVER,
  PROVIDER_GLYPH_INK,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  TYPOGRAPHY,
  type ColorScheme,
} from '../src/design-system';

const PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO_ROOT = resolve(PACKAGE_ROOT, '../..');

/** Where the Rust crates read the sheet from: the lowest crate every template crate depends on. */
export const HOSTED_CSS_PATH = resolve(REPO_ROOT, 'crates/pierre-core/src/hosted_page.css');

/**
 * The mark the lockup draws. The web picks the smallest asset at least twice
 * the rendered size (`DravrLogo`), and the lockup renders it at 32px, so this
 * is the 96px file. Its alpha channel IS the mark, which is why the sheet uses
 * it as a mask and paints it in the scheme's mark ink.
 */
const MARK_ASSET_PATH = resolve(REPO_ROOT, 'frontend/public/brand/mark-ink-96.png');
const MARK_SIZE_PX = 32;

/** WCAG 1.4.3 for text, 1.4.11 for a graphic or a component's edge. */
const TEXT_FLOOR = 4.5;
const GRAPHIC_FLOOR = 3;

/** The notice draws `warning` as a /10 tint under a /40 edge, the pairing the web notice uses. */
const NOTICE_TINT = 0.1;
const NOTICE_EDGE = 0.4;
/** The connected tag and the success icon sit on a /15 success tint. */
const SUCCESS_TINT = 0.15;
/**
 * An error takes the notice's shape in its own hue: a /10 tint under a /40
 * edge for the banner, a /15 tint for the icon, both inked in the bound
 * `on-error-container`. The dense `error-container` fill would put a red slab
 * on the dark card that outshouts the page's only call to action.
 */
const ERROR_TINT = 0.1;
const ERROR_EDGE = 0.4;
const ERROR_ICON_TINT = 0.15;

const SCHEMES: readonly ColorScheme[] = ['light', 'dark'];

type Rgb = readonly [number, number, number];

/** A `#rrggbb` token as a channel triple. */
function hexToRgb(hex: string): Rgb {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!match) {
    throw new Error(`not a #rrggbb token: ${hex}`);
  }
  return [parseInt(match[1], 16), parseInt(match[2], 16), parseInt(match[3], 16)];
}

/** A colour drawn at `alpha` over an opaque ground, as the eye receives it. */
function over(top: Rgb, alpha: number, ground: Rgb): Rgb {
  return [0, 1, 2].map((i) => Math.round(top[i] * alpha + ground[i] * (1 - alpha))) as unknown as Rgb;
}

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function luminance([r, g, b]: Rgb): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** The WCAG contrast ratio between two opaque colours. */
export function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The tokens one scheme of the hosted pages draws with, each named for its role. */
export interface HostedPalette {
  surface: string;
  card: string;
  field: string;
  onSurface: string;
  onSurfaceVariant: string;
  outline: string;
  primary: string;
  primaryHover: string;
  onPrimary: string;
  primaryContainer: string;
  onPrimaryContainer: string;
  error: string;
  onErrorContainer: string;
  success: string;
  onSuccessContainer: string;
  warning: string;
  onWarningContainer: string;
  mark: string;
}

/**
 * One scheme's palette, every value read from the shared tokens.
 *
 * The card is the one place the schemes take different tiers (DESIGN.md §4):
 * light lifts to `surface-container-lowest` (white, separated from the paper
 * by its hairline), dark to `surface-container-high`, because dark's
 * `lowest` sits below the canvas and would sink the card instead of lifting it.
 */
export function hostedPalette(scheme: ColorScheme): HostedPalette {
  const tokens = BOREAL[scheme];
  const semantic = scheme === 'light' ? SEMANTIC_COLORS : SEMANTIC_COLORS_DARK;
  const inks = scheme === 'light' ? CONTAINER_INKS : CONTAINER_INKS_DARK;
  return {
    surface: tokens.surface,
    card: scheme === 'light' ? tokens.surfaceContainerLowest : tokens.surfaceContainerHigh,
    field: tokens.surfaceContainerLow,
    onSurface: tokens.onSurface,
    onSurfaceVariant: tokens.onSurfaceVariant,
    outline: tokens.outline,
    primary: tokens.primary,
    primaryHover: PRIMARY_HOVER[scheme],
    onPrimary: tokens.onPrimary,
    primaryContainer: tokens.primaryContainer,
    onPrimaryContainer: tokens.onPrimaryContainer,
    error: tokens.error,
    onErrorContainer: tokens.onErrorContainer,
    success: semantic.success,
    onSuccessContainer: inks.success,
    warning: semantic.warning,
    onWarningContainer: inks.warning,
    mark: MARK_INK[scheme],
  };
}

/** One foreground/ground pairing the sheet draws, measured. */
export interface Measurement {
  scheme: ColorScheme;
  what: string;
  ratio: number;
  floor: number;
}

/** Every pairing the sheet draws in one scheme, measured from the token values. */
export function measurePairings(scheme: ColorScheme): Measurement[] {
  const p = hostedPalette(scheme);
  const card = hexToRgb(p.card);
  const notice = over(hexToRgb(p.warning), NOTICE_TINT, card);
  const successTint = over(hexToRgb(p.success), SUCCESS_TINT, card);
  const errorTint = over(hexToRgb(p.error), ERROR_TINT, card);
  const errorIconTint = over(hexToRgb(p.error), ERROR_ICON_TINT, card);
  const row = (what: string, fg: Rgb, bg: Rgb, floor: number): Measurement => ({
    scheme,
    what,
    ratio: contrast(fg, bg),
    floor,
  });
  const rows: Measurement[] = [
    row('body text on the page', hexToRgb(p.onSurface), hexToRgb(p.surface), TEXT_FLOOR),
    row('body text on the card', hexToRgb(p.onSurface), card, TEXT_FLOOR),
    row('secondary text on the card', hexToRgb(p.onSurfaceVariant), card, TEXT_FLOOR),
    row('primary ink (links, wordmark, secondary button) on the card', hexToRgb(p.primary), card, TEXT_FLOOR),
    row('primary button label on its fill', hexToRgb(p.onPrimary), hexToRgb(p.primary), TEXT_FLOOR),
    row('primary button label on its hover fill', hexToRgb(p.onPrimary), hexToRgb(p.primaryHover), TEXT_FLOOR),
    row('notice ink (title, body, checkbox label) on the notice', hexToRgb(p.onWarningContainer), notice, TEXT_FLOOR),
    row('error text on the card', hexToRgb(p.error), card, TEXT_FLOOR),
    row('error banner ink on its tint', hexToRgb(p.onErrorContainer), errorTint, TEXT_FLOOR),
    row('error icon ink on its tint', hexToRgb(p.onErrorContainer), errorIconTint, TEXT_FLOOR),
    row('success ink (connected tag, success icon) on its tint', hexToRgb(p.onSuccessContainer), successTint, TEXT_FLOOR),
    row('number-match digit on its container', hexToRgb(p.onPrimaryContainer), hexToRgb(p.primaryContainer), TEXT_FLOOR),
    row('checkbox edge (outline) on the notice', hexToRgb(p.outline), notice, GRAPHIC_FLOOR),
    row('checked checkbox fill on the notice', hexToRgb(p.primary), notice, GRAPHIC_FLOOR),
    row('checkmark on the checked fill', hexToRgb(p.onPrimary), hexToRgb(p.primary), GRAPHIC_FLOOR),
    row('field underline (outline) on the card', hexToRgb(p.outline), card, GRAPHIC_FLOOR),
    row('focus ring and spinner arc on the card', hexToRgb(p.primary), card, GRAPHIC_FLOOR),
    row('mark on the card', hexToRgb(p.mark), card, GRAPHIC_FLOOR),
  ];
  for (const [provider, ink] of Object.entries(PROVIDER_GLYPH_INK)) {
    rows.push(row(`${provider} glyph on the card`, hexToRgb(ink[scheme] ?? p.onSurface), card, GRAPHIC_FLOOR));
  }
  return rows;
}

/** `#rrggbb` as the bare `r g b` triple every `--color-*` variable holds (DESIGN.md §2). */
function triple(hex: string): string {
  return hexToRgb(hex).join(' ');
}

/** CSS custom-property names for a provider id; ids are already `[a-z_]`. */
function glyphVar(provider: string): string {
  return `--glyph-${provider.replace(/_/g, '-')}`;
}

/** The checked-box glyph, stroked in the scheme's `on-primary`. */
function checkGlyph(onPrimary: string): string {
  const svg =
    "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'>" +
    `<path d='M3.5 8.5l3 3 6-7' fill='none' stroke='${onPrimary}' stroke-width='2' ` +
    "stroke-linecap='round' stroke-linejoin='round'/></svg>";
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
}

/** One scheme's custom properties, indented for its block. */
function schemeVariables(scheme: ColorScheme, indent: string): string {
  const p = hostedPalette(scheme);
  const colors: Array<[string, string]> = [
    ['surface', p.surface],
    ['card', p.card],
    ['field', p.field],
    ['on-surface', p.onSurface],
    ['on-surface-variant', p.onSurfaceVariant],
    ['outline', p.outline],
    ['primary', p.primary],
    ['primary-hover', p.primaryHover],
    ['on-primary', p.onPrimary],
    ['primary-container', p.primaryContainer],
    ['on-primary-container', p.onPrimaryContainer],
    ['error', p.error],
    ['on-error-container', p.onErrorContainer],
    ['success', p.success],
    ['on-success-container', p.onSuccessContainer],
    ['warning', p.warning],
    ['on-warning-container', p.onWarningContainer],
    ['mark', p.mark],
  ];
  const lines = colors.map(([name, hex]) => `--color-${name}: ${triple(hex)};`);
  lines.push(`--ghost-border: ${GHOST_BORDER[scheme]};`);
  lines.push(`--check-glyph: ${checkGlyph(p.onPrimary)};`);
  for (const [provider, ink] of Object.entries(PROVIDER_GLYPH_INK)) {
    const value = ink[scheme];
    lines.push(`${glyphVar(provider)}: ${value === null ? 'var(--color-on-surface)' : triple(value)};`);
  }
  return lines.map((line) => `${indent}${line}`).join('\n');
}

/** The contrast table the sheet's header carries, one aligned line per pairing. */
function contrastTable(rows: Measurement[]): string {
  const width = Math.max(...rows.map((r) => r.what.length)) + 2;
  return rows
    .map((r) => {
      const ratio = `${r.ratio.toFixed(2)}:1`;
      const floor = r.floor === TEXT_FLOOR ? 'text' : 'graphic';
      return ` *   ${r.scheme.padEnd(5)}  ${r.what.padEnd(width, '.')} ${ratio.padStart(8)}  (${floor} ${r.floor}:1)`;
    })
    .join('\n');
}

/** A font stack with the Boreal face first and the platform faces behind it. */
function stack(face: string): string {
  return `'${face}', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif`;
}

/** The mono stack for codes and IDs, with the platform monospace behind it. */
function monoStack(face: string): string {
  return `'${face}', ui-monospace, SFMono-Regular, Menlo, monospace`;
}

/**
 * The Google Fonts request for the faces a hosted page sets (DESIGN.md §3).
 * A face costs a download only where a page draws text in it, so the mono
 * face rides along for the one page that shows a code.
 */
function fontsImport(): string {
  const family = (face: string, weights: string) => `family=${face.replace(/ /g, '+')}:wght@${weights}`;
  return (
    'https://fonts.googleapis.com/css2?' +
    `${family(TYPOGRAPHY.headline, '500;600')}&${family(TYPOGRAPHY.body, '400;500;600')}` +
    `&${family(TYPOGRAPHY.mono, '400')}&display=swap`
  );
}

/** The whole sheet, from the tokens and the mark's bytes. Throws below a contrast floor. */
export function renderHostedCss(markPng: Uint8Array): string {
  if (markPng.length === 0) {
    throw new Error('the mark asset is empty; the lockup would draw nothing');
  }
  const rows = SCHEMES.flatMap(measurePairings);
  const failing = rows.filter((r) => r.ratio < r.floor);
  if (failing.length > 0) {
    const detail = failing
      .map((r) => `${r.scheme} ${r.what}: ${r.ratio.toFixed(2)}:1 < ${r.floor}:1`)
      .join('\n  ');
    throw new Error(`hosted pages would ship pairings under their WCAG floor:\n  ${detail}`);
  }

  const mask = `url("data:image/png;base64,${Buffer.from(markPng).toString('base64')}")`;
  const headline = stack(TYPOGRAPHY.headline);
  const body = stack(TYPOGRAPHY.body);
  const mono = monoStack(TYPOGRAPHY.mono);
  const glyphRules = Object.keys(PROVIDER_GLYPH_INK)
    .map((provider) => `.pc-glyph[data-provider="${provider}"] { background: rgb(var(${glyphVar(provider)})); }`)
    .join('\n');

  const css = `/* ABOUTME: The Boreal stylesheet every server-rendered hosted page embeds, light and dark */
/* ABOUTME: Generated by packages/shared-constants/scripts/generate-hosted-css.ts - DO NOT EDIT */
/*
 * Regenerate from the shared tokens with:
 *   cd packages/shared-constants && bun run generate:hosted-css
 * scripts/ci/check-hosted-css.sh regenerates it and fails a push that left it behind.
 *
 * Every pairing this sheet draws, measured from the token values (WCAG 2.x):
${contrastTable(rows)}
 */
@import url('${fontsImport()}');

:root {
  color-scheme: light dark;
  --mark-mask: ${mask};
${schemeVariables('light', '  ')}
}

@media (prefers-color-scheme: dark) {
  :root {
${schemeVariables('dark', '    ')}
  }
}

*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

[hidden] {
  display: none !important;
}

body {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgb(var(--color-surface));
  color: rgb(var(--color-on-surface));
  font-family: ${body};
  font-size: 15px;
  line-height: 23px;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

/* One sheet on the paper, lifted by its hairline; no shadow, no strip. */
.card {
  width: 100%;
  max-width: 420px;
  padding: 32px;
  background: rgb(var(--color-card));
  border: 1px solid var(--ghost-border);
  border-radius: 12px;
}

.card-center {
  text-align: center;
}

/* The lockup: the Boreal Ripple mark beside the wordmark, in the mark's own ink. */
.lockup {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  margin-bottom: 24px;
  color: rgb(var(--color-primary));
  font-family: ${headline};
  font-size: 18px;
  font-weight: 600;
  line-height: 24px;
  letter-spacing: ${BRAND_TRACKING};
}

.lockup::before {
  content: '';
  flex-shrink: 0;
  width: ${MARK_SIZE_PX}px;
  height: ${MARK_SIZE_PX}px;
  background-color: rgb(var(--color-mark));
  -webkit-mask-image: var(--mark-mask);
  mask-image: var(--mark-mask);
  -webkit-mask-size: contain;
  mask-size: contain;
  -webkit-mask-repeat: no-repeat;
  mask-repeat: no-repeat;
  -webkit-mask-position: center;
  mask-position: center;
}

.lockup::after {
  content: '${PRODUCT_WORDMARK}';
}

h1 {
  margin-bottom: 8px;
  color: rgb(var(--color-on-surface));
  font-family: ${headline};
  font-size: 22px;
  font-weight: 600;
  line-height: 28px;
  letter-spacing: -0.01em;
}

p {
  color: rgb(var(--color-on-surface-variant));
}

strong {
  color: rgb(var(--color-on-surface));
  font-weight: 600;
}

a {
  color: rgb(var(--color-primary));
}

code {
  font-family: ${mono};
  font-size: 13px;
}

.subtitle {
  margin-bottom: 24px;
  color: rgb(var(--color-on-surface-variant));
  font-size: 13px;
  line-height: 18px;
  text-align: center;
}

.lead {
  margin-bottom: 24px;
}

.fineprint {
  margin-top: 16px;
  font-size: 12px;
  line-height: 16px;
  text-align: center;
}

.pill {
  display: inline-block;
  margin-top: 8px;
  padding: 2px 10px;
  border: 1px solid var(--ghost-border);
  border-radius: 9999px;
  color: rgb(var(--color-on-surface-variant));
  font-size: 12px;
  line-height: 16px;
}

/* Fields: sentence-case label over an underlined field, no enclosing box. */
.field {
  margin-bottom: 20px;
}

.field label {
  display: block;
  margin-bottom: 4px;
  color: rgb(var(--color-on-surface-variant));
  font-size: 13px;
  font-weight: 500;
  line-height: 18px;
}

.field input {
  display: block;
  width: 100%;
  padding: 8px 0;
  border: 0;
  border-bottom: 1px solid rgb(var(--color-outline));
  border-radius: 0;
  background: transparent;
  color: rgb(var(--color-on-surface));
  font: inherit;
  font-size: 16px;
  line-height: 24px;
  transition: border-color 0.2s ease-out;
}

.field input:focus {
  outline: none;
  border-bottom: 2px solid rgb(var(--color-primary));
  padding-bottom: 7px;
}

.field input:focus-visible {
  outline: 2px solid rgb(var(--color-primary));
  outline-offset: 3px;
}

.field input:-webkit-autofill {
  -webkit-text-fill-color: rgb(var(--color-on-surface));
  -webkit-box-shadow: 0 0 0 1000px rgb(var(--color-card)) inset;
  caret-color: rgb(var(--color-on-surface));
}

/* Buttons: 44px, radius 8, one filled primary per view. */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 44px;
  padding: 0 16px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  font: inherit;
  font-size: 15px;
  font-weight: 500;
  line-height: 20px;
  text-decoration: none;
  cursor: pointer;
  transition: background-color 0.2s ease-out, border-color 0.2s ease-out;
}

.btn:focus-visible {
  outline: 2px solid rgb(var(--color-primary));
  outline-offset: 2px;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-block {
  display: flex;
  width: 100%;
}

.btn-primary {
  background: rgb(var(--color-primary));
  color: rgb(var(--color-on-primary));
}

.btn-primary:hover:not(:disabled) {
  background: rgb(var(--color-primary-hover));
}

.btn-secondary {
  border-color: var(--ghost-border);
  color: rgb(var(--color-primary));
}

.btn-secondary:hover:not(:disabled) {
  border-color: rgb(var(--color-primary) / 0.4);
  background: rgb(var(--color-primary) / 0.08);
}

.btn-tertiary {
  padding: 0 8px;
  color: rgb(var(--color-primary));
  font-size: 13px;
}

.btn-tertiary:hover:not(:disabled) {
  text-decoration: underline;
}

.back-link {
  margin: 0 0 4px -8px;
}

.actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* The notice: the warning tint under its bound ink (DESIGN.md §2, bound ink). */
.callout {
  margin-bottom: 20px;
  padding: 12px;
  border: 1px solid rgb(var(--color-warning) / ${NOTICE_EDGE});
  border-radius: 8px;
  background: rgb(var(--color-warning) / ${NOTICE_TINT});
  color: rgb(var(--color-on-warning-container));
  font-size: 13px;
  line-height: 18px;
}

.callout p {
  margin-bottom: 12px;
  color: inherit;
}

.callout .callout-title {
  margin-bottom: 4px;
  font-weight: 600;
}

/* A checkbox whose empty state shows: the outline token is its edge (WCAG 1.4.11). */
.check {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  color: inherit;
  font-weight: 500;
  cursor: pointer;
}

.check input[type="checkbox"] {
  -webkit-appearance: none;
  appearance: none;
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  margin-top: 1px;
  border: 1px solid rgb(var(--color-outline));
  border-radius: 4px;
  background-color: rgb(var(--color-field));
  background-position: center;
  background-repeat: no-repeat;
  background-size: 12px 12px;
  cursor: pointer;
}

.check input[type="checkbox"]:checked {
  border-color: rgb(var(--color-primary));
  background-color: rgb(var(--color-primary));
  background-image: var(--check-glyph);
}

.check input[type="checkbox"]:focus-visible {
  outline: 2px solid rgb(var(--color-primary));
  outline-offset: 2px;
}

.alert {
  margin-bottom: 16px;
  padding: 12px;
  border-radius: 8px;
  font-size: 13px;
  line-height: 18px;
  text-align: center;
}

.alert-error {
  border: 1px solid rgb(var(--color-error) / ${ERROR_EDGE});
  background: rgb(var(--color-error) / ${ERROR_TINT});
  color: rgb(var(--color-on-error-container));
}

.error-text {
  margin-top: 12px;
  color: rgb(var(--color-error));
  font-size: 13px;
  line-height: 18px;
}

/* A code the reader compares against another screen: mono, set apart. */
.user-code {
  margin: 8px 0 20px;
  padding: 12px;
  border: 1px solid var(--ghost-border);
  border-radius: 8px;
  color: rgb(var(--color-on-surface));
  font-family: ${mono};
  font-size: 22px;
  letter-spacing: 2px;
  line-height: 28px;
  text-align: center;
}

.spinner {
  width: 40px;
  height: 40px;
  margin: 0 auto 16px;
  border: 3px solid var(--ghost-border);
  border-top-color: rgb(var(--color-primary));
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.spinner.spinner-sm {
  width: 24px;
  height: 24px;
  margin: 16px auto 0;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* A phase that is waiting or done: an icon, a line, a sentence. */
.status {
  padding: 24px 0;
  text-align: center;
}

.status p {
  margin-top: 8px;
  font-size: 13px;
  line-height: 18px;
}

.status .btn {
  margin-top: 16px;
}

.status-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  margin: 0 auto 16px;
  border-radius: 50%;
  font-size: 28px;
  font-weight: 600;
  line-height: 1;
}

.status-icon svg {
  width: 28px;
  height: 28px;
}

.status-icon-success {
  background: rgb(var(--color-success) / ${SUCCESS_TINT});
  color: rgb(var(--color-on-success-container));
}

.status-icon-error {
  background: rgb(var(--color-error) / ${ERROR_ICON_TINT});
  color: rgb(var(--color-on-error-container));
}

.number-match {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 96px;
  height: 96px;
  margin: 0 auto 16px;
  border-radius: 12px;
  background: rgb(var(--color-primary-container));
  color: rgb(var(--color-on-primary-container));
  font-family: ${headline};
  font-size: 40px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

/* The connect picker: one hairline row per provider. */
.provider-card {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 52px;
  margin-bottom: 8px;
  padding: 12px 16px;
  border: 1px solid var(--ghost-border);
  border-radius: 8px;
  background: transparent;
  color: rgb(var(--color-on-surface));
  font: inherit;
  font-size: 15px;
  font-weight: 600;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.2s ease-out, border-color 0.2s ease-out;
}

.provider-card:hover:not(:disabled) {
  border-color: rgb(var(--color-primary));
  background: rgb(var(--color-primary) / 0.04);
}

.provider-card:focus-visible {
  outline: 2px solid rgb(var(--color-primary));
  outline-offset: 2px;
}

.provider-card:disabled {
  cursor: default;
}

/* A provider's glyph ink per scheme, from PROVIDER_GLYPH_INK: its brand where
   that clears 3:1 on the card, the body ink where it does not. */
.pc-glyph {
  flex-shrink: 0;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: rgb(var(--color-on-surface));
}

${glyphRules}

.pc-label {
  flex: 1;
}

.pc-tag {
  padding: 2px 8px;
  border: 1px solid var(--ghost-border);
  border-radius: 9999px;
  color: rgb(var(--color-on-surface-variant));
  font-size: 12px;
  font-weight: 500;
  line-height: 16px;
}

.pc-tag.connected {
  border-color: transparent;
  background: rgb(var(--color-success) / ${SUCCESS_TINT});
  color: rgb(var(--color-on-success-container));
}

/* The OAuth consent page: who asks, and for what. */
.client {
  color: rgb(var(--color-on-surface));
  font-weight: 600;
  word-break: break-all;
}

.scopes {
  margin-bottom: 24px;
  padding: 16px;
  border: 1px solid var(--ghost-border);
  border-radius: 8px;
  text-align: left;
}

.scopes-title {
  margin-bottom: 8px;
  color: rgb(var(--color-on-surface-variant));
  font-size: 13px;
  font-weight: 500;
  line-height: 18px;
}

.scopes ul {
  list-style: none;
}

.scopes li {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 0;
  color: rgb(var(--color-on-surface));
}

.scopes li::before {
  content: '';
  flex-shrink: 0;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: rgb(var(--color-primary));
}

.greeting {
  margin-bottom: 24px;
  color: rgb(var(--color-on-surface-variant));
  text-align: center;
}

.register-fields {
  display: none;
}

.register-fields.active {
  display: block;
}

@media (max-width: 640px) {
  body {
    padding: 12px;
  }

  .card {
    padding: 24px;
  }
}

@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
`;

  // The templates are filled by string replacement, so a brace pair in the
  // sheet would read as a placeholder to every renderer and to the tests that
  // assert none survive a render.
  if (css.includes('{{') || css.includes('}}')) {
    throw new Error('the sheet contains a template placeholder brace pair');
  }
  return css;
}

/** Read the mark and render the sheet. */
export function generateHostedCss(): string {
  return renderHostedCss(readFileSync(MARK_ASSET_PATH));
}

/** `--out <path>` writes somewhere else — the staleness check renders into a temp file. */
function outputPath(argv: readonly string[]): string {
  const at = argv.indexOf('--out');
  if (at === -1) {
    return HOSTED_CSS_PATH;
  }
  const target = argv[at + 1];
  if (!target) {
    throw new Error('--out needs a path');
  }
  return resolve(process.cwd(), target);
}

if (import.meta.main) {
  const out = outputPath(process.argv.slice(2));
  const css = generateHostedCss();
  writeFileSync(out, css, 'utf8');
  const rows = SCHEMES.flatMap(measurePairings);
  const weakest = rows.reduce((min, r) => (r.ratio / r.floor < min.ratio / min.floor ? r : min));
  console.log(
    `hosted page css: ${css.length} bytes, ${rows.length} pairings measured, ` +
      `closest to its floor ${weakest.ratio.toFixed(2)}:1 (${weakest.scheme} ${weakest.what}) -> ${out}`,
  );
}
