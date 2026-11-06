// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A settings pane hidden from operators redirects their deep link and still renders for athletes
// ABOUTME: Covers every role, because the menu filter cannot stop a link that names the route directly

import React from 'react';
import { Text } from 'react-native';
import { render, screen } from '@testing-library/react-native';
import { ADMIN_HIDDEN_PANES, settingsPane } from '@pierre/shared-constants';
import { hiddenFromOperators } from '../src/navigation/hiddenFromOperators';

let mockRole: string | undefined;

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: mockRole === undefined ? null : { role: mockRole } }),
}));

jest.mock('expo-router', () => {
  const { Text: MockText } = jest.requireActual('react-native');
  return { Redirect: ({ href }: { href: string }) => <MockText>{`redirect:${href}`}</MockText> };
});

const Gated = hiddenFromOperators(() => <Text>pane</Text>);

describe('hiddenFromOperators', () => {
  it.each(['admin', 'super_admin'])('sends a %s back to Settings', (role) => {
    mockRole = role;
    render(<Gated />);
    expect(screen.getByText('redirect:/(app)/(tabs)/(settings)')).toBeTruthy();
    expect(screen.queryByText('pane')).toBeNull();
  });

  it.each(['user', undefined])('renders the pane for role %s', (role) => {
    mockRole = role;
    render(<Gated />);
    expect(screen.getByText('pane')).toBeTruthy();
  });

  // The menu filter cannot stop a link naming the route, so every hidden pane's
  // route has to carry the gate itself. A route file is what Expo Router loads, so
  // loading and rendering it is the same path a deep link takes.
  const hiddenRoutes = [
    ...[...ADMIN_HIDDEN_PANES].flatMap((pane) => settingsPane(pane).mobile ?? []),
    '/(app)/connections',
  ];

  it.each(hiddenRoutes)('redirects an admin away from %s', (route) => {
    mockRole = 'admin';
    const { default: Route } = jest.requireActual(`../app${route}`) as { default: React.ComponentType };
    render(<Route />);
    expect(screen.getByText('redirect:/(app)/(tabs)/(settings)')).toBeTruthy();
  });
});
