// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the brand mark to the Boreal Ripple assets and the ink swap between schemes
// ABOUTME: A regression here means a Momentum badge or a wrong-scheme ink is back on a surface

import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { DravrLogo } from '../DravrLogo';

describe('DravrLogo', () => {
  it('draws the forest mark on light and the mint mark on dark from the same edge', () => {
    const { container } = render(<DravrLogo size={40} />);
    const images = Array.from(container.querySelectorAll('img'));
    expect(images.map((img) => img.getAttribute('src'))).toEqual([
      '/brand/mark-ink-96.png',
      '/brand/mark-mint-96.png',
    ]);
    expect(images[0].className).toContain('dark:hidden');
    expect(images[1].className).toContain('dark:block');
    for (const img of images) {
      expect(img.getAttribute('width')).toBe('40');
      expect(img.getAttribute('alt')).toBe('');
    }
  });

  it('is decorative: the wordmark or the chrome names the app, never the mark twice', () => {
    const { container } = render(<DravrLogo size={64} />);
    expect(container.firstElementChild?.getAttribute('aria-hidden')).toBe('true');
  });

  it('picks the smallest asset at least twice the rendered size', () => {
    const edgeFor = (size: number) => {
      const { container, unmount } = render(<DravrLogo size={size} />);
      const src = container.querySelector('img')?.getAttribute('src') ?? '';
      unmount();
      return Number(/mark-ink-(\d+)\.png$/.exec(src)?.[1]);
    };
    expect(edgeFor(28)).toBe(96);
    expect(edgeFor(48)).toBe(96);
    expect(edgeFor(64)).toBe(192);
    expect(edgeFor(96)).toBe(192);
    expect(edgeFor(220)).toBe(512);
    expect(edgeFor(400)).toBe(512);
  });

  it('never renders the retired Momentum badge', () => {
    const { container } = render(<DravrLogo />);
    expect(container.querySelector('svg')).toBeNull();
    expect(container.innerHTML).not.toContain('dravr-icon');
  });
});
