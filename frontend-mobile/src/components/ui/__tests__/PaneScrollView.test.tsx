// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that every pane under the floating tab bar reserves clearance for it
// ABOUTME: The Account pane's logout button sat under the bar — caption visible, control untappable

import React from 'react';
import { Text } from 'react-native';
import { render, screen } from '@testing-library/react-native';
import { PaneScrollView } from '../PaneScrollView';
import { tabBarBottomOffset } from '../ExpandableTabBar';

const BOTTOM_INSET = 34; // a device with a home indicator

jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 34, left: 0, right: 0 }),
}));

/** The `paddingBottom` the container actually applied, after the style merge. */
function paddingBottomOf(testID: string): number | undefined {
  const style = screen.getByTestId(testID).props.contentContainerStyle as unknown;
  const entries = (Array.isArray(style) ? style : [style]).filter(Boolean) as Record<string, number>[];
  return entries.reduce<number | undefined>(
    (acc, entry) => (entry?.paddingBottom === undefined ? acc : entry.paddingBottom),
    undefined,
  );
}

describe('PaneScrollView', () => {
  /**
   * carnet#253: the tab bar floats over the pane. Without this clearance the
   * Account pane's "Se déconnecter" button sat entirely behind it — its caption
   * rendered below the bar and was visible, the control was not, and taps on it
   * switched tabs instead.
   */
  it('reserves the tab bar clearance below the content', () => {
    render(
      <PaneScrollView testID="pane">
        <Text>row</Text>
      </PaneScrollView>,
    );

    expect(paddingBottomOf('pane')).toBe(tabBarBottomOffset(BOTTOM_INSET));
  });

  /**
   * The clearance is the container's job, so it has to survive a pane that sets
   * its own padding — that is exactly how a pane would silently lose it again.
   */
  it('wins over a padding the caller passed', () => {
    render(
      <PaneScrollView testID="pane" contentContainerStyle={{ padding: 16, paddingBottom: 8 }}>
        <Text>row</Text>
      </PaneScrollView>,
    );

    expect(paddingBottomOf('pane')).toBe(tabBarBottomOffset(BOTTOM_INSET));
  });

  it('keeps the rest of the caller style', () => {
    render(
      <PaneScrollView testID="pane" contentContainerStyle={{ padding: 16, gap: 24 }}>
        <Text>row</Text>
      </PaneScrollView>,
    );

    const style = screen.getByTestId('pane').props.contentContainerStyle as Record<string, number>[];
    const merged = Object.assign({}, ...style.filter(Boolean));
    expect(merged.padding).toBe(16);
    expect(merged.gap).toBe(24);
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
