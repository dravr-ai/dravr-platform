// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for mobile UsageWarningBanner component
// ABOUTME: Tests rendering states, accessibility roles, and dismiss behavior

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
