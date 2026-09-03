// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Dravr lockup lives on the chat tab's header and nowhere else on the phone
// ABOUTME: Pins the mark, the wordmark and its lockup type spec in both schemes, and the absence on other tabs

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PRODUCT_WORDMARK } from '@pierre/shared-constants';
import { BOREAL_LIGHT, BOREAL_DARK } from '../src/constants/theme';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted preference below, which is what the app really reads.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({}),
  useFocusEffect: (cb: () => void) => {
    const React = require('react');
    React.useEffect(() => cb(), [cb]);
  },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

const mockGetConversations = jest.fn();
const mockBrowseStore = jest.fn();
const mockListCoaches = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    updateConversation: jest.fn(),
    deleteConversation: jest.fn(),
    markConversationRead: jest.fn(),
    markConversationUnread: jest.fn(),
    listParticipants: jest.fn().mockResolvedValue([]),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
  },
  notificationsApi: { getUnreadCount: jest.fn().mockResolvedValue({ unread_count: 0 }) },
  storeApi: {
    browse: (...args: unknown[]) => mockBrowseStore(...args),
    search: jest.fn(),
  },
  coachesApi: { list: (...args: unknown[]) => mockListCoaches(...args) },
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider } from '../src/contexts/ThemeContext';
import { BrandLockup } from '../src/components/ui/BrandLockup';
import { ConversationsScreen } from '../src/screens/conversations/ConversationsScreen';
import { StoreScreen } from '../src/screens/store/StoreScreen';
import { ChatHeader } from '../src/screens/chat/ChatHeader';

const APPEARANCE_KEY = 'pierre.appearance_pref';

/** DESIGN.md §1: the wordmark is tracked at 0.15em of its own type size. */
const BRAND_TRACKING_RATIO = 0.15;

function renderInTheme(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ThemeProvider>{ui}</ThemeProvider>
    </QueryClientProvider>,
  );
}

describe('the Dravr lockup on the phone', () => {
  beforeEach(async () => {
    jest.clearAllMocks();
    mockGetConversations.mockResolvedValue({ conversations: [], total: 0 });
    mockBrowseStore.mockResolvedValue({ coaches: [], total: 0 });
    mockListCoaches.mockResolvedValue({ coaches: [] });
    await AsyncStorage.setItem(APPEARANCE_KEY, 'dark');
  });

  describe('the chat tab header', () => {
    it('carries the mark AND the wordmark, in place of the screen title', async () => {
      const screen = renderInTheme(<ConversationsScreen />);

      await waitFor(() => expect(screen.getByTestId('conversations-title')).toBeTruthy());

      // Mark and name together — the directive is "mark PLUS name".
      expect(screen.getByTestId('conversations-title-mark')).toBeTruthy();
      expect(screen.getByTestId('conversations-title-wordmark')).toHaveTextContent(PRODUCT_WORDMARK);

      // The lockup REPLACES the title: the header carries no separate one.
      expect(screen.queryByText('Chats')).toBeNull();
      expect(screen.queryByText('Discussions')).toBeNull();

      // The destination keeps a spoken name for assistive tech.
      expect(screen.getByTestId('conversations-title').props.accessibilityLabel).toBe('Chats');
      expect(screen.getByTestId('conversations-title').props.accessibilityRole).toBe('header');
    });

    it('draws the mark from the shipped badge asset', async () => {
      const screen = renderInTheme(<ConversationsScreen />);
      await waitFor(() => expect(screen.getByTestId('conversations-title-mark')).toBeTruthy());

      // require() of a PNG resolves to a registered asset id under jest; what
      // matters is that a source was handed to the Image at all.
      expect(screen.getByTestId('conversations-title-mark').props.source).toBeDefined();
    });
  });

  describe('the lockup type spec (DESIGN.md §1 / §3)', () => {
    it('sets Space Grotesk 600 at 0.15em in the brand ink — dark', async () => {
      await AsyncStorage.setItem(APPEARANCE_KEY, 'dark');
      const screen = renderInTheme(<BrandLockup size={28} />);

      const wordmark = await waitFor(() => screen.getByTestId('brand-lockup-wordmark'));
      const style = wordmark.props.style as {
        fontFamily: string;
        fontSize: number;
        letterSpacing: number;
        color: string;
      };

      expect(style.fontFamily).toBe('SpaceGrotesk_SemiBold');
      expect(style.letterSpacing).toBeCloseTo(style.fontSize * BRAND_TRACKING_RATIO, 5);
      expect(style.color).toBe(BOREAL_DARK.brand);
      expect(style.color).toBe('#a3d0be');
    });

    it('sets the same spec in the light scheme, in light-mode brand ink', async () => {
      await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
      const screen = renderInTheme(<BrandLockup size={28} />);

      const wordmark = await waitFor(() => {
        const node = screen.getByTestId('brand-lockup-wordmark');
        const nodeStyle = node.props.style as { color: string };
        // The persisted preference lands one frame after mount.
        expect(nodeStyle.color).toBe(BOREAL_LIGHT.brand);
        return node;
      });
      const style = wordmark.props.style as {
        fontFamily: string;
        fontSize: number;
        letterSpacing: number;
        color: string;
      };

      expect(style.fontFamily).toBe('SpaceGrotesk_SemiBold');
      expect(style.letterSpacing).toBeCloseTo(style.fontSize * BRAND_TRACKING_RATIO, 5);
      // Not `primary` (#00241a), which reads as black at this size — that is
      // the whole reason the brand ink exists.
      expect(style.color).toBe('#255f4d');
      expect(style.color).not.toBe(BOREAL_LIGHT.primary);
    });
  });

  describe('every other surface', () => {
    it('leaves the Discover tab with its own title and no wordmark', async () => {
      const screen = renderInTheme(<StoreScreen />);

      await waitFor(() => expect(screen.getByTestId('store-screen')).toBeTruthy());
      expect(screen.queryByText(PRODUCT_WORDMARK)).toBeNull();
      expect(screen.queryByTestId('brand-lockup')).toBeNull();
      // Its own screen title stays.
      expect(screen.getByText('Discover')).toBeTruthy();
    });

    it('leaves an open thread showing the thread, not the brand', async () => {
      const screen = renderInTheme(
        <ChatHeader
          currentConversation={null}
          insetTop={0}
          providerStatus={null}
          onBackPress={jest.fn()}
          onTitlePress={jest.fn()}
        />,
      );

      expect(screen.queryByText(PRODUCT_WORDMARK)).toBeNull();
      expect(screen.queryByTestId('brand-lockup')).toBeNull();
      expect(screen.getByTestId('chat-title')).toBeTruthy();
    });
  });
});
