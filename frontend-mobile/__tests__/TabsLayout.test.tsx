// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shell: three labelled system tabs in order, the unread badge on Chat, and no floating chrome in the tree
// ABOUTME: The glass pill and its "+" are gone; a bar drawn by hand, or a blur import anywhere, fails this

import React from 'react';
import { readdirSync, readFileSync, statSync } from 'fs';
import { join } from 'path';
import { render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockGetConversations = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
  },
}));
jest.mock('../src/hooks/useServerStatus', () => ({
  useServerStatus: () => ({ isServerReachable: true, isChecking: false, checkNow: jest.fn() }),
}));

import TabsLayout from '../app/(app)/(tabs)/_layout';
import { TAB_BAR_TABS } from '../src/navigation/tabs';
import { CHAT_LIST_ROUTE } from '../src/navigation/routes';

function renderTabs() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <TabsLayout />
    </QueryClientProvider>,
  );
}

/** Every source file under `dir`, recursively. */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe('the system tab bar', () => {
  beforeEach(() => {
    mockGetConversations.mockResolvedValue({ conversations: [], total: 0, limit: 50, offset: 0 });
  });

  it('offers exactly three labelled tabs, chat first', async () => {
    const { findAllByTestId, getAllByTestId } = renderTabs();

    const labels = (await findAllByTestId('tab-label')).map((node) => node.props.children);
    // The unit setup pins English.
    expect(labels).toEqual(['Chat', 'Discover', 'Settings']);
    expect(getAllByTestId(/^tab-(chat|discover|settings)$/).map((node) => node.props.testID)).toEqual([
      'tab-chat',
      'tab-discover',
      'tab-settings',
    ]);
    expect(TAB_BAR_TABS.map((tab) => tab.route)).toEqual(['(chat)', '(discover)', '(settings)']);
    expect(CHAT_LIST_ROUTE).toBe(`/(app)/(tabs)/${TAB_BAR_TABS[0].route}`);
  });

  it('wears the unread total on the chat tab, and nothing when there is none', async () => {
    mockGetConversations.mockResolvedValue({
      conversations: [
        { id: 'c1', title: 'A', unread_count: 2, message_count: 4, created_at: '2026-08-20T10:00:00Z', updated_at: '2026-08-25T10:00:00Z' },
        { id: 'c2', title: 'B', unread_count: 3, message_count: 4, created_at: '2026-08-20T10:00:00Z', updated_at: '2026-08-25T10:00:00Z' },
      ],
      total: 2,
      limit: 50,
      offset: 0,
    });
    const { findByTestId } = renderTabs();

    expect(await findByTestId('tab-badge')).toHaveTextContent('5');
  });

  it('draws no badge on a read list', async () => {
    const { findAllByTestId, queryByTestId } = renderTabs();
    await findAllByTestId('tab-label');

    expect(queryByTestId('tab-badge')).toBeNull();
  });

  /**
   * D1 of Boreal v2.2: the bar is the platform's. A blur import, or the glass
   * container that hosted the old pill, means someone drew a bar again.
   */
  it('imports no blur anywhere in the app', () => {
    const offenders = [...sourceFiles(join(__dirname, '..', 'src')), ...sourceFiles(join(__dirname, '..', 'app'))]
      .filter((file) => /expo-blur|expo-glass-effect|BlurView|GlassContainer|ExpandableTabBar/.test(readFileSync(file, 'utf8')))
      .map((file) => file.replace(join(__dirname, '..'), ''));

    expect(offenders).toEqual([]);
  });
});
