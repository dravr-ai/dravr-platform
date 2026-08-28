// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The precached shell opens offline looking healthy; this strip is what says otherwise
// ABOUTME: Asserts it stays out of the way while online and is announced politely when it appears

import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import OfflineBanner from '../OfflineBanner';

function setOnLine(value: boolean) {
  Object.defineProperty(window.navigator, 'onLine', {
    configurable: true,
    get: () => value,
  });
}

afterEach(() => setOnLine(true));

describe('OfflineBanner', () => {
  it('renders nothing at all while online', () => {
    setOnLine(true);
    const { container } = render(<OfflineBanner />);
    expect(container).toBeEmptyDOMElement();
  });

  it('names the connection as the problem once offline', () => {
    setOnLine(false);
    render(<OfflineBanner />);
    const banner = screen.getByTestId('offline-banner');
    expect(banner).toBeInTheDocument();
    expect(banner).toHaveTextContent("You're offline");
  });

  it('is announced politely rather than as an interrupting alert', () => {
    setOnLine(false);
    render(<OfflineBanner />);
    const banner = screen.getByRole('status');
    expect(banner).toHaveAttribute('aria-live', 'polite');
  });

  it('clears itself when the connection comes back', () => {
    setOnLine(false);
    render(<OfflineBanner />);
    expect(screen.getByTestId('offline-banner')).toBeInTheDocument();

    act(() => {
      setOnLine(true);
      window.dispatchEvent(new Event('online'));
    });
    expect(screen.queryByTestId('offline-banner')).not.toBeInTheDocument();
  });

  it('pads for the notch, since viewport-fit=cover puts y=0 under the status bar', () => {
    setOnLine(false);
    render(<OfflineBanner />);
    expect(screen.getByTestId('offline-banner').getAttribute('style')).toContain(
      'safe-area-inset-top',
    );
  });
});
