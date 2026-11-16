// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for mobile UsageWarningBanner component
// ABOUTME: Tests rendering states, each level's alert tone, accessibility roles, and dismiss behavior

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import { UsageWarningBanner } from '../src/screens/chat/UsageWarningBanner';

describe('UsageWarningBanner', () => {
  it('renders nothing when level is none', () => {
    const { queryByTestId } = render(
      <UsageWarningBanner level="none" message="" />
    );
    expect(queryByTestId('usage-warning-banner')).toBeNull();
  });

  it('renders nothing when message is empty', () => {
    const { queryByTestId } = render(
      <UsageWarningBanner level="warning" message="" />
    );
    expect(queryByTestId('usage-warning-banner')).toBeNull();
  });

  it('renders warning banner with message text', () => {
    const { getByTestId, getByText } = render(
      <UsageWarningBanner level="warning" message="80% of daily messages used" />
    );
    expect(getByTestId('usage-warning-banner')).toBeTruthy();
    expect(getByText('80% of daily messages used')).toBeTruthy();
  });

  it('renders burst banner with message text', () => {
    const { getByTestId, getByText } = render(
      <UsageWarningBanner level="burst" message="In burst zone for daily tokens" />
    );
    expect(getByTestId('usage-warning-banner')).toBeTruthy();
    expect(getByText('In burst zone for daily tokens')).toBeTruthy();
  });

  it('renders blocked banner with message text', () => {
    const { getByTestId, getByText } = render(
      <UsageWarningBanner level="blocked" message="Daily messages limit reached" />
    );
    expect(getByTestId('usage-warning-banner')).toBeTruthy();
    expect(getByText('Daily messages limit reached')).toBeTruthy();
  });

  // Each level draws through the alert table: warning and burst on the
  // warning tint (burst with the deeper edge), blocked on the error tint, the
  // message in the tone's bound ink — never the bare hue, which falls under
  // AA on its own tint.
  it.each([
    ['warning', 'bg-warning/10', 'border-warning/30', 'text-on-warning-container'],
    ['burst', 'bg-warning/10', 'border-warning/60', 'text-on-warning-container'],
    ['blocked', 'bg-error/10', 'border-error/30', 'text-on-error-container'],
  ] as const)('draws the %s level on the alert table', (level, fill, edge, ink) => {
    const { getByTestId, getByText } = render(<UsageWarningBanner level={level} message="Usage" />);
    const banner = getByTestId('usage-warning-banner');
    expect(banner.props.className).toContain(fill);
    expect(banner.props.className).toContain(edge);
    expect(getByText('Usage').props.className).toContain(ink);
  });

  it('has accessibility role alert', () => {
    const { getByTestId } = render(
      <UsageWarningBanner level="warning" message="80% used" />
    );
    const banner = getByTestId('usage-warning-banner');
    expect(banner.props.accessibilityRole).toBe('alert');
  });

  it('can be dismissed when not blocked', () => {
    const { getByTestId, queryByTestId } = render(
      <UsageWarningBanner level="warning" message="80% used" />
    );
    expect(getByTestId('dismiss-warning-button')).toBeTruthy();
    fireEvent.press(getByTestId('dismiss-warning-button'));
    expect(queryByTestId('usage-warning-banner')).toBeNull();
  });

  it('cannot be dismissed when blocked', () => {
    const { queryByTestId } = render(
      <UsageWarningBanner level="blocked" message="Limit reached" />
    );
    expect(queryByTestId('dismiss-warning-button')).toBeNull();
    expect(queryByTestId('usage-warning-banner')).toBeTruthy();
  });
});
