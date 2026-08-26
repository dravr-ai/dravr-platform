// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the floating tab bar — the tab set it renders, the tabs layout, and the route groups on disk
// ABOUTME: Pins the three copies of the tab list to each other so a tab removed from one cannot linger in another

import React from 'react';
import fs from 'fs';
import path from 'path';
import { fireEvent, render } from '@testing-library/react-native';

const mockRouter = {
  push: jest.fn(),
  replace: jest.fn(),
  back: jest.fn(),
  navigate: jest.fn(),
  canGoBack: () => true,
};
let mockSegments: string[] = ['(app)', '(tabs)', '(chat)'];
let mockGlobalParams: Record<string, string> = {};
const mockTabsScreen = jest.fn((_props: { name: string; options?: { title?: string } }) => null);

jest.mock('expo-router', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    useRouter: () => mockRouter,
    useSegments: () => mockSegments,
    useGlobalSearchParams: () => mockGlobalParams,
    useLocalSearchParams: () => ({}),
    useFocusEffect: () => {},
    Tabs: Object.assign(
      ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
      { Screen: (props: { name: string; options?: { title?: string } }) => mockTabsScreen(props) },
    ),
  };
});

jest.mock('../src/hooks/useServerStatus', () => ({
  useServerStatus: () => ({ isServerReachable: true, isChecking: false, checkNow: jest.fn() }),
}));
jest.mock('../src/components/ServerStatusBanner', () => ({ ServerStatusBanner: () => null }));
jest.mock('../src/services/api', () => ({
  chatApi: {
    createConversation: jest.fn(),
    listParticipants: jest.fn().mockResolvedValue([]),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
  },
  groupsApi: { listMyGroups: jest.fn().mockResolvedValue({ groups: [] }) },
}));

import { ExpandableTabBar, TAB_BAR_TABS } from '../src/components/ui/ExpandableTabBar';
import TabsLayout from '../app/(app)/(tabs)/_layout';
import { CHAT_LIST_ROUTE, CHAT_THREAD_ROUTE } from '../src/navigation/routes';

/** The route-group directories expo-router turns into tabs. */
const TABS_DIR = path.join(__dirname, '..', 'app', '(app)', '(tabs)');

/** The testIDs of the collapsed pill's tab buttons, in render order. */
function renderedTabIds(getAllByTestId: (id: RegExp) => Array<{ props: { testID: string } }>): string[] {
  return getAllByTestId(/^tab-[a-z]+$/).map((node) => node.props.testID);
}

describe('ExpandableTabBar', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSegments = ['(app)', '(tabs)', '(chat)'];
    mockGlobalParams = {};
  });

  it('puts Chat first, where the app lands, and lists no Coaches tab', () => {
    expect(TAB_BAR_TABS.map((tab) => tab.route)).toEqual(['(chat)', '(discover)', '(groups)', '(settings)']);
    expect(CHAT_LIST_ROUTE).toBe(`/(app)/(tabs)/${TAB_BAR_TABS[0].route}`);
    expect(TAB_BAR_TABS.map((tab) => tab.testID)).not.toContain('tab-coaches');
  });

  // Turns red if the bar and the tabs layout ever list different tabs again —
  // the layout used to hold an independent copy of this list, which is how a
  // tab could be filtered from one and not the other.
  it('renders exactly the tabs the tabs layout registers, in the same order', () => {
    const { getAllByTestId } = render(<ExpandableTabBar />);
    const rendered = renderedTabIds(getAllByTestId);
    const renderedLabels = getAllByTestId(/^tab-[a-z]+$/).map((node) => node.props.accessibilityLabel);

    render(<TabsLayout />);
    const registered = mockTabsScreen.mock.calls.map(([props]) => props.name);
    const registeredTitles = mockTabsScreen.mock.calls.map(([props]) => props.options?.title);

    expect(rendered).toEqual(['tab-chat', 'tab-discover', 'tab-groups', 'tab-settings']);
    expect(registered).toEqual(['(chat)', '(discover)', '(groups)', '(settings)']);
    expect(renderedLabels).toEqual(registeredTitles);
    expect(rendered).toHaveLength(registered.length);
  });

  // Turns red if a route group survives on disk after leaving the list (or the
  // other way round): expo-router registers every `(group)` directory as a
  // tab whether or not the bar shows it.
  it('matches the route groups on disk one to one', () => {
    const groupsOnDisk = fs
      .readdirSync(TABS_DIR)
      .filter((entry) => entry.startsWith('(') && entry.endsWith(')'))
      .sort();
    expect(groupsOnDisk).toEqual([...TAB_BAR_TABS.map((tab) => tab.route)].sort());
    expect(groupsOnDisk).not.toContain('(coaches)');
    expect(groupsOnDisk).not.toContain('(social)');
  });

  // The expanded menu sits behind the animated pill (display: none until the
  // "+" opens it), so its rows are queried with hidden elements included.
  it('renders no Coaches tab, menu item or quick action', () => {
    const { queryByTestId } = render(<ExpandableTabBar />);
    expect(queryByTestId('tab-coaches')).toBeNull();
    expect(queryByTestId('tab-menu-item-(chat)', { includeHiddenElements: true })).toBeTruthy();
    expect(queryByTestId('tab-menu-item-(coaches)', { includeHiddenElements: true })).toBeNull();
    expect(queryByTestId('quick-action-new-coach', { includeHiddenElements: true })).toBeNull();
  });

  it('opens another tab by its route group', () => {
    const { getByTestId } = render(<ExpandableTabBar />);
    fireEvent.press(getByTestId('tab-discover'));
    expect(mockRouter.navigate).toHaveBeenCalledWith('/(app)/(tabs)/(discover)');
  });

  it('re-tapping Chat from inside a thread pops back to the conversation list', () => {
    mockSegments = ['(app)', '(tabs)', '(chat)', '[conversationId]'];
    mockGlobalParams = { conversationId: 'conv-1' };
    const { getByTestId } = render(<ExpandableTabBar />);
    fireEvent.press(getByTestId('tab-chat'));
    expect(mockRouter.navigate).toHaveBeenCalledWith(CHAT_LIST_ROUTE);
  });

  it('offers the chat quick actions behind the "+", and new chat opens an empty thread', () => {
    const hidden = { includeHiddenElements: true };
    const { getByTestId, getByLabelText, queryByTestId } = render(<ExpandableTabBar />);
    fireEvent.press(getByLabelText('Open menu'));
    // The "+" flipped to the close affordance: the menu is open.
    expect(getByLabelText('Close menu')).toBeTruthy();

    expect(getByTestId('quick-action-new-chat', hidden)).toBeTruthy();
    expect(getByTestId('quick-action-new-group-chat', hidden)).toBeTruthy();
    // No thread is open on the conversation list, so nothing to add someone to.
    expect(queryByTestId('quick-action-add-participant', hidden)).toBeNull();

    fireEvent.press(getByTestId('quick-action-new-chat', hidden));
    expect(mockRouter.push).toHaveBeenCalledWith({
      pathname: CHAT_THREAD_ROUTE,
      params: { conversationId: 'new' },
    });
  });

  it('offers "add someone to this discussion" only while a thread is open', () => {
    mockSegments = ['(app)', '(tabs)', '(chat)', '[conversationId]'];
    mockGlobalParams = { conversationId: 'conv-1' };
    const { getByTestId } = render(<ExpandableTabBar />);
    expect(getByTestId('quick-action-add-participant', { includeHiddenElements: true })).toBeTruthy();
  });
});
