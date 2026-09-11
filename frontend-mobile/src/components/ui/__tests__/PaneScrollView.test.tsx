// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that every pane under the native header and tab bar insets itself for both
// ABOUTME: The Account pane's logout button once sat under a floating bar — caption visible, control untappable

import React from 'react';
import { Text } from 'react-native';
import { render, screen } from '@testing-library/react-native';
import { PaneScrollView } from '../PaneScrollView';

describe('PaneScrollView', () => {
  /**
   * carnet#253: a bar over the pane hid the Account pane's "Se déconnecter"
   * button — its caption rendered below the bar and was visible, the control
   * was not, and taps on it switched tabs instead. The system header and tab
   * bar are translucent too, and the platform's own inset is what keeps the
   * first and last rows out from under them.
   */
  it('lets the platform inset the content for the header and the tab bar', () => {
    render(
      <PaneScrollView testID="pane">
        <Text>row</Text>
      </PaneScrollView>,
    );

    expect(screen.getByTestId('pane').props.contentInsetAdjustmentBehavior).toBe('automatic');
  });

  it('keeps the caller style', () => {
    render(
      <PaneScrollView testID="pane" contentContainerStyle={{ padding: 16, gap: 24 }}>
        <Text>row</Text>
      </PaneScrollView>,
    );

    const style = screen.getByTestId('pane').props.contentContainerStyle as Record<string, number>;
    expect(style.padding).toBe(16);
    expect(style.gap).toBe(24);
  });

  it('forwards the other props the caller set', () => {
    render(
      <PaneScrollView testID="pane" showsVerticalScrollIndicator={false}>
        <Text>row</Text>
      </PaneScrollView>,
    );

    expect(screen.getByTestId('pane').props.showsVerticalScrollIndicator).toBe(false);
  });
});
