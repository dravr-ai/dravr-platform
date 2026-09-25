// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Dravr lockup lives on the Home and chat tab headers and nowhere else on the phone, and a press on it goes Home
// ABOUTME: Pins the mark, the wordmark and its lockup type spec in both schemes, the button it becomes, and the absence on other tabs

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PRODUCT_WORDMARK } from '@pierre/shared-constants';
import { BOREAL_LIGHT, BOREAL_DARK } from '@pierre/shared-constants';

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted preference below, which is what the app really reads.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
  }),
);

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
import { ChatHeaderTitle } from '../src/screens/chat/ChatHeaderTitle';
import { HOME_ROUTE } from '../src/navigation/routes';

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
    mockBrowseStore.mockResolvedValue({ agents: [], total: 0 });
    mockListCoaches.mockResolvedValue({ agents: [] });
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

      // It is the way Home, and says so to assistive tech.
      expect(screen.getByTestId('conversations-title').props.accessibilityLabel).toBe('Home');
      expect(screen.getByTestId('conversations-title').props.accessibilityRole).toBe('button');
    });

    it('goes Home when pressed', async () => {
      const screen = renderInTheme(<ConversationsScreen />);
      await waitFor(() => expect(screen.getByTestId('conversations-title')).toBeTruthy());

      fireEvent.press(screen.getByTestId('conversations-title'));

      expect(mockRouter.navigate).toHaveBeenCalledWith(HOME_ROUTE);
      expect(HOME_ROUTE).toBe('/(app)/(tabs)/(home)');
      // A press on the brand is navigation, never a new thread.
      expect(mockRouter.push).not.toHaveBeenCalled();
    });

    it('carries the one "+" of the app, trailing in the native header', async () => {
      const screen = renderInTheme(<ConversationsScreen />);
      await waitFor(() => expect(screen.getByTestId('conversations-title')).toBeTruthy());

      // carnet#213 took the "+" out of the THREAD header; the tab bar's "+"
      // then left with the tab bar (Boreal v2.2 D1). The header button is the
      // app's one entry point for starting something.
      expect(screen.getByTestId('new-chat-button')).toBeTruthy();
      expect(screen.queryByTestId('chat-plus-button')).toBeNull();

      // The empty state's ink link is a call to action, not chrome, and stays.
      expect(screen.getByTestId('conversations-empty-start')).toBeTruthy();
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
    it('sets Schibsted Grotesk 600 at 0.15em in the primary ink — dark', async () => {
      await AsyncStorage.setItem(APPEARANCE_KEY, 'dark');
      const screen = renderInTheme(<BrandLockup size={28} />);

      const wordmark = await waitFor(() => screen.getByTestId('brand-lockup-wordmark'));
      const style = wordmark.props.style as {
        fontFamily: string;
        fontSize: number;
        letterSpacing: number;
        color: string;
      };

      expect(style.fontFamily).toBe('SchibstedGrotesk');
      expect(style.letterSpacing).toBeCloseTo(style.fontSize * BRAND_TRACKING_RATIO, 5);
      expect(style.color).toBe(BOREAL_DARK.primary);
      expect(style.color).toBe('#a3d0be');
    });

    it('sets the same spec in the light scheme, in light-mode primary ink', async () => {
      await AsyncStorage.setItem(APPEARANCE_KEY, 'light');
      const screen = renderInTheme(<BrandLockup size={28} />);

      const wordmark = await waitFor(() => {
        const node = screen.getByTestId('brand-lockup-wordmark');
        const nodeStyle = node.props.style as { color: string };
        // The persisted preference lands one frame after mount.
        expect(nodeStyle.color).toBe(BOREAL_LIGHT.primary);
        return node;
      });
      const style = wordmark.props.style as {
        fontFamily: string;
        fontSize: number;
        letterSpacing: number;
        color: string;
      };

      expect(style.fontFamily).toBe('SchibstedGrotesk');
      expect(style.letterSpacing).toBeCloseTo(style.fontSize * BRAND_TRACKING_RATIO, 5);
      // Sage-forest: the v1 primary (#00241a) read as black at this size, so
      // the green lived in a separate `brand` ink; v2 promoted that ink to
      // `primary`, and the wordmark reads the one token.
      expect(style.color).toBe('#255f4d');
      expect(style.color).toBe(BOREAL_LIGHT.primary);
    });
  });

  describe('the pressable lockup', () => {
    it('keeps the look of the heading lockup: same mark, same wordmark, no badge', async () => {
      const onPress = jest.fn();
      const screen = renderInTheme(
        <BrandLockup size={28} onPress={onPress} accessibilityLabel="Home" testID="pressable-lockup" />,
      );

      const wordmark = await waitFor(() => screen.getByTestId('pressable-lockup-wordmark'));
      expect(wordmark).toHaveTextContent(PRODUCT_WORDMARK);
      expect((wordmark.props.style as { fontFamily: string }).fontFamily).toBe('SchibstedGrotesk');
      expect(screen.getByTestId('pressable-lockup-mark').props.source).toBeDefined();

      fireEvent.press(screen.getByTestId('pressable-lockup'));
      expect(onPress).toHaveBeenCalledTimes(1);
    });

    it('reaches the 44 pt thumb floor through its hit slop', () => {
      const screen = renderInTheme(<BrandLockup size={28} onPress={jest.fn()} testID="pressable-lockup" />);

      const slop = screen.getByTestId('pressable-lockup').props.hitSlop as { top: number; bottom: number };
      expect(28 + slop.top + slop.bottom).toBe(44);
    });

    it('stays a heading when nothing is pressed through it', () => {
      const screen = renderInTheme(<BrandLockup size={28} />);

      expect(screen.getByTestId('brand-lockup').props.accessibilityRole).toBe('header');
    });
  });

  describe('every other surface', () => {
    it('leaves the Discover tab with its own title and no wordmark', async () => {
      const screen = renderInTheme(<StoreScreen />);

      await waitFor(() => expect(screen.getByTestId('store-screen')).toBeTruthy());
      expect(screen.queryByText(PRODUCT_WORDMARK)).toBeNull();
      expect(screen.queryByTestId('brand-lockup')).toBeNull();
      // Its title is the native large title the discover layout sets; the
      // screen configures only the header's search field.
      expect(screen.queryByTestId('stack-header-title-view')).toBeNull();
      expect(screen.getByTestId('header-search-input')).toBeTruthy();
    });

    it('leaves an open thread showing the thread, not the brand', async () => {
      const screen = renderInTheme(
        <ChatHeaderTitle currentConversation={null} providerStatus={null} onTitlePress={jest.fn()} />,
      );

      expect(screen.queryByText(PRODUCT_WORDMARK)).toBeNull();
      expect(screen.queryByTestId('brand-lockup')).toBeNull();
      expect(screen.getByTestId('chat-title')).toBeTruthy();
    });
  });
});
