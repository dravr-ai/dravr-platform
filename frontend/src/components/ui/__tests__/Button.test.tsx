// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Button's success, danger-outline and warning-outline variants to their feedback hues and measures each label at rest and on hover, in both schemes
// ABOUTME: Approve and reactivate draw success, revoke and suspend danger-outline, impersonate warning-outline, so no state button hand-writes its hue

import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Button } from '../Button';
import { AA_TEXT, GROUNDS, inkOnFill, SCHEMES, toneRule, utilityFill } from '../../../test/contrast';

describe('Button success variant', () => {
  it('draws the btn-success class', () => {
    render(<Button variant="success">Approve</Button>);
    expect(screen.getByRole('button', { name: 'Approve' })).toHaveClass('btn-base', 'btn-success');
  });

  it.each(SCHEMES)('labels clear AA at rest and on hover, in %s', (scheme) => {
    const rest = toneRule('btn-success');
    const hover = toneRule('btn-success:hover');
    // A filled button is opaque, so its ground does not matter; `surface` stands in.
    expect(inkOnFill(scheme, rest.ink, rest.fill, 'surface')).toBeGreaterThanOrEqual(AA_TEXT);
    expect(inkOnFill(scheme, hover.ink, hover.fill, 'surface')).toBeGreaterThanOrEqual(AA_TEXT);
  });

  it('needs an opaque hover: white on a /80 hover falls under AA', () => {
    // `bg-activity hover:bg-activity/80 text-on-primary`: white on the /80
    // hover over a card in light.
    expect(inkOnFill('light', 'on-primary', utilityFill('bg-activity/80'), 'surface-container-lowest')).toBeLessThan(AA_TEXT);
  });
});

describe('Button danger-outline variant', () => {
  it('draws the btn-danger-outline class', () => {
    render(<Button variant="danger-outline">Revoke</Button>);
    expect(screen.getByRole('button', { name: 'Revoke' })).toHaveClass('btn-base', 'btn-danger-outline');
  });

  it.each(SCHEMES)('labels clear AA at rest and on the error hover tint over every ground, in %s', (scheme) => {
    const hover = toneRule('btn-danger-outline:hover');
    expect(hover.fill.token).toBe('error');
    const failures: string[] = [];
    for (const ground of GROUNDS) {
      // At rest the button is transparent: its label sits on the ground.
      const rest = inkOnFill(scheme, hover.ink, { token: 'error', alpha: 0 }, ground);
      const over = inkOnFill(scheme, hover.ink, hover.fill, ground);
      if (rest < AA_TEXT) failures.push(`rest over ${ground}: ${rest.toFixed(2)}:1`);
      if (over < AA_TEXT) failures.push(`hover over ${ground}: ${over.toFixed(2)}:1`);
    }
    expect(failures).toEqual([]);
  });

  it('needs the container ink: `text-error` on `hover:bg-error/10` falls under AA', () => {
    expect(inkOnFill('light', 'error', utilityFill('bg-error/10'), 'surface-container')).toBeLessThan(AA_TEXT);
    expect(inkOnFill('light', 'error', utilityFill('bg-error/20'), 'surface')).toBeLessThan(AA_TEXT);
  });
});

describe('Button warning-outline variant', () => {
  it('draws the btn-warning-outline class', () => {
    render(<Button variant="warning-outline">Impersonate</Button>);
    expect(screen.getByRole('button', { name: 'Impersonate' })).toHaveClass('btn-base', 'btn-warning-outline');
  });

  it.each(SCHEMES)('labels clear AA at rest and on the warning hover tint over every ground, in %s', (scheme) => {
    const hover = toneRule('btn-warning-outline:hover');
    expect(hover.fill.token).toBe('warning');
    const failures: string[] = [];
    for (const ground of GROUNDS) {
      // At rest the button is transparent: its label sits on the ground.
      const rest = inkOnFill(scheme, hover.ink, { token: 'warning', alpha: 0 }, ground);
      const over = inkOnFill(scheme, hover.ink, hover.fill, ground);
      if (rest < AA_TEXT) failures.push(`rest over ${ground}: ${rest.toFixed(2)}:1`);
      if (over < AA_TEXT) failures.push(`hover over ${ground}: ${over.toFixed(2)}:1`);
    }
    expect(failures).toEqual([]);
  });
});
