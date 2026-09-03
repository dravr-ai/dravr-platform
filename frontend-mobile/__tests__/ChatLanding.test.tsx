// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Asserts the app lands on the chat tab once onboarding is complete, and that tab is the conversation list
// ABOUTME: Drives the root layout's route guard with a finished onboarding context and renders the (chat) index route

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { USER_SURFACES } from '@pierre/shared-constants';

const mockRouter = {
  push: jest.fn(),
  replace: jest.fn(),
  back: jest.fn(),
  navigate: jest.fn(),
  canGoBack: () => true,
};
let mockSegments: string[] = ['(auth)', 'login'];

jest.mock('expo-router', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    useRouter: () => mockRouter,
    useSegments: () => mockSegments,
    useLocalSearchParams: () => ({}),
    useGlobalSearchParams: () => ({}),
    useFocusEffect: (cb: () => void | (() => void)) => {
      React.useEffect(() => cb(), [cb]);
    },
    useNavigationContainerRef: () => ({
      addListener: () => () => {},
      isReady: () => false,
      getCurrentRoute: () => undefined,
    }),
    Slot: () => React.createElement(View, { testID: 'slot' }),
    Stack: Object.assign(
      ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
      { Screen: () => null },
    ),
    Tabs: Object.assign(
      ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
      { Screen: () => null },
    ),
  };
});

jest.mock('expo-splash-screen', () => ({
  preventAutoHideAsync: jest.fn(),
  hideAsync: jest.fn(),
}));
jest.mock('@expo-google-fonts/space-grotesk', () => ({
  useFonts: () => [true],
  SpaceGrotesk_400Regular: 'font',
  SpaceGrotesk_500Medium: 'font',
  SpaceGrotesk_600SemiBold: 'font',
  SpaceGrotesk_700Bold: 'font',
}));
jest.mock('@expo-google-fonts/plus-jakarta-sans', () => ({
  PlusJakartaSans_400Regular: 'font',
  PlusJakartaSans_500Medium: 'font',
  PlusJakartaSans_600SemiBold: 'font',
  PlusJakartaSans_700Bold: 'font',
}));
jest.mock('@expo-google-fonts/inter', () => ({
  Inter_400Regular: 'font',
  Inter_500Medium: 'font',
  Inter_600SemiBold: 'font',
}));

jest.mock('../src/contexts/AuthContext', () => ({
  AuthProvider: ({ children }: { children: React.ReactNode }) => children,
  useAuth: () => ({
    isAuthenticated: true,
    isLoading: false,
    user: { id: 'user-1', email: 'athlete@dravr.ai', user_status: 'active', role: 'user', analytics_consent: false },
  }),
}));
jest.mock('../src/providers/QueryProvider', () => ({
  QueryProvider: ({ children }: { children: React.ReactNode }) => children,
}));
// An athlete who has finished every onboarding step: a provider connected,
// the pre-connect questions answered, the coach proposal seen, and a tenant
// with no messaging channel to pick or link.
jest.mock('../src/hooks/useOnboardingStatus', () => ({
  useOnboardingStatus: () => ({ data: { needs_provider_connection: false } }),
}));
jest.mock('../src/hooks/useCoachProposalSeen', () => ({ useCoachProposalSeen: () => ({ seen: true }) }));
jest.mock('../src/hooks/useProfileTypeChosen', () => ({ useProfileTypeChosen: () => ({ chosen: true }) }));
jest.mock('../src/hooks/useOnboardingFlag', () => ({ useOnboardingFlag: () => ({ done: true }) }));
jest.mock('../src/hooks/useProviderSkipped', () => ({ useProviderSkipped: () => ({ skipped: false }) }));
jest.mock('../src/hooks/useMessagingOnboarding', () => ({
  useMessagingOnboarding: () => ({
    loading: false,
    availableCount: 0,
    channelChosen: false,
    channelDone: false,
    configureDone: false,
  }),
}));
jest.mock('../src/services/analytics', () => ({
  trackMobile: jest.fn(),
  bootMobileAnalytics: jest.fn(),
  shutdownMobileAnalytics: jest.fn(),
}));
jest.mock('../src/i18n/localePersister', () => ({ persistLocale: jest.fn() }));

const mockGetConversations = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    updateConversation: jest.fn(),
    deleteConversation: jest.fn(),
    createConversation: jest.fn(),
    listParticipants: jest.fn(),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
  },
  coachesApi: { list: jest.fn().mockResolvedValue({ coaches: [] }) },
  notificationsApi: { getUnreadCount: jest.fn().mockResolvedValue({ unread_count: 0 }) },
  // The root layout hands this to initI18n at import time, so a mock of this
  // barrel has to carry it — the live catalogue overlay is part of booting.
  i18nApi: { bundle: jest.fn().mockResolvedValue({ status: 'unchanged' }) },
}));

import RootLayout from '../app/_layout';
import ChatIndexRoute from '../app/(app)/(tabs)/(chat)/index';
import { ConversationsScreen } from '../src/screens/conversations/ConversationsScreen';
import { TAB_BAR_TABS } from '../src/components/ui/ExpandableTabBar';
import { CHAT_LIST_ROUTE } from '../src/navigation/routes';

describe('chat-first landing', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSegments = ['(auth)', 'login'];
  });

  // Turns red if the route guard sends a fully onboarded athlete anywhere but
  // the chat tab — the "we land on that icon/page" clause of the cutover.
  it('replaces the auth route with the chat tab once onboarding is complete', async () => {
    render(<RootLayout />);

    await waitFor(() => expect(mockRouter.replace).toHaveBeenCalledWith('/(app)/(tabs)/(chat)'));
    expect(mockRouter.replace).toHaveBeenCalledTimes(1);
    expect(mockRouter.replace).toHaveBeenCalledWith(CHAT_LIST_ROUTE);
  });

  it('leaves an athlete already in the app where they are', async () => {
    mockSegments = ['(app)', '(tabs)', '(discover)'];
    render(<RootLayout />);

    // The guard settles synchronously in its effect; give it a tick to run.
    await waitFor(() => expect(mockRouter.replace).not.toHaveBeenCalled());
  });

  // Turns red if the chat tab's index stops being the conversation list — a
  // deep link to /(app)/(tabs)/(chat) must open the Telegram-shaped list,
  // never a composer.
  it('serves the conversation list at the chat tab index', async () => {
    expect(ChatIndexRoute).toBe(ConversationsScreen);
    mockGetConversations.mockResolvedValue({
      conversations: [
        {
          id: 'conv-1',
          title: 'Tempo Tuesday',
          coach_id: null,
          message_count: 3,
          unread_count: 0,
          created_at: '2026-08-20T10:00:00Z',
          updated_at: '2026-08-25T10:00:00Z',
        },
      ],
      total: 1,
      limit: 50,
      offset: 0,
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });

    const { findByTestId, getByTestId, getByText } = render(
      <QueryClientProvider client={client}>
        <ChatIndexRoute />
      </QueryClientProvider>,
    );

    expect(await findByTestId('conversation-row-conv-1')).toBeTruthy();
    expect(getByText('Tempo Tuesday')).toBeTruthy();
    expect(getByTestId('conversations-screen')).toBeTruthy();
    expect(getByTestId('chat-plus-button')).toBeTruthy();
    expect(mockGetConversations).toHaveBeenCalledTimes(1);
  });

  it('keeps the registry, the tab bar and the landing route on one path', () => {
    expect(USER_SURFACES.find((surface) => surface.id === 'chat')?.mobile).toBe(CHAT_LIST_ROUTE);
    expect(CHAT_LIST_ROUTE).toBe(`/(app)/(tabs)/${TAB_BAR_TABS[0].route}`);
  });
});
