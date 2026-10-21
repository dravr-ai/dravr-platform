// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the onboarding coach proposal — « Démarrer » opens a thread bound to the chosen coach
// ABOUTME: Pins the lead agent handed to onComplete, the profile sentence and the localized sport and category words

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import OnboardingCoachProposal from '../OnboardingCoachProposal';

const getProposal = vi.fn();
const recordUsage = vi.fn();
const createConversation = vi.fn();
const openPersonalTopic = vi.fn();

vi.mock('../../services/api', () => ({
  coachesApi: {
    getProposal: (...a: unknown[]) => getProposal(...a),
    recordUsage: (...a: unknown[]) => recordUsage(...a),
  },
  chatApi: {
    createConversation: (...a: unknown[]) => createConversation(...a),
    openPersonalTopic: (...a: unknown[]) => openPersonalTopic(...a),
  },
}));

const PROPOSAL = {
  profile: {
    has_profile: true,
    window_days: 14,
    total_activities: 9,
    primary_sport: 'Trail Running',
    sport_mix: [
      { sport: 'trail_running', share: 0.6 },
      { sport: 'Kayaking V2', share: 0.4 },
    ],
  },
  agents: [
    {
      agent: { id: 'coach-trail', title: 'Trail Coach', category: 'training' },
      reason: 'You run hills.',
    },
  ],
};

/** Two proposed agents, so the lead is visibly the one tapped and not merely the only one. */
const TWO_AGENT_PROPOSAL = {
  ...PROPOSAL,
  agents: [
    PROPOSAL.agents[0],
    {
      agent: { id: 'coach-road', title: 'Road Coach', category: 'training' },
      reason: 'You race on the road.',
    },
  ],
};

function renderProposal(onComplete = vi.fn()) {
  // The component sets `retry: 1` itself; a zero delay keeps the error case quick.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, retryDelay: 0 } },
  });
  render(
    <QueryClientProvider client={client}>
      <OnboardingCoachProposal userDisplayName="Maya" onComplete={onComplete} />
    </QueryClientProvider>,
  );
  return onComplete;
}

describe('OnboardingCoachProposal', () => {
  beforeEach(() => {
    getProposal.mockReset().mockResolvedValue(PROPOSAL);
    recordUsage.mockReset().mockResolvedValue(undefined);
    createConversation.mockReset().mockResolvedValue({ id: 'conv-9' });
    openPersonalTopic.mockReset().mockResolvedValue({
      conversation: { id: 'conv-9', title: 'Trail Coach', personal: true },
      divider: null,
    });
    window.location.hash = '';
  });

  it('opens a topic bound to the chosen coach in the personal thread and lands the athlete on Home', async () => {
    const onComplete = renderProposal();
    const start = await screen.findByRole('button', { name: 'Start' });
    await userEvent.click(start);

    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    // The chosen coach is the lead the step records as the athlete's agent.
    expect(onComplete).toHaveBeenCalledWith('coach-trail');
    expect(recordUsage).toHaveBeenCalledWith('coach-trail');
    // The athlete's one thread, rebound to the coach — never a second thread.
    expect(openPersonalTopic).toHaveBeenCalledWith({ agent_id: 'coach-trail' });
    expect(createConversation).not.toHaveBeenCalled();
    expect(window.location.hash).toBe('#home/chat/conv-9');
  });

  it('hands onComplete the agent whose card was tapped, not the first one', async () => {
    getProposal.mockResolvedValue(TWO_AGENT_PROPOSAL);
    const onComplete = renderProposal();
    const starts = await screen.findAllByRole('button', { name: 'Start' });
    await userEvent.click(starts[1]);

    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(onComplete).toHaveBeenCalledWith('coach-road');
    expect(openPersonalTopic).toHaveBeenCalledWith({ agent_id: 'coach-road' });
  });

  it('Skip hands onComplete the first proposed agent and opens no topic', async () => {
    getProposal.mockResolvedValue(TWO_AGENT_PROPOSAL);
    const onComplete = renderProposal();
    await userEvent.click(await screen.findByRole('button', { name: 'Skip for now' }));

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete).toHaveBeenCalledWith('coach-trail');
    expect(recordUsage).not.toHaveBeenCalled();
    expect(openPersonalTopic).not.toHaveBeenCalled();
  });

  it('Continue after a failed proposal hands onComplete no agent at all', async () => {
    getProposal.mockRejectedValue(new Error('network'));
    const onComplete = renderProposal();
    await userEvent.click(
      await screen.findByRole('button', { name: 'Continue to dashboard' }, { timeout: 5000 }),
    );

    expect(onComplete).toHaveBeenCalledTimes(1);
    // Exactly no argument — not a click event mistaken for an agent id.
    expect(onComplete.mock.calls[0]).toEqual([]);
  });

  it('writes the profile as one sentence with localized sport and category words', async () => {
    renderProposal();
    expect(await screen.findByText(/Over the last 14 days we logged 9 activities/)).toBeInTheDocument();
    expect(screen.getByText(/mostly Trail running/)).toBeInTheDocument();
    expect(screen.getByText('Kayaking')).toBeInTheDocument();
    expect(screen.getByText('Training')).toBeInTheDocument();
  });
});
