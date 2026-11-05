// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Resolves the Boreal colour tokens into the literal strings Chart.js paints with
// ABOUTME: A canvas cannot read a CSS variable, so the values are re-read whenever the theme flips

import { useMemo } from 'react';
import { useTheme } from './useTheme';

export interface ChartTheme {
  /** The scheme these values were read under; a new one means a new palette. */
  scheme: 'light' | 'dark';
  /** Legend, axis title and tick labels. */
  text: string;
  /** Grid lines. */
  grid: string;
  /** Series colours in draw order, each legible on a card of the current theme. */
  series: string[];
}

// Primary, then the pillar accents, then two neutrals so a chart with many slices
// still draws each in its own colour. `activity` holds index 5, which UsageAnalytics
// names. `success`, `warning` and `info` stay out: dark mode gives those three the
// same values as the pillars, so listing them would draw two series in one colour.
const SERIES_TOKENS = [
  '--color-primary',
  '--color-nutrition',
  '--color-recovery',
  '--color-mobility',
  '--color-error',
  '--color-activity',
  '--color-on-surface',
  '--color-outline',
] as const;

function readToken(style: CSSStyleDeclaration, name: string): string {
  return `rgb(${style.getPropertyValue(name).trim().split(/\s+/).join(', ')})`;
}

/** A resolved colour at reduced opacity, for fills under a line or inside a bar. */
export function tint(color: string, alpha: number): string {
  return color.replace('rgb(', 'rgba(').replace(')', `, ${alpha})`);
}

function readChartTheme(scheme: ChartTheme['scheme']): ChartTheme {
  const style = getComputedStyle(document.documentElement);
  return {
    scheme,
    text: readToken(style, '--color-on-surface-variant'),
    grid: readToken(style, '--color-outline-variant'),
    series: SERIES_TOKENS.map((name) => readToken(style, name)),
  };
}

/**
 * The chart palette for the theme on screen.
 *
 * `ThemeProvider` puts `.dark` on `<html>` before it publishes the new scheme, so
 * the render this hook re-runs in already reads the flipped variables.
 */
export function useChartTheme(): ChartTheme {
  const { scheme } = useTheme();
  return useMemo(() => readChartTheme(scheme), [scheme]);
}
