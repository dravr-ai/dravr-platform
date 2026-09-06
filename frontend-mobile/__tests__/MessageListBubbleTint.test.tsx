// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the athlete's bubble to the sage tint and its paired ink, in both schemes
// ABOUTME: DESIGN.md §5 — the phone wore a neutral grey tier here while the web bubble already wore the tint

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';

jest.mock('nativewind', () => ({
  ...jest.requireActual('nativewind'),
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('@expo/vector-icons', () => {
  const { View } = require('react-native');
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

jest.mock('../src/services/api', () => ({
  ...jest.requireActual('../src/services/api'),
  userApi: { ...jest.requireActual('../src/services/api').userApi, updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider } from '../src/contexts/ThemeContext';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

const APPEARANCE_KEY = 'pierre.appearance_pref';

const MESSAGES: Message[] = [
  { id: 'm1', role: 'user', content: 'Repos ou seuil demain ?', created_at: new Date().toISOString() },
  { id: 'm2', role: 'assistant', content: 'Seuil — ta forme le permet.', created_at: new Date().toISOString() },
];

function renderList() {
  return render(
    <ThemeProvider>
      <MessageList
        bottomInset={0}
        messages={MESSAGES}
        isLoading={false}
        isSending={false}
        messageFeedback={{}}
        messageFeedbackComment={{}}
        flatListRef={React.createRef()}
        onScrollToBottom={jest.fn()}
        onThumbsUp={jest.fn()}
        onThumbsDown={jest.fn()}
        onSubmitFeedbackReason={jest.fn()}
        onRetryMessage={jest.fn()}
        onOpenUrl={jest.fn()}
        onReconnectProvider={jest.fn()}
      />
    </ThemeProvider>,
  );
}

/** Read the resolved text colour of a node whose style may be an array. */
function ink(node: { props: { style: unknown } }): string | undefined {
  const style = node.props.style as { color?: string } | Array<{ color?: string }>;
  const list = Array.isArray(style) ? style : [style];
  return list.reduce<string | undefined>((found, layer) => layer?.color ?? found, undefined);
}

/** Read the resolved background of a node whose style may be an array. */
function background(node: { props: { style: unknown } }): string | undefined {
  const style = node.props.style as { backgroundColor?: string } | Array<{ backgroundColor?: string }>;
  const list = Array.isArray(style) ? style : [style];
  return list.reduce<string | undefined>((found, layer) => layer?.backgroundColor ?? found, undefined);
}

/**
 * The athlete's bubble is the nearest filled ancestor of their own words.
 * Anchoring on the text rather than on the clock keeps this pointed at the
 * one message whose side is being asserted — both roles render a clock.
 */
function athleteBubble(screen: ReturnType<typeof renderList>) {
  let node = screen.getByText('Repos ou seuil demain ?').parent;
  while (node && background(node as never) === undefined) {
    node = node.parent;
  }
  return node;
}

describe("the athlete's bubble", () => {
  it('sits on the sage tint in light, not on a neutral grey tier', async () => {
    await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
    const screen = renderList();

    await waitFor(() => {
      expect(background(athleteBubble(screen) as never)).toBe(BOREAL_LIGHT.primaryContainer);
    });
    // The tier it used to wear. On the paper canvas it read as a grey blob.
    expect(background(athleteBubble(screen) as never)).not.toBe(BOREAL_LIGHT.surfaceContainerHigh);
  });

  it('sits on the same tint in dark', async () => {
    await AsyncStorage.setItem(APPEARANCE_KEY, 'dark');
    const screen = renderList();

    await waitFor(() => {
      expect(background(athleteBubble(screen) as never)).toBe(BOREAL_DARK.primaryContainer);
    });
  });

  it('pairs the tint with its own ink, never the body ink', async () => {
    await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
    const screen = renderList();

    // The assertion belongs INSIDE the wait. `useAppearancePref` starts at
    // the dark default and only flips once AsyncStorage resolves, so waiting
    // for the text alone succeeds on the first render — before the theme has
    // landed — and the colour check then races it.
    await waitFor(() => {
      expect(ink(screen.getByText('Repos ou seuil demain ?'))).toBe(
        BOREAL_LIGHT.onPrimaryContainer,
      );
    });

    // …and it is not the body ink the canvas uses.
    expect(ink(screen.getByText('Repos ou seuil demain ?'))).not.toBe(BOREAL_LIGHT.onSurface);
  });
});
