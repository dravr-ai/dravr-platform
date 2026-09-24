// ABOUTME: Tests the web provider glyphs — the captured COROS id draws COROS' official mark
// ABOUTME: Pins the mark's own 1024 viewBox and that an unknown id falls back to the neutral disc
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { PROVIDER_COLORS } from '@pierre/shared-constants';
import { ProviderIcon } from '../ProviderConnectionCards';

describe('ProviderIcon', () => {
  it('draws the official COROS mark for the captured COROS account', () => {
    const { container } = render(<ProviderIcon providerId="sciotte_coros" />);
    const svg = container.querySelector('svg');
    // The mark is drawn at its source's own scale, not squeezed into 24x24.
    expect(svg?.getAttribute('viewBox')).toBe('0 0 1024 1024');
    expect(container.querySelector('path')?.getAttribute('d')).toMatch(/^M611\.28637781 226\.3848448/);
    expect(PROVIDER_COLORS.sciotte_coros).toBe('#F8273B');
  });

  it('falls back to the neutral disc for an unknown id', () => {
    const { container } = render(<ProviderIcon providerId="not_a_provider" />);
    expect(container.querySelector('svg')?.getAttribute('viewBox')).toBe('0 0 24 24');
  });
});
