// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the onboarding coach proposal — « Démarrer » opens a thread bound to the chosen coach
// ABOUTME: Pins the profile sentence and the localized sport and category words the cards show

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import OnboardingCoachProposal from '../OnboardingCoachProposal';

const getProposal = vi.fn();
const recordUsage = vi.fn();
const createConversation = vi.fn();

vi.mock('../../services/api', () => ({
  coachesApi: {
    getProposal: (...a: unknown[]) => getProposal(...a),
    recordUsage: (...a: unknown[]) => recordUsage(...a),
  },
  chatApi: {
    createConversation: (...a: unknown[]) => createConversation(...a),
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
  coaches: [
    {
      coach: { id: 'coach-trail', title: 'Trail Coach', category: 'training' },
      reason: 'You run hills.',
    },
  ],
};

function renderProposal(onComplete = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
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
    window.location.hash = '';
  });

  it('opens a thread bound to the chosen coach and lands the athlete in it', async () => {
    const onComplete = renderProposal();
    const start = await screen.findByRole('button', { name: 'Start' });
    await userEvent.click(start);

    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(recordUsage).toHaveBeenCalledWith('coach-trail');
    expect(createConversation).toHaveBeenCalledWith({ coach_id: 'coach-trail', title: 'Trail Coach' });
    expect(window.location.hash).toBe('#chat/conv-9');
  });

  it('writes the profile as one sentence with localized sport and category words', async () => {
    renderProposal();
    expect(await screen.findByText(/Over the last 14 days we logged 9 activities/)).toBeInTheDocument();
    expect(screen.getByText(/mostly Trail running/)).toBeInTheDocument();
    expect(screen.getByText('Kayaking')).toBeInTheDocument();
    expect(screen.getByText('Training')).toBeInTheDocument();
  });
});
