// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the shared USD cost formatter.
// ABOUTME: Pins the zero case and sub-cent precision that the three old copies each got wrong.

import { describe, it, expect } from 'vitest';
import { formatCost } from '../formatCost';

describe('formatCost', () => {
  it('renders exactly zero as $0.00, matching the whole-cent rows beside it', () => {
    // ActivityTab rendered "$0.0000" here; LlmConsumptionPanel special-cased it
    // only after the bug was reported live.
    expect(formatCost(0)).toBe('$0.00');
  });

  it('keeps two decimals for whole-cent amounts', () => {
    expect(formatCost(0.48)).toBe('$0.48');
    expect(formatCost(1.06)).toBe('$1.06');
    expect(formatCost(12)).toBe('$12.00');
  });

  it('shows sub-cent spend at significant digits instead of collapsing it', () => {
    // The real value observed from /admin/usage/recent-activity for
    // llama-3.1-8b-instant. toFixed(2) gives "$0.00" and toFixed(4) gives
    // "$0.0001" — both misreport it; toFixed(4) on a smaller figure gives
    // "$0.0000", which is what shipped.
    expect(formatCost(0.00008688)).toBe('$0.000087');
  });

  it('does not collapse a value that four decimal places would zero out', () => {
    // 0.00004 -> toFixed(4) === "0.0000". This is the case that made the old
    // sub-cent branch pointless.
    const rendered = formatCost(0.00004);
    expect(rendered).not.toBe('$0.0000');
    expect(rendered).toBe('$0.00004');
  });

  it('treats the cent boundary consistently', () => {
    expect(formatCost(0.01)).toBe('$0.01');
    expect(formatCost(0.0099)).toBe('$0.0099');
  });

  it('never renders NaN or Infinity into a cost column', () => {
    expect(formatCost(Number.NaN)).toBe('$0.00');
    expect(formatCost(Number.POSITIVE_INFINITY)).toBe('$0.00');
  });

  it('keeps the sign in front of the currency symbol for credits', () => {
    expect(formatCost(-0.48)).toBe('-$0.48');
  });
});
