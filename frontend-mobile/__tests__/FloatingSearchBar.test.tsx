// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins where the floating search bar sits — above the tab bar on a home-indicator device, not behind it
// ABOUTME: Asserts the resolved `bottom` in points for a 34pt inset and for none, at rest and on keyboard dismissal

import React from 'react';
import { act, render } from '@testing-library/react-native';
import { Animated, Keyboard, Platform } from 'react-native';

let mockBottomInset = 34;
jest.mock('react-native-safe-area-context', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    SafeAreaProvider: ({ children }: { children: React.ReactNode }) =>
      React.createElement(View, null, children),
    SafeAreaView: ({ children, ...props }: { children: React.ReactNode }) =>
      React.createElement(View, props, children),
    useSafeAreaInsets: () => ({ top: 44, bottom: mockBottomInset, left: 0, right: 0 }),
    useSafeAreaFrame: () => ({ x: 0, y: 0, width: 390, height: 844 }),
  };
});

import { FloatingSearchBar } from '../src/components/ui/FloatingSearchBar';
import { tabBarBottomOffset, TAB_BAR_BOTTOM_OFFSET } from '../src/components/ui/ExpandableTabBar';

type Listener = (event: { endCoordinates: { height: number }; duration?: number }) => void;

/** Capture the keyboard listeners the bar registers so a test can fire them. */
function captureKeyboard() {
  const listeners: Record<string, Listener> = {};
  jest.spyOn(Keyboard, 'addListener').mockImplementation(((event: string, cb: Listener) => {
    listeners[event] = cb;
    return { remove: jest.fn() };
  }) as unknown as typeof Keyboard.addListener);
  return listeners;
}

/** The `bottom` the container actually renders with, in points. */
function renderedBottom(node: { props: { style: unknown } }): number {
  const style = node.props.style as Record<string, unknown> | Array<Record<string, unknown>>;
  const flat = Array.isArray(style) ? Object.assign({}, ...style) : style;
  const bottom = flat.bottom;
  // Animated hands the host view a resolved number; anything else means the
  // value never reached the layout and the assertion below would be a lie.
  if (typeof bottom === 'number') return bottom;
  const node_ = bottom as { __getValue?: () => number };
  if (typeof node_?.__getValue === 'function') return node_.__getValue();
  throw new Error(`search bar rendered a non-numeric bottom: ${String(bottom)}`);
}

function renderBar() {
  const { getByTestId } = render(
    <FloatingSearchBar value="" onChangeText={jest.fn()} testID="conversation-search-input" />,
  );
  return getByTestId('conversation-search-input-container');
}

const SHOW_EVENT = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
const HIDE_EVENT = Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide';

describe('FloatingSearchBar', () => {
  beforeEach(() => {
    mockBottomInset = 34;
  });
  afterEach(() => jest.restoreAllMocks());

  // The bar is absolute, so its parent's safe-area padding never reaches it.
  // Pinned to the bare 68pt constant it sat inside the tab bar, which occupies
  // 34..90 above the screen edge on this hardware (carnet#208).
  it('rests above the tab bar on a device with a home indicator', () => {
    captureKeyboard();
    expect(renderedBottom(renderBar())).toBe(102);
    expect(102).toBe(tabBarBottomOffset(34));
    expect(102).toBeGreaterThan(TAB_BAR_BOTTOM_OFFSET);
  });

  it('claims no dead space on a device without one', () => {
    mockBottomInset = 0;
    captureKeyboard();
    expect(renderedBottom(renderBar())).toBe(68);
    expect(68).toBe(tabBarBottomOffset(0));
  });

  // The animation runs outside React, so the target it is given is what the
  // rendered tree can be asked about. The dismissed target is the second half
  // of carnet#208: resting correctly and then animating back to the bare
  // constant would put the bar under the bar again on the first search.
  it('rides the keyboard up and animates back to the inset-aware resting place', () => {
    const listeners = captureKeyboard();
    const targets: unknown[] = [];
    jest.spyOn(Animated, 'timing').mockImplementation(((
      value: unknown,
      config: { toValue: unknown },
    ) => {
      targets.push(config.toValue);
      return { start: jest.fn() };
    }) as unknown as typeof Animated.timing);

    renderBar();

    act(() => listeners[SHOW_EVENT]({ endCoordinates: { height: 336 }, duration: 250 }));
    act(() => listeners[HIDE_EVENT]({ endCoordinates: { height: 0 }, duration: 250 }));

    expect(targets).toEqual([336, 102]);
  });
});
