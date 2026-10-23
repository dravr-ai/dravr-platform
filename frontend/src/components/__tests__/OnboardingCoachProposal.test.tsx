// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for onboarding's Your team step — the ticked count, Start writes every ticked agent, Skip the lead
// ABOUTME: Pins the awaited write (Starting…, inline error + retry), Change and Browse all agents, and the lead's topic

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import OnboardingCoachProposal from '../OnboardingCoachProposal';

const getProposal = vi.fn();
const recordUsage = vi.fn();
const openPersonalTopic = vi.fn();

vi.mock('../../services/api', () => ({
  coachesApi: {
    getProposal: (...a: unknown[]) => getProposal(...a),
    recordUsage: (...a: unknown[]) => recordUsage(...a),
  },
  chatApi: {
    openPersonalTopic: (...a: unknown[]) => openPersonalTopic(...a),
  },
}));

/**
 * Discover in the sheet, reduced to what the step relies on: the mode it is
 * opened in, the ids it is told the step already holds, and a pick of one
 * listing. Its own install-nothing contract is pinned in StoreScreen.test.
 */
const PICKABLE = { id: 'agent-mind', title: 'Mind Agent', category: 'custom', tags: [] };
vi.mock('../StoreScreen', () => ({
  default: ({
    mode,
    pickedIds,
    onPick,
  }: {
    mode: string;
    pickedIds: string[];
    onPick: (agent: unknown) => void;
  }) => (
    <div data-testid="store-pick" data-mode={mode} data-picked={pickedIds.join(',')}>
      <button type="button" onClick={() => onPick(PICKABLE)}>
        pick mind
      </button>
      <button
        type="button"
        onClick={() => onPick({ id: 'agent-fuel', title: 'Fuelling Agent', category: 'nutrition', tags: [] })}
      >
        pick fuelling
      </button>
    </div>
  ),
}));

const agent = (id: string, title: string, category: string) => ({ id, title, category });

const PROPOSAL = {
  profile: {
    has_profile: true,
    window_days: 90,
    total_activities: 46,
    primary_sport: 'ride',
    sport_mix: [
      { sport: 'ride', count: 33, share: 0.72 },
      { sport: 'run', count: 10, share: 0.21 },
    ],
  },
  goal: 'a gran fondo in June',
  agents: [{ agent: agent('agent-endurance', 'Endurance', 'training'), match_score: 0.9, reason: 'You ride four times a week.' }],
  team: {
    lead: { agent: agent('agent-endurance', 'Endurance', 'training'), match_score: 0.9, reason: 'You ride four times a week.' },
    members: [
      { agent: agent('agent-fuel', 'Fuelling Agent', 'nutrition'), rule: 'goal', preselected: true, reason: 'Long days are won on food.' },
      { agent: agent('agent-recovery', 'Recovery Agent', 'recovery'), rule: 'sleep_source', source: 'Garmin', preselected: true, reason: 'Garmin sends your sleep.' },
      { agent: agent('agent-mobility', 'Mobility Agent', 'mobility'), rule: 'strength', preselected: false, reason: 'Short routines.' },
    ],
  },
};

function renderStep({
  onSave = vi.fn().mockResolvedValue(undefined),
  onComplete = vi.fn(),
  withoutProvider = false,
} = {}) {
  // The step sets `retry: 1` itself; a zero delay keeps the error case quick.
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, retryDelay: 0 } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingCoachProposal
        userDisplayName="Camille"
        withoutProvider={withoutProvider}
        onSave={onSave}
        onComplete={onComplete}
      />
    </QueryClientProvider>,
  );
  return { onSave, onComplete };
}

const startButton = (count: number) => screen.findByRole('button', { name: `Start with my team · ${count}` });

describe('OnboardingCoachProposal — Your team', () => {
  beforeEach(() => {
    getProposal.mockReset().mockResolvedValue(PROPOSAL);
    recordUsage.mockReset().mockResolvedValue(undefined);
    openPersonalTopic.mockReset().mockResolvedValue({
      conversation: { id: 'conv-9', title: 'Endurance', personal: true },
      divider: null,
    });
    window.location.hash = '';
  });

  it('reads the data back, leads with the proposal\'s lead and ticks the members the rules preselected', async () => {
    renderStep();
    expect(await screen.findByRole('heading', { name: "Camille, here's your starting team" })).toBeInTheDocument();
    const read = screen.getByRole('region', { name: 'What we read' });
    expect(read).toHaveTextContent('Last 90 days · 46 activities, mostly Ride. Goal: a gran fondo in June');
    // The count is the compared number: mono.
    expect(within(read).getByText('46')).toHaveClass('font-mono');

    const lead = screen.getByTestId('team-lead');
    expect(within(lead).getByText('Endurance')).toBeInTheDocument();
    // The lead is always on the team: no tick box to take it off.
    expect(within(lead).queryByRole('checkbox')).not.toBeInTheDocument();

    expect(screen.getByRole('checkbox', { name: /Fuelling Agent/ })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: /Recovery Agent/ })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: /Mobility Agent/ })).not.toBeChecked();
    expect(screen.getByText('Nutrition · from your goal')).toBeInTheDocument();
    expect(screen.getByText('Recovery · from your Garmin data')).toBeInTheDocument();
    expect(screen.getByText('Mobility · from your strength work')).toBeInTheDocument();
    // One lead plus two ticked members, and the one filled button on the step.
    expect(await startButton(3)).toBeInTheDocument();
  });

  it('counts the lead plus whatever is ticked, live', async () => {
    const user = userEvent.setup();
    renderStep();
    await startButton(3);
    await user.click(screen.getByRole('checkbox', { name: /Mobility Agent/ }));
    expect(await startButton(4)).toBeInTheDocument();
    await user.click(screen.getByRole('checkbox', { name: /Fuelling Agent/ }));
    await user.click(screen.getByRole('checkbox', { name: /Recovery Agent/ }));
    await user.click(screen.getByRole('checkbox', { name: /Mobility Agent/ }));
    expect(await startButton(1)).toBeInTheDocument();
  });

  it('Start writes the lead and every ticked agent, then opens the lead\'s topic and lands on Home', async () => {
    const user = userEvent.setup();
    const { onSave, onComplete } = renderStep();
    await user.click(await startButton(3));

    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave).toHaveBeenCalledWith({
      status: 'complete',
      leadAgentId: 'agent-endurance',
      teamAgentIds: ['agent-endurance', 'agent-fuel', 'agent-recovery'],
    });
    expect(openPersonalTopic).toHaveBeenCalledWith({ agent_id: 'agent-endurance' });
    // The write lands before the topic opens: a refused team opens nothing.
    expect(onSave.mock.invocationCallOrder[0]).toBeLessThan(openPersonalTopic.mock.invocationCallOrder[0]);
    expect(window.location.hash).toBe('#home/chat/conv-9');
  });

  it('Skip writes the lead alone and opens no topic', async () => {
    const user = userEvent.setup();
    const { onSave, onComplete } = renderStep();
    await user.click(await screen.findByRole('button', { name: 'Skip — start with Endurance only' }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(onSave).toHaveBeenCalledWith({
      status: 'skipped',
      leadAgentId: 'agent-endurance',
      teamAgentIds: ['agent-endurance'],
    });
    expect(openPersonalTopic).not.toHaveBeenCalled();
    expect(recordUsage).not.toHaveBeenCalled();
  });

  it('waits on the write — Starting…, disabled — and a failure keeps the athlete here with a retry', async () => {
    const user = userEvent.setup();
    let reject: (error: Error) => void = () => undefined;
    const onSave = vi
      .fn()
      .mockImplementationOnce(() => new Promise<void>((_resolve, rej) => (reject = rej)))
      .mockResolvedValueOnce(undefined);
    const { onComplete } = renderStep({ onSave });
    await user.click(await startButton(3));

    const starting = await screen.findByRole('button', { name: 'Starting…' });
    expect(starting).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Skip — start with Endurance only' })).toBeDisabled();

    reject(new Error('503'));
    expect(await screen.findByRole('alert')).toHaveTextContent("We couldn't set up your team just now.");
    expect(onComplete).not.toHaveBeenCalled();
    expect(openPersonalTopic).not.toHaveBeenCalled();
    expect(await startButton(3)).toBeEnabled();

    await user.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(onSave).toHaveBeenCalledTimes(2);
    expect(onSave.mock.calls[1]).toEqual(onSave.mock.calls[0]);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('Change opens Discover to pick a lead; a ticked member picked there leaves the list', async () => {
    const user = userEvent.setup();
    const { onSave } = renderStep();
    await user.click(await screen.findByRole('button', { name: 'Choose another agent to lead your thread' }));

    const sheet = await screen.findByRole('dialog', { name: 'Choose who leads your thread' });
    const store = within(sheet).getByTestId('store-pick');
    expect(store).toHaveAttribute('data-mode', 'pickLead');
    expect(store).toHaveAttribute('data-picked', 'agent-endurance');
    await user.click(within(sheet).getByRole('button', { name: 'pick fuelling' }));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(within(screen.getByTestId('team-lead')).getByText('Fuelling Agent')).toBeInTheDocument();
    expect(screen.queryByRole('checkbox', { name: /Fuelling Agent/ })).not.toBeInTheDocument();
    await user.click(await startButton(2));
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith({
        status: 'complete',
        leadAgentId: 'agent-fuel',
        teamAgentIds: ['agent-fuel', 'agent-recovery'],
      }),
    );
  });

  it('Browse all agents opens Discover to add to the team; the pick joins the list ticked', async () => {
    const user = userEvent.setup();
    const { onSave } = renderStep();
    await user.click(await screen.findByRole('button', { name: 'Browse all agents' }));

    const sheet = await screen.findByRole('dialog', { name: 'All agents' });
    const store = within(sheet).getByTestId('store-pick');
    expect(store).toHaveAttribute('data-mode', 'pickTeam');
    expect(store).toHaveAttribute('data-picked', 'agent-endurance,agent-fuel,agent-recovery');
    await user.click(within(sheet).getByRole('button', { name: 'pick mind' }));
    // The sheet stays open for a second pick; what the step holds follows.
    expect(within(sheet).getByTestId('store-pick')).toHaveAttribute(
      'data-picked',
      'agent-endurance,agent-fuel,agent-recovery,agent-mind',
    );
    await user.click(within(sheet).getByRole('button', { name: 'Close' }));

    expect(screen.getByRole('checkbox', { name: /Mind Agent/ })).toBeChecked();
    expect(screen.getByText('Custom · added from Discover')).toBeInTheDocument();
    await user.click(await startButton(4));
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith({
        status: 'complete',
        leadAgentId: 'agent-endurance',
        teamAgentIds: ['agent-endurance', 'agent-fuel', 'agent-recovery', 'agent-mind'],
      }),
    );
  });

  it('a cold start with no member says so and still starts with the lead', async () => {
    getProposal.mockResolvedValue({
      profile: { has_profile: false, window_days: 90, total_activities: 0, sport_mix: [] },
      agents: PROPOSAL.agents,
      team: { lead: PROPOSAL.team.lead, members: [] },
    });
    renderStep();
    expect(await screen.findByText('No other agent fits your data yet. Browse all agents to add one.')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'What we read' })).toHaveTextContent(/didn't find recent activities/);
    expect(await startButton(1)).toBeInTheDocument();
  });

  it('a server that predates teams still leads with its first proposal', async () => {
    getProposal.mockResolvedValue({ profile: PROPOSAL.profile, agents: PROPOSAL.agents });
    const user = userEvent.setup();
    const { onSave } = renderStep();
    await user.click(await startButton(1));
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith({
        status: 'complete',
        leadAgentId: 'agent-endurance',
        teamAgentIds: ['agent-endurance'],
      }),
    );
  });

  it('Continue after a failed proposal records the step without an agent and goes on', async () => {
    getProposal.mockRejectedValue(new Error('network'));
    const { onSave, onComplete } = renderStep();
    const user = userEvent.setup();
    await user.click(await screen.findByRole('button', { name: 'Continue to dashboard' }, { timeout: 5000 }));

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0]).toEqual([]);
    expect(onSave).toHaveBeenCalledWith({ status: 'complete' });
  });

  // Found by the 2026-10-23 onboarding e2e on dev: an athlete who continued
  // without connecting was told their training data was being analyzed and
  // that suggestions would sharpen "as your provider syncs".
  it('never claims to read the training of an athlete who connected nothing', async () => {
    getProposal.mockResolvedValue({
      ...PROPOSAL,
      goal: undefined,
      profile: { has_profile: false, window_days: 90, total_activities: 0, sport_mix: [] },
    });
    renderStep({ withoutProvider: true });
    expect(screen.getByText('Putting your starting team together…')).toBeInTheDocument();
    expect(screen.queryByText('Analyzing your training data…')).not.toBeInTheDocument();

    await startButton(3);
    expect(
      screen.getByText('A few agents to start with. Connect a provider and your picks will follow your training.'),
    ).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'What we read' })).toHaveTextContent(
      'No provider connected yet — here are some agents to start with.',
    );
    expect(screen.queryByText(/as your provider syncs/)).not.toBeInTheDocument();
  });
});
