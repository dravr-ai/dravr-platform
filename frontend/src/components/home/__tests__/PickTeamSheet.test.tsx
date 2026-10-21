// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's Pick your team sheet — the onboarding picker over Home for an athlete with no team
// ABOUTME: Red if Start or Skip writes anything but a complete coach_proposal step with the lead (+ every ticked agent), or the sheet stays open (carnet#1098)

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PickTeamSheet } from '../YourTeam';

const api = vi.hoisted(() => ({
  getProposal: vi.fn(),
  setOnboardingStep: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  coachesApi: { getProposal: api.getProposal, list: vi.fn() },
  chatApi: { openPersonalTopic: vi.fn(), getPersonalConversation: vi.fn() },
  userApi: { setOnboardingStep: api.setOnboardingStep },
}));

// Discover in the picker's sheet is pinned by the step's own suite.
vi.mock('../../StoreScreen', () => ({ default: () => null }));

const agent = (id: string, title: string, category: string) => ({ id, title, category });

const PROPOSAL = {
  profile: { has_profile: true, window_days: 90, total_activities: 12, primary_sport: 'ride', sport_mix: [] },
  agents: [{ agent: agent('agent-endurance', 'Endurance', 'training'), match_score: 0.9, reason: 'You ride often.' }],
  team: {
    lead: { agent: agent('agent-endurance', 'Endurance', 'training'), match_score: 0.9, reason: 'You ride often.' },
    members: [
      { agent: agent('agent-fuel', 'Fuelling', 'nutrition'), rule: 'goal', preselected: true, reason: 'Long days.' },
      { agent: agent('agent-mobility', 'Mobility', 'mobility'), rule: 'strength', preselected: false, reason: 'Short routines.' },
    ],
  },
};

function renderSheet() {
  const onClose = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, retryDelay: 0 } } });
  render(
    <QueryClientProvider client={client}>
      <PickTeamSheet side="right" onClose={onClose} />
    </QueryClientProvider>,
  );
  return { onClose };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.getProposal.mockResolvedValue(PROPOSAL);
  api.setOnboardingStep.mockResolvedValue(undefined);
});

describe('PickTeamSheet', () => {
  it('opens the onboarding picker titled Pick your team, and Start writes the coach_proposal step', async () => {
    const user = userEvent.setup();
    const { onClose } = renderSheet();

    expect(await screen.findByRole('dialog', { name: 'Pick your team' })).toBeInTheDocument();
    await user.click(await screen.findByRole('button', { name: 'Start with my team · 2' }));

    await waitFor(() =>
      expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('coach_proposal', 'complete', {
        agentId: 'agent-endurance',
        teamAgentIds: ['agent-endurance', 'agent-fuel'],
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it('Skip writes the step complete with the lead alone, never skipped', async () => {
    const user = userEvent.setup();
    const { onClose } = renderSheet();

    await user.click(await screen.findByRole('button', { name: 'Skip — start with Endurance only' }));

    await waitFor(() =>
      expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('coach_proposal', 'complete', {
        agentId: 'agent-endurance',
        teamAgentIds: ['agent-endurance'],
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it('keeps the sheet open when the team could not be written', async () => {
    const user = userEvent.setup();
    api.setOnboardingStep.mockRejectedValue(new Error('503'));
    const { onClose } = renderSheet();

    await user.click(await screen.findByRole('button', { name: 'Start with my team · 2' }));
    expect(await screen.findByText("We couldn't set up your team just now.")).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('says suggestions failed, with a retry, when no proposal comes back', async () => {
    const user = userEvent.setup();
    api.getProposal.mockRejectedValue(new Error('503'));
    renderSheet();

    expect(await screen.findByText("We couldn't build your agent suggestions just now.", {}, { timeout: 3000 })).toBeInTheDocument();
    api.getProposal.mockResolvedValue(PROPOSAL);
    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('button', { name: 'Start with my team · 2' })).toBeInTheDocument();
  });
});
