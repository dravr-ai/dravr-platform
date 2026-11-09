// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the chart palette follows the theme on screen rather than a colour picked once
// ABOUTME: Drives the real ThemeProvider, so a flip travels the same path a user's toggle does

import React from 'react';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { ThemeProvider, useTheme } from '../useTheme';
import { tint, useChartTheme } from '../useChartTheme';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const LIGHT = { primary: '37 95 77', text: '65 72 69' };
const DARK = { primary: '163 208 190', text: '192 200 195' };

// jsdom has no stylesheet, so the swap `html.dark` does in the token block is
// reproduced here: the variables must be read after the class lands, which is the
// ordering a palette resolved too early would get wrong.
const stylesheet = document.createElement('style');
const FAKE_TOKENS = `
  :root { --color-primary: ${LIGHT.primary}; --color-on-surface-variant: ${LIGHT.text}; }
  html.dark { --color-primary: ${DARK.primary}; --color-on-surface-variant: ${DARK.text}; }
`;

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <ThemeProvider>{children}</ThemeProvider>
);

const rgb = (triple: string) => `rgb(${triple.split(' ').join(', ')})`;

describe('useChartTheme', () => {
  beforeEach(() => {
    stylesheet.textContent = FAKE_TOKENS;
    document.head.appendChild(stylesheet);
    window.localStorage.setItem('dravr.theme', 'light');
  });

  afterEach(() => {
    stylesheet.remove();
    window.localStorage.removeItem('dravr.theme');
    document.documentElement.classList.remove('dark');
  });

  it('reads the tokens of the theme on screen and re-reads them when it flips', () => {
    const { result } = renderHook(() => ({ chart: useChartTheme(), theme: useTheme() }), { wrapper });

    expect(result.current.chart.scheme).toBe('light');
    expect(result.current.chart.series[0]).toBe(rgb(LIGHT.primary));
    expect(result.current.chart.text).toBe(rgb(LIGHT.text));

    act(() => result.current.theme.toggle());

    expect(result.current.chart.scheme).toBe('dark');
    expect(result.current.chart.series[0]).toBe(rgb(DARK.primary));
    expect(result.current.chart.text).toBe(rgb(DARK.text));
  });

  it('keeps the same palette object across renders of one theme', () => {
    const { result, rerender } = renderHook(() => useChartTheme(), { wrapper });
    const first = result.current;
    rerender();
    // A new object per render would hand Chart.js new options and redraw every chart.
    expect(result.current).toBe(first);
  });

  it('tints a resolved colour without changing its hue', () => {
    expect(tint(rgb(LIGHT.primary), 0.6)).toBe('rgba(37, 95, 77, 0.6)');
  });

  it('draws eight series in eight colours under the real tokens of both themes', () => {
    // The token file is the data under test: its values are what must stay apart,
    // and jsdom loads no stylesheet of its own to read them from.
    stylesheet.textContent = readFileSync(resolve(process.cwd(), 'src/boreal-tokens.generated.css'), 'utf8');
    const { result } = renderHook(() => ({ chart: useChartTheme(), theme: useTheme() }), { wrapper });

    expect(result.current.chart.series).toHaveLength(8);
    expect(new Set(result.current.chart.series).size).toBe(8);

    act(() => result.current.theme.toggle());

    expect(result.current.chart.scheme).toBe('dark');
    expect(new Set(result.current.chart.series).size).toBe(8);
  });
});
