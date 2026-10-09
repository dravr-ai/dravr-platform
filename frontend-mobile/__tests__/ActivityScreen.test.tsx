// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One activity's screen over mocked reads — its title, map, figures and splits, then the chat about it underneath
// ABOUTME: Pins the figures, every question answered in the same screen, one thread per activity reopened on return, and the honest states

import React from 'react';
import { Alert, type AlertButton } from 'react-native';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityDetailResponse, ActivityRouteResponse } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';

import { LATEST_ROUTE_RESPONSE, TEMPO_DETAIL_RESPONSE } from '../integration/app/helpers/homeFixtures';

const mockPush = jest.fn();
const mockBack = jest.fn();
let mockParams: { provider?: string; activityId?: string } = { provider: 'strava', activityId: '9000' };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: mockBack, navigate: jest.fn(), canGoBack: () => true }),
    useLocalSearchParams: () => mockParams,
  }),
);
// No navigator under a unit test, so the header the column offsets by is 0 tall.
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => 0,
}));
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});
jest.mock('expo-linking', () => ({ openURL: jest.fn() }));
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));

const mockGetActivityDetail = jest.fn<Promise<ActivityDetailResponse>, [string, string]>();
const mockGetActivityRoute = jest.fn<Promise<ActivityRouteResponse>, [string, string]>();
const mockCreateConversation = jest.fn();
const mockGetConversationMessages = jest.fn();
const mockSendTurn = jest.fn();
const mockLinkActivityConversation = jest.fn<Promise<void>, [string, string, string | null]>();
const mockDeleteUploadedActivity = jest.fn<Promise<void>, [string]>();
jest.mock('../src/services/api', () => ({
  athleteApi: {
    getActivityDetail: (provider: string, id: string) => mockGetActivityDetail(provider, id),
    getActivityRoute: (provider: string, id: string) => mockGetActivityRoute(provider, id),
    linkActivityConversation: (provider: string, id: string, conversationId: string | null) =>
      mockLinkActivityConversation(provider, id, conversationId),
    deleteUploadedActivity: (id: string) => mockDeleteUploadedActivity(id),
  },
  chatApi: {
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
    getConversations: () => Promise.resolve({ conversations: [] }),
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    getConversationVerdicts: () => Promise.resolve({ verdicts: [] }),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    submitMessageFeedback: jest.fn(),
    deleteMessageFeedback: jest.fn(),
  },
  oauthApi: {},
  coachesApi: {},
  groupsApi: {},
  notificationsApi: {},
}));

// The thread's own state — messages, conversations — is the real hooks'. The
// provider, usage and voice hooks hand back the same state every render.
jest.mock('../src/screens/chat/useProviderStatus', () => {
  const state = {
    connectedProviders: [],
    providersLoaded: true,
    selectedProvider: null,
    connectingProvider: null,
    needsCredentialsProvider: null,
    error: null,
    hasConnectedProvider: true,
    loadProviderStatus: jest.fn(),
    setSelectedProvider: jest.fn(),
    setNeedsCredentialsProvider: jest.fn(),
    handleConnectProvider: jest.fn(),
  };
  return { useProviderStatus: () => state };
});
jest.mock('../src/screens/chat/useUsageStatus', () => {
  const state = {
    data: null,
    isLoading: false,
    level: null,
    message: null,
    sendDisabled: false,
    invalidate: jest.fn(),
    applyNotice: jest.fn(),
  };
  return { useUsageStatus: () => state };
});
jest.mock('../src/screens/chat/useChatVoiceInput', () => {
  const state = { isListening: false, isAvailable: false, partialTranscript: '', handleVoicePress: jest.fn() };
  return { useChatVoiceInput: () => state };
});

import { ActivityScreen } from '../src/screens/activity/ActivityScreen';

const TEMPO_DAY = 'Thursday, September 17';
const THREAD = 'conv-tempo';
const REPLY = 'Even splits: the second kilometre was your fastest.';

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
}

function renderScreen(client = newClient()) {
  const view = render(
    <QueryClientProvider client={client}>
      <ActivityScreen />
    </QueryClientProvider>,
  );
  return Object.assign(view, { client });
}

/** Answer every turn the way the transport does: the stored question, then the reply. */
function answerTurns() {
  let n = 0;
  mockSendTurn.mockImplementation(
    (conversationId: string, content: string, options: { onDone?: (turn: unknown) => void }) => {
      n += 1;
      options.onDone?.({
        user_message: { id: `u-${n}`, conversation_id: conversationId, role: 'user', content, created_at: '2026-09-30T10:00:00Z' },
        assistant: {
          message: { id: `a-${n}`, conversation_id: conversationId, role: 'assistant', content: REPLY, created_at: '2026-09-30T10:00:02Z' },
          blocks: [],
          finish_reason: 'stop',
        },
        telemetry: { model: 'm', provider_name: 'p', tool_calls_count: 0, tools_called: [], execution_time_ms: 1 },
      });
      return Promise.resolve();
    },
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  mockParams = { provider: 'strava', activityId: '9000' };
  // The server's link: what the screen links, a later detail read names.
  const links = new Map<string, string | null>();
  mockLinkActivityConversation.mockImplementation(async (provider, id, conversationId) => {
    links.set(`${provider}/${id}`, conversationId);
  });
  mockGetActivityDetail.mockImplementation(async (provider, id) => ({
    ...TEMPO_DETAIL_RESPONSE,
    conversation_id: links.get(`${provider}/${id}`) ?? null,
  }));
  mockGetActivityRoute.mockResolvedValue(LATEST_ROUTE_RESPONSE);
  mockCreateConversation.mockResolvedValue({ id: THREAD, title: 'Sep 30', created_at: '2026-09-30T10:00:00Z', updated_at: '2026-09-30T10:00:00Z' });
  mockGetConversationMessages.mockResolvedValue({ messages: [] });
  answerTurns();
});

describe('ActivityScreen', () => {
  it('shows the activity: its title, when, the map, the figures and the splits', async () => {
    const screen = renderScreen();

    const figures = await screen.findByTestId('activity-figures');
    expect(mockGetActivityDetail).toHaveBeenCalledWith('strava', '9000');
    expect(screen.getByTestId('stack-header-title')).toHaveTextContent('Tempo Thursday');
    expect(screen.getByTestId('activity-when')).toHaveTextContent(/Thu, Sep 17 · Run/);
    expect(await screen.findByTestId('activity-map')).toBeTruthy();
    expect(mockGetActivityRoute).toHaveBeenCalledWith('strava', '9000');

    const figure = (id: string) => within(figures).getByTestId(`activity-figure-${id}`);
    expect(figure('distance')).toHaveTextContent('Distance10.20 km');
    expect(figure('duration')).toHaveTextContent('Duration50m 12s');
    expect(figure('average_speed')).toHaveTextContent('Avg pace4:55 /km');
    expect(figure('average_heart_rate')).toHaveTextContent('Avg heart rate158 bpm');
    expect(figure('calories')).toHaveTextContent('Calories712 kcal');
    // Not held by the cache, and a pace sport prints no top speed.
    expect(within(figures).queryByTestId('activity-figure-average_power')).toBeNull();
    expect(within(figures).queryByTestId('activity-figure-max_speed')).toBeNull();

    expect(screen.getByTestId('activity-splits-row-1')).toHaveTextContent('11.00 km4:584:58 /km151 bpm+6 m');
    expect(screen.getByTestId('activity-splits-row-2')).toHaveTextContent('21.00 km4:504:50 /km160 bpm-4 m');
    expect(screen.queryByTestId('activity-laps')).toBeNull();
    // The chat is under the activity: its questions, and the composer at the foot.
    expect(screen.getByTestId('activity-prompts')).toBeTruthy();
    expect(screen.getByTestId('message-input')).toBeTruthy();
  });

  it('shows the Garmin attribution a Garmin-recorded activity carries, and none otherwise', async () => {
    mockGetActivityDetail.mockResolvedValue({
      ...TEMPO_DETAIL_RESPONSE,
      activity: { ...TEMPO_DETAIL_RESPONSE.activity, provider: 'intervals_icu', attribution: 'Garmin Forerunner 965' },
    });
    const screen = renderScreen();
    expect(await screen.findByTestId('activity-attribution')).toHaveTextContent('· Garmin Forerunner 965');
    expect(screen.getByTestId('activity-when')).toHaveTextContent(/· Run · Garmin Forerunner 965$/);
  });

  it('shows no attribution when the activity carries none', async () => {
    const screen = renderScreen();
    await screen.findByTestId('activity-figures');
    expect(screen.queryByTestId('activity-attribution')).toBeNull();
  });

  // "50:12" beside a distance reads as hours as easily as minutes, and French
  // writes neither "50m 12s" nor a bare clock: the figure is in the athlete's
  // words, while the splits stay a clock column, alike in every language.
  it('writes the duration in the athlete\'s language, and keeps the splits a clock', async () => {
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      const screen = renderScreen();
      const figures = await screen.findByTestId('activity-figures');
      expect(within(figures).getByTestId('activity-figure-duration')).toHaveTextContent('Durée50 min 12 s');
      expect(within(figures).getByTestId('activity-figure-distance')).toHaveTextContent('Distance10,20 km');
      expect(screen.getByTestId('activity-splits-row-1')).toHaveTextContent('11,00 km4:584:58 /km151 bpm+6 m');
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  it('answers a suggested question right under the activity, and sends the next one to the same thread', async () => {
    const screen = renderScreen();

    const prompts = await screen.findByTestId('activity-prompts');
    expect(within(prompts).getAllByRole('button')).toHaveLength(4);
    expect(prompts).toHaveTextContent('Analyze this effortCompare with my recent onesRecovery adviceWhat to adjust');

    const compare = `How does my activity “Tempo Thursday” from ${TEMPO_DAY} compare with my recent ones?`;
    await act(async () => {
      fireEvent.press(screen.getByTestId('activity-prompt-compare'));
    });

    await waitFor(() => expect(screen.getByText(REPLY)).toBeTruthy());
    expect(screen.getByText(compare)).toBeTruthy();
    // Still the activity's screen: nothing navigated away, the figures stand.
    expect(mockPush).not.toHaveBeenCalled();
    expect(screen.getByTestId('activity-figures')).toBeTruthy();
    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
    // carnet#828: a chip's question is reported as one, not as typed.
    expect(mockSendTurn).toHaveBeenLastCalledWith(THREAD, compare, expect.objectContaining({ origin: 'chip' }));
    expect(mockLinkActivityConversation).toHaveBeenCalledWith('strava', '9000', THREAD);

    await act(async () => {
      fireEvent.press(screen.getByTestId('activity-prompt-recovery'));
    });
    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(2));
    expect(mockSendTurn).toHaveBeenLastCalledWith(
      THREAD,
      `What recovery do you advise after my activity “Tempo Thursday” from ${TEMPO_DAY}?`,
      expect.objectContaining({ origin: 'chip' }),
    );
    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
    // Both turns are rows of the one thread: the first question and its reply stay.
    await waitFor(() => expect(screen.getAllByText(REPLY)).toHaveLength(2));
    expect(screen.getByText(compare)).toBeTruthy();
  });

  it('sends a typed first question in the sentence that names the activity', async () => {
    const screen = renderScreen();

    const input = await screen.findByTestId('message-input');
    fireEvent.changeText(input, '  Was I too fast on the first kilometre? ');
    await act(async () => {
      fireEvent.press(screen.getByTestId('send-button'));
    });

    const asked = `About my activity “Tempo Thursday” from ${TEMPO_DAY}: Was I too fast on the first kilometre?`;
    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledWith(THREAD, asked, expect.anything()));
    expect(await screen.findByText(asked)).toBeTruthy();
    expect(screen.getByTestId('message-input').props.value).toBe('');
  });

  it('reopens the thread an earlier visit opened — from the server, with nothing cached — and asks there', async () => {
    const first = renderScreen();
    await act(async () => {
      fireEvent.press(await first.findByTestId('activity-prompt-analyze'));
    });
    await first.findByText(REPLY);
    first.unmount();

    // The same activity again on a fresh cache — a relaunch, another device:
    // the server's detail names the thread, which is read, not created.
    const asked = `Analyze my activity “Tempo Thursday” from ${TEMPO_DAY}: how did this effort go?`;
    mockGetConversationMessages.mockResolvedValue({
      messages: [
        { id: 'u-1', conversation_id: THREAD, role: 'user', content: asked, created_at: '2026-09-30T10:00:00Z' },
        { id: 'a-1', conversation_id: THREAD, role: 'assistant', content: REPLY, created_at: '2026-09-30T10:00:02Z' },
      ],
    });
    const again = renderScreen(newClient());
    expect(await again.findByText(REPLY)).toBeTruthy();
    expect(again.getByText(asked)).toBeTruthy();
    expect(mockGetConversationMessages).toHaveBeenCalledWith(THREAD);

    await act(async () => {
      fireEvent.press(again.getByTestId('activity-prompt-adjust'));
    });
    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(2));
    expect(mockSendTurn).toHaveBeenLastCalledWith(THREAD, expect.any(String), expect.anything());
    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
  });

  it('keeps a typed question and says so when the thread cannot be opened', async () => {
    mockCreateConversation.mockRejectedValue({ response: { status: 500, data: {} } });
    const screen = renderScreen();

    fireEvent.changeText(await screen.findByTestId('message-input'), 'Was I too fast?');
    await act(async () => {
      fireEvent.press(screen.getByTestId('send-button'));
    });

    expect(await screen.findByTestId('activity-chat-error')).toHaveTextContent(/.+/);
    expect(screen.getByTestId('message-input').props.value).toBe('Was I too fast?');
    expect(mockSendTurn).not.toHaveBeenCalled();
  });

  it('says an activity the athlete does not hold is not among theirs, and asks once', async () => {
    mockGetActivityDetail.mockRejectedValue({ response: { status: 404, data: { message: 'not found' } } });
    const screen = renderScreen();

    expect(await screen.findByTestId('activity-not-found')).toHaveTextContent(
      "This activity isn't among your synced activities.",
    );
    expect(screen.queryByTestId('activity-prompts')).toBeNull();
    expect(mockGetActivityDetail).toHaveBeenCalledTimes(1);
  });

  it('offers a retry when the read failed, and shows the activity once it answers', async () => {
    mockGetActivityDetail.mockRejectedValue({ response: { status: 500, data: {} } });
    const screen = renderScreen();

    expect(await screen.findByTestId('activity-failed', {}, { timeout: 6000 })).toHaveTextContent(
      /^This activity couldn't be loaded\./,
    );
    // Asked twice more on its own before saying so.
    expect(mockGetActivityDetail).toHaveBeenCalledTimes(3);
    mockGetActivityDetail.mockResolvedValue(TEMPO_DETAIL_RESPONSE);
    fireEvent.press(screen.getByTestId('activity-retry'));
    await waitFor(() => expect(screen.getByTestId('activity-figures')).toBeTruthy());
  }, 10_000);

  describe('Delete on an uploaded activity', () => {
    const UPLOAD_ID = `${'a'.repeat(64)}-0`;

    /** The buttons of the alert the last call raised. */
    function alertButtons(alert: jest.SpyInstance): AlertButton[] {
      const call = alert.mock.calls[alert.mock.calls.length - 1];
      return (call?.[2] ?? []) as AlertButton[];
    }

    it("is not offered on a provider's activity", async () => {
      const screen = renderScreen();
      await screen.findByTestId('activity-figures');
      expect(screen.queryByTestId('activity-delete')).toBeNull();
    });

    it('deletes an upload once confirmed, then goes back', async () => {
      mockParams = { provider: 'upload', activityId: UPLOAD_ID };
      mockDeleteUploadedActivity.mockResolvedValue(undefined);
      const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => {});
      try {
        const screen = renderScreen();
        const action = await screen.findByTestId('activity-delete');
        expect(action.props.accessibilityLabel).toBe('Delete this uploaded activity');

        fireEvent.press(action);
        expect(alert).toHaveBeenCalledWith(
          'Delete this activity?',
          expect.stringContaining("This can't be undone."),
          expect.any(Array),
        );
        expect(mockDeleteUploadedActivity).not.toHaveBeenCalled();
        const confirm = alertButtons(alert).find((button) => button.style === 'destructive');
        expect(confirm?.text).toBe('Delete');
        act(() => confirm?.onPress?.());

        await waitFor(() => expect(mockBack).toHaveBeenCalledTimes(1));
        expect(mockDeleteUploadedActivity).toHaveBeenCalledWith(UPLOAD_ID);
      } finally {
        alert.mockRestore();
      }
    });

    it('sends nothing when the confirmation is cancelled', async () => {
      mockParams = { provider: 'upload', activityId: UPLOAD_ID };
      const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => {});
      try {
        const screen = renderScreen();
        fireEvent.press(await screen.findByTestId('activity-delete'));
        const cancel = alertButtons(alert).find((button) => button.style === 'cancel');
        act(() => cancel?.onPress?.());
        expect(mockDeleteUploadedActivity).not.toHaveBeenCalled();
        expect(mockBack).not.toHaveBeenCalled();
      } finally {
        alert.mockRestore();
      }
    });

    it('says a failed delete and stays on the activity', async () => {
      mockParams = { provider: 'upload', activityId: UPLOAD_ID };
      mockDeleteUploadedActivity.mockRejectedValue({ response: { status: 500, data: {} } });
      const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => {});
      try {
        const screen = renderScreen();
        fireEvent.press(await screen.findByTestId('activity-delete'));
        const confirm = alertButtons(alert).find((button) => button.style === 'destructive');
        act(() => confirm?.onPress?.());

        await waitFor(() =>
          expect(alert).toHaveBeenCalledWith('Error', "This activity couldn't be deleted. Try again."),
        );
        expect(mockBack).not.toHaveBeenCalled();
      } finally {
        alert.mockRestore();
      }
    });
  });
});
