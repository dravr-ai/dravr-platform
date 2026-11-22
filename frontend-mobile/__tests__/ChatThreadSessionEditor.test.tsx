// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "Build from family" in the coach's plan thread on the phone — offered to the owner of a thread about an athlete, and to nobody else
// ABOUTME: The thread reads its draft as ChatScreen does, so the server's answer for the owner is what turns the day's action on

import React from 'react';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { DraftState, PlanDay, SubjectPlanResponse, WorkoutPlan } from '@pierre/shared-types';

const CONVERSATION_ID = 'conv-plan-chloe';
const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-chloe';

const mockGetSubjectPlan = jest.fn();
const mockListParticipants = jest.fn();
const mockGetFamilies = jest.fn();
const mockPreview = jest.fn();
const mockSaveDay = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getSubjectPlan: (...a: unknown[]) => mockGetSubjectPlan(...a),
    listParticipants: (...a: unknown[]) => mockListParticipants(...a),
    getSubjectFamilies: (...a: unknown[]) => mockGetFamilies(...a),
    previewSubjectSession: (...a: unknown[]) => mockPreview(...a),
    saveSubjectDraftDay: (...a: unknown[]) => mockSaveDay(...a),
  },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: COACH_ID }, isAuthenticated: true }),
}));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
// The transcript, reduced to one row's plan card and the open-draft slot: the
// card takes the day action the thread hands the list, as the list's own
// workout_plan arm passes it (SessionEditorSheet.test.tsx mounts that arm).
jest.mock('../src/screens/chat/MessageList', () => {
  const React = require('react');
  const { View } = require('react-native');
  const WorkoutPlanCard = require('../src/screens/chat/WorkoutPlanCard').default;
  return {
    MessageList: ({
      planDraft,
      onBuildPlanDay,
    }: {
      planDraft?: React.ReactNode;
      onBuildPlanDay?: (day: unknown) => void;
    }) =>
      React.createElement(
        View,
        null,
        React.createElement(
          View,
          { testID: 'thread-row-card' },
          React.createElement(WorkoutPlanCard, { plan: mockThreadPlan, onBuildDay: onBuildPlanDay }),
        ),
        planDraft ?? null,
      ),
  };
});
jest.mock('../src/screens/chat/ChatInputBar', () => ({ ChatInputBar: () => null }));
jest.mock('../src/screens/chat/UsageWarningBanner', () => ({ UsageWarningBanner: () => null }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({ useChatVoiceInput: () => ({}) }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));

import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';
import { usePlanThreadDraft } from '../src/hooks/useCoachPlan';

const SESSION_DAY: PlanDay = {
  date: '2026-11-24',
  sport: 'run',
  workout: 'Easy run',
  duration_min: 45,
  intensity: 'easy',
  rest: false,
};
const REST_DAY: PlanDay = { date: '2026-11-25', sport: 'rest', workout: 'Rest', intensity: 'rest', rest: true };

/** The plan the thread's row card shows, and the draft the read holds open. */
const mockThreadPlan: WorkoutPlan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
  phases: [],
  weeks: [{ week_start: '2026-11-23', focus: 'build', current: true, days: [SESSION_DAY, REST_DAY] }],
  weeks_deferred: 0,
};

const DRAFT: DraftState = { draft_id: 'draft-7', updated_at: '2026-11-21T14:05:00Z', conflicts: [], season_relaid: false };

function subjectPlan(draftState: DraftState | null): SubjectPlanResponse {
  return {
    athlete: { user_id: ATHLETE_ID, display_name: 'Chloé Tremblay' },
    today: '2026-11-21',
    live: mockThreadPlan,
    draft: draftState ? mockThreadPlan : null,
    draft_state: draftState,
  };
}

/**
 * The thread as ChatScreen hosts it: the draft read is made for a thread
 * about an athlete, and handed to ChatThread with the athlete's name.
 */
function PlanThreadHost({ subjectName }: { subjectName: string | null }) {
  const planDraftRead = usePlanThreadDraft(CONVERSATION_ID, subjectName !== null);
  const messagesHook = {
    scrollToBottom: jest.fn(),
    quotaNotice: null,
    verdicts: [],
    verdictsLoading: false,
    refreshVerdicts: jest.fn(),
    messages: [],
    messageBlocks: {},
    turnInFlight: false,
  } as unknown as ChatThreadProps['messagesHook'];
  const usageStatus = { applyNotice: jest.fn(), sendDisabled: false } as unknown as ChatThreadProps['usageStatus'];
  const providerStatus = {
    connectedProviders: [],
    setNeedsCredentialsProvider: jest.fn(),
    loadProviderStatus: jest.fn(),
    needsCredentialsProvider: null,
  } as unknown as ChatThreadProps['providerStatus'];
  return (
    <ChatThread
      conversationId={CONVERSATION_ID}
      messagesHook={messagesHook}
      usageStatus={usageStatus}
      providerStatus={providerStatus}
      isLoading={false}
      inputText=""
      onChangeInputText={jest.fn()}
      inputRef={React.createRef()}
      sendText={jest.fn().mockResolvedValue(undefined)}
      planSubjectName={subjectName}
      planDraftRead={planDraftRead}
    />
  );
}

function renderThread(subjectName: string | null) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <PlanThreadHost subjectName={subjectName} />
    </QueryClientProvider>,
  );
}

describe('"Build from family" in the coach\'s plan thread', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockListParticipants.mockResolvedValue([
      { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-11-20T12:00:00Z' },
    ]);
    mockGetFamilies.mockResolvedValue({ families: [] });
  });

  it("offers every day of the thread's plan card, the rest day included, to the thread's owner", async () => {
    mockGetSubjectPlan.mockResolvedValue(subjectPlan(null));
    renderThread('Chloé Tremblay');

    const card = within(screen.getByTestId('thread-row-card'));
    expect(await card.findByTestId('workout-plan-day-build-2026-11-24')).toHaveTextContent('Build from family');
    expect(card.getByTestId('workout-plan-day-build-2026-11-25')).toHaveTextContent('Build from family');
    expect(mockGetSubjectPlan).toHaveBeenCalledWith(CONVERSATION_ID);
  });

  it("offers the open draft's days too, the card the thread draws for a draft no row shows", async () => {
    mockGetSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
    renderThread('Chloé Tremblay');

    const draftCard = within(await screen.findByTestId('chat-plan-thread-open-draft'));
    expect(draftCard.getByTestId('workout-plan-day-build-2026-11-25')).toBeTruthy();
  });

  it('offers nothing to a member of the thread who does not own it, whom the server refuses the plan', async () => {
    mockGetSubjectPlan.mockRejectedValue(Object.assign(new Error('Not found'), { response: { status: 404 } }));
    renderThread('Chloé Tremblay');

    await waitFor(() => expect(mockGetSubjectPlan).toHaveBeenCalledWith(CONVERSATION_ID));
    expect(screen.getByText('Harricana 65K')).toBeTruthy();
    expect(screen.queryByText('Build from family')).toBeNull();
    expect(screen.queryByTestId('session-editor-sheet')).toBeNull();
  });

  it('offers nothing in a thread about no athlete, and reads no plan for it', () => {
    renderThread(null);

    expect(screen.getByText('Harricana 65K')).toBeTruthy();
    expect(screen.queryByText('Build from family')).toBeNull();
    expect(mockGetSubjectPlan).not.toHaveBeenCalled();
  });

  it('opens the editor on the day pressed, and closes it on Cancel', async () => {
    mockGetSubjectPlan.mockResolvedValue(subjectPlan(null));
    renderThread('Chloé Tremblay');

    fireEvent.press(await screen.findByTestId('workout-plan-day-build-2026-11-25'));

    expect(screen.getByTestId('session-editor-title')).toHaveTextContent('Build a session for Wednesday, November 25');
    expect(await screen.findByTestId('session-editor-no-families')).toHaveTextContent(
      'No workout families are available for Chloé Tremblay.',
    );
    expect(mockGetFamilies).toHaveBeenCalledWith(CONVERSATION_ID, 'en');

    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    expect(screen.queryByTestId('session-editor-title')).toBeNull();
  });
});
