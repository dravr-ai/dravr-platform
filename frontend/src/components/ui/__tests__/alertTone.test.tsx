// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures every alert tone's ink against its tint, composited over each ground, in both schemes
// ABOUTME: Joins ALERT_TONE to its index.css rule and the generated tokens, and pins the Alert block's shape and tone classes

import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Alert } from '../Alert';
import { ALERT_TONE, type AlertTone } from '../alertTone';
import { BADGE_TONE } from '../badgeTone';
import { WASH_TONE } from '../washTone';
import {
  AA_GRAPHIC,
  AA_TEXT,
  composite,
  contrast,
  GROUNDS,
  inkOnFill,
  SCHEMES,
  token,
  toneRule,
  utilityFill,
  utilityInk,
} from '../../../test/contrast';

const TONES = Object.keys(ALERT_TONE) as AlertTone[];

describe('alert tones', () => {
  it('has an index.css rule for every tone in the table, drawn in its own feedback hue', () => {
    for (const tone of TONES) {
      expect(toneRule(ALERT_TONE[tone].tone).fill.token).toBe(tone);
    }
  });

  it('names, as its child ink, the token its rule draws the body in', () => {
    for (const tone of TONES) {
      expect(utilityInk(ALERT_TONE[tone].ink)).toBe(toneRule(ALERT_TONE[tone].tone).ink);
    }
  });

  it.each(SCHEMES)('every body ink clears AA on its tint over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { fill, ink } = toneRule(ALERT_TONE[tone].tone);
      for (const ground of GROUNDS) {
        const ratio = inkOnFill(scheme, ink, fill, ground);
        if (ratio < AA_TEXT) failures.push(`${tone} over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it.each(SCHEMES)('every child ink — icon or title — clears the glyph floor and AA on its tint, in %s', (scheme) => {
    const failures: string[] = [];
    for (const tone of TONES) {
      const { fill } = toneRule(ALERT_TONE[tone].tone);
      const ink = utilityInk(ALERT_TONE[tone].ink);
      for (const ground of GROUNDS) {
        const ratio = inkOnFill(scheme, ink, fill, ground);
        if (ratio < Math.max(AA_GRAPHIC, AA_TEXT)) failures.push(`${tone} child over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('needs the bound ink: a bare hue on its own tint falls under the floor', () => {
    // In light: `bg-error/20
    // text-error` on the canvas (4.25:1), a `text-warning` glyph on
    // `bg-warning/10` over a sheet (2.60:1), `bg-success/10 text-success` on
    // the top tier (2.87:1).
    expect(inkOnFill('light', 'error', utilityFill('bg-error/20'), 'surface')).toBeLessThan(AA_TEXT);
    expect(inkOnFill('light', 'warning', utilityFill('bg-warning/10'), 'surface-container-low')).toBeLessThan(AA_GRAPHIC);
    expect(inkOnFill('light', 'success', utilityFill('bg-success/10'), 'surface-container-highest')).toBeLessThan(AA_GRAPHIC);
    for (const tone of ['error', 'warning', 'success'] as const) {
      const { fill, ink } = toneRule(ALERT_TONE[tone].tone);
      expect(inkOnFill('light', ink, fill, 'surface')).toBeGreaterThanOrEqual(AA_TEXT);
    }
  });
});

/**
 * The washes, read from the table the components draw them with (WASH_TONE:
 * the version history's cells, the private lane's composer and quote chip,
 * the current plan card), so a change to a component's fill or ink is a
 * change measured here.
 */
const WASHES: Array<[string, string, string]> = Object.entries(WASH_TONE).flatMap(([wash, { fill, inks }]) =>
  Object.entries(inks).map(([role, ink]): [string, string, string] => [`${wash}, ${role}`, utilityInk(ink), fill]),
);

describe('washes', () => {
  it.each(SCHEMES)('every ink clears AA on its wash over every ground, in %s', (scheme) => {
    const failures: string[] = [];
    for (const [name, ink, wash] of WASHES) {
      for (const ground of GROUNDS) {
        const ratio = inkOnFill(scheme, ink, utilityFill(wash), ground);
        if (ratio < AA_TEXT) failures.push(`${name} over ${ground}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(failures).toEqual([]);
  });

  it.each(SCHEMES)('the private composer placeholder clears AA on its wash over the bar it sits on, in %s', (scheme) => {
    // The composer bar is `bg-surface` (MessageInput) and the phone's is
    // `bg-background-primary`, the same token; `outline` on the wash falls to
    // 4.34:1 only on the top tier, where no composer sits.
    expect(inkOnFill(scheme, 'outline', utilityFill(WASH_TONE.privateLane.fill), 'surface')).toBeGreaterThanOrEqual(AA_TEXT);
  });
});

describe('a badge inside an alert', () => {
  it.each(SCHEMES)('the globally-disabled tools banner labels its error chips at AA on the admin canvas, in %s', (scheme) => {
    // ToolAvailability renders only under AdminConfiguration, which Dashboard
    // mounts in its page shell; nothing between them paints a ground, so the
    // banner sits on Dashboard's root `bg-surface`. The chips (Badge
    // `destructive` and the PIERRE_DISABLED_TOOLS code chip) composite twice.
    const alert = toneRule(ALERT_TONE.error.tone);
    const badge = toneRule(BADGE_TONE.error.tone);
    const behind = composite(badge.fill, scheme, composite(alert.fill, scheme, token(scheme, 'surface')));
    expect(contrast(token(scheme, badge.ink), behind)).toBeGreaterThanOrEqual(AA_TEXT);
  });
});

describe('Alert', () => {
  it.each(TONES)('draws the %s block in the alert shape and its tone class', (tone) => {
    render(
      <Alert tone={tone} data-testid="alert" className="mb-4">
        Message
      </Alert>,
    );
    const block = screen.getByTestId('alert');
    expect(block).toHaveTextContent('Message');
    expect(block).toHaveClass('alert', ALERT_TONE[tone].tone, 'mb-4');
  });

  it('passes the role through, and sets none of its own', () => {
    const { rerender } = render(<Alert tone="error" role="alert">Failed</Alert>);
    expect(screen.getByRole('alert')).toHaveTextContent('Failed');
    rerender(<Alert tone="info">Note</Alert>);
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
