// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the floating tab bar — the tab set it renders, the tabs layout, and the route groups on disk
// ABOUTME: Pins the three copies of the tab list to each other so a tab removed from one cannot linger in another

import React from 'react';
import fs from 'fs';
import path from 'path';
import { fireEvent, render as rtlRender } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

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

// The shared setup's reanimated stand-in builds a fresh shared value on every
// render, so every value the bar mutates is back at its initial by the time the
// tree is read. The pill's height is exactly what this suite measures, so here
// a shared value has to survive a render.
jest.mock('react-native-reanimated', () => {
  const React = require('react');
  const { View } = require('react-native');
  const layoutAnimation: Record<string, () => unknown> = {
    duration: () => layoutAnimation,
    delay: () => layoutAnimation,
  };
  return {
    __esModule: true,
    default: {
      View: React.forwardRef(
        ({ style, ...props }: { style?: unknown; children?: React.ReactNode }, ref: unknown) =>
          React.createElement(View, { ...props, ref, style }, props.children),
      ),
      createAnimatedComponent: (Component: React.ComponentType) =>
        React.forwardRef((props: object, ref: unknown) =>
          React.createElement(Component, { ...props, ref }),
        ),
    },
    useSharedValue: (initial: unknown) => {
      const ref = React.useRef(null);
      if (ref.current === null) ref.current = { value: initial };
      return ref.current;
    },
    useAnimatedStyle: (fn: () => unknown) => fn(),
    withSpring: (toValue: unknown) => toValue,
    withTiming: (toValue: unknown) => toValue,
    withSequence: (...animations: unknown[]) => animations[animations.length - 1],
    FadeInDown: layoutAnimation,
  };
});

jest.mock('../src/hooks/useServerStatus', () => ({
  useServerStatus: () => ({ isServerReachable: true, isChecking: false, checkNow: jest.fn() }),
}));
jest.mock('../src/components/ServerStatusBanner', () => ({ ServerStatusBanner: () => null }));
const mockGetConversations = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    createConversation: jest.fn(),
    listParticipants: jest.fn().mockResolvedValue([]),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
  },
}));

/** The bar reads the conversation list's cache for the chat badge. */
function render(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

import {
  expandedSheetHeight,
  ExpandableTabBar,
  TAB_BAR_TABS,
} from '../src/components/ui/ExpandableTabBar';
import { MENU_ROW_HEIGHT } from '../src/components/ui/TabMenuItem';
import TabsLayout from '../app/(app)/(tabs)/_layout';
import { CHAT_LIST_ROUTE, CHAT_THREAD_ROUTE } from '../src/navigation/routes';

/** The route-group directories expo-router turns into tabs. */
const TABS_DIR = path.join(__dirname, '..', 'app', '(app)', '(tabs)');

/** The testIDs of the collapsed pill's tab buttons, in render order. */
function renderedTabIds(getAllByTestId: (id: RegExp) => Array<{ props: { testID: string } }>): string[] {
  return getAllByTestId(/^tab-[a-z]+$/).map((node) => node.props.testID);
}

/** The pill's rendered height in points, sheet open or shut. */
function pillHeight(getByTestId: (id: string) => { props: { style: unknown } }): number {
  const style = getByTestId('expandable-tab-bar-pill').props.style as Array<Record<string, unknown>>;
  const flat = Object.assign({}, ...style) as { height?: unknown };
  if (typeof flat.height !== 'number') {
    throw new Error(`pill rendered a non-numeric height: ${String(flat.height)}`);
  }
  return flat.height;
}

describe('ExpandableTabBar', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSegments = ['(app)', '(tabs)', '(chat)'];
    mockGlobalParams = {};
    mockGetConversations.mockResolvedValue({ conversations: [], total: 0, limit: 50, offset: 0 });
  });

  it('puts Chat first, where the app lands, and lists no Coaches or Groups tab', () => {
    expect(TAB_BAR_TABS.map((tab) => tab.route)).toEqual(['(chat)', '(discover)', '(settings)']);
    expect(CHAT_LIST_ROUTE).toBe(`/(app)/(tabs)/${TAB_BAR_TABS[0].route}`);
    expect(TAB_BAR_TABS.map((tab) => tab.testID)).not.toContain('tab-coaches');
    expect(TAB_BAR_TABS.map((tab) => tab.testID)).not.toContain('tab-groups');
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

    expect(rendered).toEqual(['tab-chat', 'tab-discover', 'tab-settings']);
    expect(registered).toEqual(['(chat)', '(discover)', '(settings)']);
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
    // Group management moved into the group's own chat thread; the route
    // group is gone, so no deep link can land on a tab that does not exist.
    expect(groupsOnDisk).not.toContain('(groups)');
  });

  // The expanded menu sits behind the animated pill (display: none until the
  // "+" opens it), so its rows are queried with hidden elements included.
  it('renders no Coaches or Groups tab, menu item or quick action', () => {
    const { queryByTestId } = render(<ExpandableTabBar />);
    expect(queryByTestId('tab-coaches')).toBeNull();
    expect(queryByTestId('tab-groups')).toBeNull();
    expect(queryByTestId('tab-menu-item-(coaches)', { includeHiddenElements: true })).toBeNull();
    expect(queryByTestId('tab-menu-item-(groups)', { includeHiddenElements: true })).toBeNull();
    expect(queryByTestId('quick-action-new-coach', { includeHiddenElements: true })).toBeNull();
  });

  // The badge and the list read one query, so a row read elsewhere clears
  // the pill too. Turns red if the bar starts counting for itself.
  it('wears the unread total of the conversation list on the chat tab', async () => {
    mockGetConversations.mockResolvedValue({
      conversations: [
        { id: 'c1', title: 'A', coach_id: null, message_count: 4, unread_count: 3, created_at: '2026-08-20T10:00:00Z', updated_at: '2026-08-26T10:00:00Z' },
        { id: 'c2', title: 'B', coach_id: null, message_count: 2, unread_count: 2, created_at: '2026-08-20T10:00:00Z', updated_at: '2026-08-26T10:00:00Z' },
      ],
      total: 2,
      limit: 50,
      offset: 0,
    });
    const { findByTestId, queryByTestId } = render(<ExpandableTabBar />);
    expect(await findByTestId('tab-chat-badge')).toHaveTextContent('5');
    expect(queryByTestId('tab-discover-badge')).toBeNull();
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

  // The sheet used to repeat the three destinations that sit in the row
  // directly beneath it — same icons, same order — so most of it was a second
  // copy of the navigation the athlete was already looking at (carnet#209).
  it('holds actions only: no destination row, no destination label', () => {
    const hidden = { includeHiddenElements: true };
    const { getByLabelText, queryByTestId, queryByText, getAllByTestId } = render(<ExpandableTabBar />);
    fireEvent.press(getByLabelText('Open menu'));

    for (const tab of TAB_BAR_TABS) {
      expect(queryByTestId(`tab-menu-item-${tab.route}`, hidden)).toBeNull();
    }
    expect(queryByText('Chat', hidden)).toBeNull();
    expect(queryByText('Discover', hidden)).toBeNull();
    expect(queryByText('Settings', hidden)).toBeNull();

    const rows = getAllByTestId(/^quick-action-/, hidden).map((node) => node.props.testID);
    expect(rows).toEqual(['quick-action-new-chat', 'quick-action-new-group-chat']);
  });

  it('holds actions only with a thread open, where the sheet has three', () => {
    mockSegments = ['(app)', '(tabs)', '(chat)', '[conversationId]'];
    mockGlobalParams = { conversationId: 'conv-1' };
    const hidden = { includeHiddenElements: true };
    const { getByLabelText, queryByTestId, queryByText, getAllByTestId } = render(<ExpandableTabBar />);
    fireEvent.press(getByLabelText('Open menu'));

    for (const tab of TAB_BAR_TABS) {
      expect(queryByTestId(`tab-menu-item-${tab.route}`, hidden)).toBeNull();
    }
    expect(queryByText('Chat', hidden)).toBeNull();
    expect(queryByText('Discover', hidden)).toBeNull();
    expect(queryByText('Settings', hidden)).toBeNull();

    const rows = getAllByTestId(/^quick-action-/, hidden).map((node) => node.props.testID);
    expect(rows).toEqual([
      'quick-action-new-chat',
      'quick-action-new-group-chat',
      'quick-action-add-participant',
    ]);
  });

  // A fixed 380pt opened the same sheet whatever it held, so it was mostly
  // empty — and emptier still once the destinations left it (carnet#209).
  it('opens to the height of the rows it holds, and shuts back to the pill', () => {
    const { getByLabelText, getByTestId } = render(<ExpandableTabBar />);
    expect(pillHeight(getByTestId)).toBe(56);

    fireEvent.press(getByLabelText('Open menu'));
    // 56 collapsed row + 12 padding above + 12 below + two 48pt rows.
    expect(pillHeight(getByTestId)).toBe(56 + 12 + 12 + 2 * MENU_ROW_HEIGHT);
    expect(pillHeight(getByTestId)).toBe(expandedSheetHeight(2));

    fireEvent.press(getByLabelText('Close menu'));
    expect(pillHeight(getByTestId)).toBe(56);
  });

  it('keeps the destinations lit while the sheet above them is open', () => {
    const { getByLabelText, getByTestId } = render(<ExpandableTabBar />);
    const opacityOf = () => {
      const style = getByTestId('tab-bar-destinations').props.style as
        | Record<string, unknown>
        | Array<Record<string, unknown>>;
      const flat = Array.isArray(style) ? Object.assign({}, ...style) : style;
      return (flat as { opacity?: number }).opacity ?? 1;
    };

    expect(opacityOf()).toBe(1);

    // The row used to fade out here while still holding its 56pt, which left a
    // blank band under the actions once the sheet stopped repeating the
    // destinations (carnet#209).
    fireEvent.press(getByLabelText('Open menu'));
    expect(opacityOf()).toBe(1);
    expect(getByTestId('tab-bar-destinations').props.style).toBeDefined();
  });

  it('opens taller for the third action a thread adds', () => {
    mockSegments = ['(app)', '(tabs)', '(chat)', '[conversationId]'];
    mockGlobalParams = { conversationId: 'conv-1' };
    const { getByLabelText, getByTestId } = render(<ExpandableTabBar />);

    fireEvent.press(getByLabelText('Open menu'));
    expect(pillHeight(getByTestId)).toBe(expandedSheetHeight(3));
    expect(expandedSheetHeight(3) - expandedSheetHeight(2)).toBe(MENU_ROW_HEIGHT);
    expect(pillHeight(getByTestId)).toBe(56 + 12 + 12 + 3 * MENU_ROW_HEIGHT);
  });
});
