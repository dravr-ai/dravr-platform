// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the mobile onboarding coach proposal — « Démarrer » opens a thread bound to the chosen coach
// ABOUTME: Pins the recordUsage → markSeen → createConversation → push order and the localized sport and category words

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachProposalResponse } from '@pierre/shared-types';

const mockRouter = {
  push: jest.fn(),
  replace: jest.fn(),
  back: jest.fn(),
  navigate: jest.fn(),
  canGoBack: () => true,
};
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    isAuthenticated: true,
    user: { id: 'user-1', email: 'maya@dravr.ai', display_name: 'Maya' },
  }),
}));

const mockMarkSeen = jest.fn();
jest.mock('../src/hooks/useCoachProposalSeen', () => ({
  useCoachProposalSeen: () => ({ seen: false, markSeen: (...args: unknown[]) => mockMarkSeen(...args) }),
}));

const mockCreateConversation = jest.fn();
jest.mock('../src/screens/chat/useConversations', () => ({
  useConversations: () => ({
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
  }),
}));

const mockGetProposal = jest.fn();
const mockRecordUsage = jest.fn();
jest.mock('../src/services/api', () => ({
  coachesApi: {
    getProposal: (...args: unknown[]) => mockGetProposal(...args),
    recordUsage: (...args: unknown[]) => mockRecordUsage(...args),
  },
}));

import { OnboardingCoachProposalScreen } from '../src/screens/onboarding/OnboardingCoachProposalScreen';
import { CHAT_THREAD_ROUTE, threadHref } from '../src/navigation/routes';

// The same proposal the web sibling test renders: a primary sport spelled as
// the wire spells it, a mix entry in snake_case and one carrying a version
// suffix, and one proposed coach in the training category.
const PROPOSAL: CoachProposalResponse = {
  profile: {
    has_profile: true,
    window_days: 14,
    total_activities: 9,
    primary_sport: 'Trail Running',
    sport_mix: [
      { sport: 'trail_running', count: 5, share: 0.6 },
      { sport: 'Kayaking V2', count: 4, share: 0.4 },
    ],
  },
  coaches: [
    {
      coach: {
        id: 'coach-trail',
        title: 'Trail Coach',
        description: 'Hills and long climbs',
        system_prompt: 'You coach trail runners.',
        category: 'training',
        tags: ['trail'],
        token_count: 800,
        is_favorite: false,
        use_count: 0,
        last_used_at: null,
        created_at: '2026-08-01T00:00:00Z',
        updated_at: '2026-08-01T00:00:00Z',
        is_system: true,
      },
      match_score: 0.9,
      reason: 'You run hills.',
    },
  ],
};

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <OnboardingCoachProposalScreen />
    </QueryClientProvider>,
  );
}

/** The index of a mock's first call in jest's global call sequence. */
function firstCallOrder(fn: jest.Mock): number {
  return fn.mock.invocationCallOrder[0];
}

describe('OnboardingCoachProposalScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetProposal.mockResolvedValue(PROPOSAL);
    mockRecordUsage.mockResolvedValue(undefined);
    mockMarkSeen.mockResolvedValue(undefined);
    mockCreateConversation.mockResolvedValue({ id: 'conv-9', title: 'Trail Coach', coach_id: 'coach-trail' });
  });

  // A coach the store shipped without a title used to open a thread named "",
  // which the list then drew as an untitled row. Web has fallen back to the
  // clock-shaped default since the messenger cutover; mobile does now too.
  it('names the thread by the clock when the coach has no title', async () => {
    mockGetProposal.mockResolvedValue({
      ...PROPOSAL,
      coaches: [{ ...PROPOSAL.coaches[0], coach: { ...PROPOSAL.coaches[0].coach, title: '' } }],
    });

    const { findByText } = renderScreen();
    fireEvent.press(await findByText('Start'));

    await waitFor(() => expect(mockCreateConversation).toHaveBeenCalledTimes(1));
    expect(mockCreateConversation.mock.calls[0][0]).toEqual({
      coach_id: 'coach-trail',
      title: expect.stringMatching(/^Chat .+ \d{2}:\d{2}$/),
    });
  });

  it('« Start » records the choice, completes the step, then lands the athlete in a thread bound to the coach', async () => {
    const { findByText } = renderScreen();

    fireEvent.press(await findByText('Start'));

    await waitFor(() => expect(mockRouter.push).toHaveBeenCalledTimes(1));
    expect(mockRecordUsage).toHaveBeenCalledTimes(1);
    expect(mockRecordUsage).toHaveBeenCalledWith('coach-trail');
    expect(mockMarkSeen).toHaveBeenCalledTimes(1);
    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
    expect(mockCreateConversation).toHaveBeenCalledWith({ coach_id: 'coach-trail', title: 'Trail Coach' });
    expect(mockRouter.push).toHaveBeenCalledWith({
      pathname: CHAT_THREAD_ROUTE,
      params: { conversationId: 'conv-9' },
    });
    expect(mockRouter.push.mock.calls[0][0]).toEqual(threadHref('conv-9'));
    expect(mockRouter.replace).not.toHaveBeenCalled();

    // The step is marked before the thread exists, so the layout may leave
    // onboarding even if the create call is slow; the push comes last.
    expect(firstCallOrder(mockRecordUsage)).toBeLessThan(firstCallOrder(mockMarkSeen));
    expect(firstCallOrder(mockMarkSeen)).toBeLessThan(firstCallOrder(mockCreateConversation));
    expect(firstCallOrder(mockCreateConversation)).toBeLessThan(firstCallOrder(mockRouter.push));
  });

  it('a failed thread creation still completes the step, navigates nowhere and leaks no rejection', async () => {
    mockCreateConversation.mockRejectedValue(new Error('conversation_create_failed'));
    const onUnhandledRejection = jest.fn();
    process.on('unhandledRejection', onUnhandledRejection);

    try {
      const { findByText } = renderScreen();

      fireEvent.press(await findByText('Start'));

      await waitFor(() => expect(mockCreateConversation).toHaveBeenCalledTimes(1));
      expect(mockCreateConversation).toHaveBeenCalledWith({ coach_id: 'coach-trail', title: 'Trail Coach' });
      // Node reports an orphaned rejection only after the microtask queue
      // drains; a macrotask boundary lets that report land before we look.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));

      expect(mockMarkSeen).toHaveBeenCalledTimes(1);
      expect(firstCallOrder(mockMarkSeen)).toBeLessThan(firstCallOrder(mockCreateConversation));
      expect(mockRouter.push).not.toHaveBeenCalled();
      expect(mockRouter.replace).not.toHaveBeenCalled();
      expect(onUnhandledRejection).not.toHaveBeenCalled();
    } finally {
      process.off('unhandledRejection', onUnhandledRejection);
    }
  });

  it('writes the profile as one sentence with localized sport and category words', async () => {
    const { findByText, getByText } = renderScreen();

    expect(await findByText("Here's your starting lineup, Maya")).toBeTruthy();
    // `primary_sport` arrives as the wire spells it ('Trail Running') and is
    // folded to the vocabulary's label, never interpolated raw into the prose.
    expect(getByText('Over the last 14 days we logged 9 activities, mostly Trail running.')).toBeTruthy();
    // The mix rows: snake_case resolves to the label, and a version suffix
    // ('Kayaking V2') is stripped before the lookup.
    expect(getByText('Trail running')).toBeTruthy();
    expect(getByText('Kayaking')).toBeTruthy();
    expect(getByText('60%')).toBeTruthy();
    expect(getByText('40%')).toBeTruthy();
    // The card: the coach's title and rationale as sent, the category localized.
    expect(getByText('Trail Coach')).toBeTruthy();
    expect(getByText('Training')).toBeTruthy();
    expect(getByText('You run hills.')).toBeTruthy();
    expect(getByText('Skip for now')).toBeTruthy();
  });
});
