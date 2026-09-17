// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile OnboardingCoachProposalScreen — analyzing spinner, inferred profile, top-3 coaches
// ABOUTME: Pins the sport-mix summary, the "Start" flow (recordUsage → markSeen → createConversation → push) and the error/skip fallbacks

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Agent, AgentProposalResponse } from '@pierre/shared-types';
import { OnboardingCoachProposalScreen } from '../OnboardingCoachProposalScreen';
import { coachesApi } from '../../../services/api';
import { useCoachProposalSeen } from '../../../hooks/useCoachProposalSeen';
import { useConversations } from '../../chat/useConversations';

const mockPush = jest.fn();

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush }),
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', display_name: 'Jean' } }),
}));
jest.mock('../../../hooks/useCoachProposalSeen');
jest.mock('../../../hooks/useOnboardingProgress', () => ({
  useOnboardingProgress: () => [
    { id: 'coach_proposal', labelKey: 'onboarding.stepAgent', status: 'current' },
  ],
}));
jest.mock('../../chat/useConversations');
jest.mock('../../../services/api', () => ({
  coachesApi: { getProposal: jest.fn(), recordUsage: jest.fn() },
}));

const getProposal = coachesApi.getProposal as jest.Mock;
const recordUsage = coachesApi.recordUsage as jest.Mock;
const markSeen = jest.fn();
const createConversation = jest.fn();

function baseAgent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: 'agent-1',
    title: 'Coach Ada',
    description: null,
    system_prompt: 'You are a coach.',
    category: 'training',
    tags: [],
    token_count: 100,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    is_system: true,
    ...overrides,
  };
}

const PROPOSAL: AgentProposalResponse = {
  profile: {
    has_profile: true,
    primary_sport: 'Running',
    total_activities: 12,
    window_days: 90,
    sport_mix: [{ sport: 'Running', count: 10, share: 0.8 }],
  },
  agents: [
    { agent: baseAgent(), match_score: 0.9, reason: 'You run a lot, so Ada fits.' },
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

describe('OnboardingCoachProposalScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    getProposal.mockReset().mockImplementation(() => new Promise(() => {}));
    recordUsage.mockReset().mockResolvedValue(undefined);
    markSeen.mockReset().mockResolvedValue(undefined);
    createConversation.mockReset().mockResolvedValue({ id: 'conv-1' });
    (useCoachProposalSeen as jest.Mock).mockReturnValue({ seen: false, markSeen });
    (useConversations as jest.Mock).mockReturnValue({ createConversation });
  });

  it('shows the analyzing spinner under the progress hairline while the proposal loads', () => {
    renderScreen();
    expect(screen.getByTestId('onboarding-progress-bar')).toBeTruthy();
    expect(screen.getByText('Analyzing your training data…')).toBeTruthy();
  });

  it('renders the inferred sport-mix summary and the proposed coach with its reason', async () => {
    getProposal.mockResolvedValue(PROPOSAL);
    renderScreen();

    expect(await screen.findByText('Coach Ada')).toBeTruthy();
    expect(screen.getByText('Training')).toBeTruthy();
    expect(screen.getByText('You run a lot, so Ada fits.')).toBeTruthy();
    expect(screen.getByText(/Over the last 90 days we logged 12 activities/)).toBeTruthy();
  });

  it('starting a coach records usage, marks the step seen, and opens its thread', async () => {
    getProposal.mockResolvedValue(PROPOSAL);
    renderScreen();

    fireEvent.press(await screen.findByText('Start'));

    await waitFor(() => expect(recordUsage).toHaveBeenCalledWith('agent-1'));
    await waitFor(() => expect(markSeen).toHaveBeenCalled());
    // No title: the server names the thread after the agent it is bound to.
    await waitFor(() => expect(createConversation).toHaveBeenCalledWith({ agent_id: 'agent-1' }));
    await waitFor(() =>
      expect(mockPush).toHaveBeenCalledWith(
        expect.objectContaining({ params: { conversationId: 'conv-1' } }),
      ),
    );
  });

  it('a cold-start profile with no activity history shows the fallback line, not a broken summary', async () => {
    getProposal.mockResolvedValue({
      profile: { has_profile: false, total_activities: 0, window_days: 90, sport_mix: [] },
      agents: [],
    });
    renderScreen();

    expect(
      await screen.findByText(
        "We didn't find recent activities yet — here are some agents to start with. Your picks will sharpen as your provider syncs more data.",
      ),
    ).toBeTruthy();
  });

  it('an error loading the proposal offers Continue, which marks the step seen', async () => {
    getProposal.mockRejectedValue(new Error('network'));
    renderScreen();

    // The screen's own query retries once before settling into `isError`, so
    // this needs more than `findByText`'s default 1s timeout.
    expect(
      await screen.findByText("We couldn't build your agent suggestions just now.", {}, { timeout: 5000 }),
    ).toBeTruthy();
    fireEvent.press(screen.getByText('Continue'));
    expect(markSeen).toHaveBeenCalled();
  }, 10000);

  it('"Skip for now" marks the step seen without starting a coach', async () => {
    getProposal.mockResolvedValue(PROPOSAL);
    renderScreen();

    fireEvent.press(await screen.findByText('Skip for now'));
    expect(markSeen).toHaveBeenCalled();
    expect(recordUsage).not.toHaveBeenCalled();
  });
});
