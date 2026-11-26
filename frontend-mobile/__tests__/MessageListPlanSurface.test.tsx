// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card in the athlete workspace's thread is one line — « Plan proposed · See it on the calendar » — leading back to the plan tab (carnet#1818)
// ABOUTME: Pins the line, its press, the subject-plan read it keeps fresh, and that every other host still draws the card whole

import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { SubjectPlanResponse, WorkoutPlan } from '@pierre/shared-types';

const mockGetSubjectPlan = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getSubjectPlan: (...a: unknown[]) => mockGetSubjectPlan(...a),
    listParticipants: jest.fn().mockResolvedValue([]),
  },
}));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
jest.mock('../src/screens/chat/presentMessageMenu', () => ({ presentMessageMenu: jest.fn() }));

import { MessageList } from '../src/screens/chat/MessageList';
import type { WorkspacePlanSurface } from '../src/screens/chat/PlanChangeEntry';
import type { Message } from '../src/types';

const PLAN_ID = 'conv-plan-lea';

const PLAN: WorkoutPlan = {
  goal_race: { name: 'The Traka 200', date: '2027-04-30', discipline: 'gravel', priority: 'A' },
  phases: [],
  weeks: [
    {
      week_start: '2026-11-23',
      focus: 'Prep',
      current: true,
      days: [{ date: '2026-11-25', sport: 'ride', workout: 'Track endurance', duration_min: 60, intensity: 'Z2', rest: false }],
    },
  ],
  weeks_deferred: 0,
};

const SUBJECT_PLAN: SubjectPlanResponse = {
  athlete: { user_id: 'u-lea', display_name: 'Léa Gagnon' },
  today: '2026-11-25',
  live: PLAN,
  draft: null,
  draft_state: null,
  proposed_dates: [],
};

/** The agent's turn that saved the plan, as the thread reads it back. */
const PLAN_ROW: Message = {
  id: 'msg-plan-1',
  role: 'assistant',
  content: 'Here is her fortnight.',
  created_at: '2026-11-25T10:00:00Z',
  scene_blocks: JSON.stringify([{ kind: 'workout_plan', source_tool: 'save_training_plan', plan: PLAN }]),
} as Message;

function renderList(planSurface: WorkspacePlanSurface | null) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <MessageList
        conversationId={PLAN_ID}
        messages={[PLAN_ROW]}
        planSubjectName="Léa Gagnon"
        isLoading={false}
        isSending={false}
        messageFeedback={{}}
        messageFeedbackComment={{}}
        messageBlocks={{}}
        flatListRef={React.createRef()}
        onScrollToBottom={jest.fn()}
        onThumbsUp={jest.fn()}
        onThumbsDown={jest.fn()}
        onSubmitFeedbackReason={jest.fn()}
        onRetryMessage={jest.fn()}
        onOpenUrl={jest.fn()}
        onReconnectProvider={jest.fn()}
        planSurface={planSurface}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  mockGetSubjectPlan.mockResolvedValue(SUBJECT_PLAN);
});

describe('a plan card in the athlete workspace', () => {
  it('is one line saying a plan was proposed, never the card', async () => {
    renderList({ kind: 'workspace', onShowCalendar: jest.fn() });

    expect(screen.getByTestId('plan-change-entry')).toHaveTextContent('Plan proposed·See it on the calendar');
    expect(screen.queryByTestId('workout-plan-card')).toBeNull();
    expect(screen.queryByText('Track endurance')).toBeNull();
    // The line keeps the plan tab's read of the thread fresh, as a card does.
    await screen.findByTestId('plan-change-entry');
    expect(mockGetSubjectPlan).toHaveBeenCalledWith(PLAN_ID);
  });

  it('leads back to the plan tab', () => {
    const onShowCalendar = jest.fn();
    renderList({ kind: 'workspace', onShowCalendar });

    fireEvent.press(screen.getByTestId('plan-change-entry-show'));

    expect(onShowCalendar).toHaveBeenCalledTimes(1);
  });

  it('is drawn whole by every other host', () => {
    renderList(null);

    expect(screen.getByTestId('workout-plan-card')).toBeTruthy();
    expect(screen.getByText('Track endurance')).toBeTruthy();
    expect(screen.queryByTestId('plan-change-entry')).toBeNull();
  });
});
