// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingCoachGroup — the athlete, the group's name and its agent on one screen, then the invite
// ABOUTME: Pins the create → thread → 30-day invite sequence, the honest seat card, the QR behind a link, retry and skip

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '../../constants/queryKeys';
import OnboardingCoachGroup from '../OnboardingCoachGroup';

const {
  createGroupMock,
  createInviteMock,
  createConversationMock,
  listAgentsMock,
  getCoachAccessRequestMock,
  requestCoachAccessMock,
  authState,
} = vi.hoisted(() => ({
  createGroupMock: vi.fn(),
  createInviteMock: vi.fn(),
  createConversationMock: vi.fn(),
  listAgentsMock: vi.fn(),
  getCoachAccessRequestMock: vi.fn(),
  requestCoachAccessMock: vi.fn(),
  authState: { managesRoster: false },
}));

vi.mock('../../services/api', () => ({
  groupsApi: { createGroup: createGroupMock, createInvite: createInviteMock },
  chatApi: { createConversation: createConversationMock },
  coachesApi: { list: listAgentsMock },
  userApi: {
    getCoachAccessRequest: getCoachAccessRequestMock,
    requestCoachAccess: requestCoachAccessMock,
  },
  authApi: { getSession: vi.fn() },
  providersApi: { getProvidersStatus: vi.fn() },
}));
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { manages_roster: authState.managesRoster }, updateUser: vi.fn() }),
}));

const accessRequest = (status: 'pending' | 'granted' | 'declined') => ({
  id: 'r-1',
  user_id: 'u-1',
  group_id: 'g-1',
  group_tenant_id: 't-1',
  status,
  created_at: '2026-10-07T12:00:00Z',
  decided_at: null,
  decided_by: null,
});

vi.mock('qrcode', () => ({
  default: { toDataURL: vi.fn().mockResolvedValue('data:image/png;base64,QR') },
}));

const group = (coachUserId: string | null, name = 'Léa · Marathon de Montréal') => ({
  id: 'g-1',
  tenant_id: 't-1',
  name,
  description: null,
  agent_id: 'a-2',
  owner_id: 'u-1',
  coach_user_id: coachUserId,
});

const agent = (id: string, title: string, extra: Record<string, unknown> = {}) => ({
  id,
  title,
  description: `${title} description`,
  category: 'training',
  is_hidden: false,
  tags: [],
  ...extra,
});

function renderStep({ seatSettledByInvite = false }: { seatSettledByInvite?: boolean } = {}) {
  const onComplete = vi.fn();
  const onCreated = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingCoachGroup onCreated={onCreated} onComplete={onComplete} seatSettledByInvite={seatSettledByInvite} />
    </QueryClientProvider>,
  );
  return { onComplete, onCreated, client };
}

/** The athlete and their goal; the group's name follows them. */
async function describeTheAthlete() {
  await userEvent.type(screen.getByTestId('onboarding-group-athlete'), 'Léa');
  await userEvent.type(screen.getByTestId('onboarding-group-goal'), 'Marathon de Montréal');
}

async function createTheGroup() {
  await describeTheAthlete();
  await screen.findByTestId('onboarding-group-agent');
  await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));
}

describe('OnboardingCoachGroup — one screen for the group', () => {
  beforeEach(() => {
    authState.managesRoster = false;
    createGroupMock.mockReset().mockResolvedValue(group('u-1'));
    createConversationMock.mockReset().mockResolvedValue({ id: 'c-1' });
    createInviteMock.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    getCoachAccessRequestMock.mockReset().mockResolvedValue({ request: null });
    requestCoachAccessMock.mockReset();
    listAgentsMock.mockReset().mockResolvedValue({
      agents: [
        agent('a-1', 'Road Running Agent', { tags: ['running'] }),
        agent('a-2', 'Endurance Agent', { tags: ['endurance'] }),
        agent('a-3', 'Hidden Agent', { is_hidden: true }),
        agent('a-4', 'Roster Agent', { tags: ['coach-tool'] }),
      ],
      total: 4,
      metadata: {},
    });
  });

  it('fills the group name in from the athlete and goal until the coach types their own', async () => {
    renderStep();
    await describeTheAthlete();
    expect(screen.getByTestId('onboarding-group-name')).toHaveValue('Léa · Marathon de Montréal');

    await userEvent.clear(screen.getByTestId('onboarding-group-name'));
    await userEvent.type(screen.getByTestId('onboarding-group-name'), 'Les Rouleurs');
    await userEvent.type(screen.getByTestId('onboarding-group-athlete'), 'h');
    expect(screen.getByTestId('onboarding-group-name')).toHaveValue('Les Rouleurs');
  });

  it('never greets the coach a third time', () => {
    renderStep();
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Set up your group');
    expect(screen.queryByText(/Welcome/)).not.toBeInTheDocument();
  });

  it('defaults the agent to the endurance generalist and offers the rest behind Change', async () => {
    renderStep();
    expect(await screen.findByTestId('onboarding-group-agent')).toHaveTextContent('Endurance Agent');
    expect(screen.queryByTestId('onboarding-group-agents')).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Choose another agent for the group' }));
    expect(screen.getByTestId('onboarding-group-agent-a-1')).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-group-agent-a-2')).toHaveAttribute('aria-checked', 'true');
    expect(screen.queryByTestId('onboarding-group-agent-a-3')).not.toBeInTheDocument();
    // A coach-facing agent never answers a group's athletes.
    expect(screen.queryByTestId('onboarding-group-agent-a-4')).not.toBeInTheDocument();

    await userEvent.click(screen.getByTestId('onboarding-group-agent-a-1'));
    expect(screen.getByTestId('onboarding-group-agent')).toHaveTextContent('Road Running Agent');
  });

  it('creates the group with its agent and goal, opens its thread and issues a 30-day invite', async () => {
    const { onCreated } = renderStep();
    await createTheGroup();

    await waitFor(() =>
      expect(createGroupMock).toHaveBeenCalledWith({
        name: 'Léa · Marathon de Montréal',
        agent_id: 'a-2',
        coach_is_me: true,
        description: 'Marathon de Montréal',
      }),
    );
    expect(createConversationMock).toHaveBeenCalledWith({ group_id: 'g-1', agent_id: 'a-2' });
    expect(createInviteMock).toHaveBeenCalledWith('g-1', { expires_in_days: 30 });
    expect(await screen.findByTestId('onboarding-group-link')).toHaveTextContent('/groups/join/ABCD2345');
    expect(onCreated).toHaveBeenCalled();
  });

  it('marks the cached chat list stale once the group thread exists, so the group is on it', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData(QUERY_KEYS.chat.conversations(), { pages: [], pageParams: [] });
    render(
      <QueryClientProvider client={client}>
        <OnboardingCoachGroup onComplete={vi.fn()} />
      </QueryClientProvider>,
    );
    await createTheGroup();

    await waitFor(() => expect(createConversationMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(client.getQueryState(QUERY_KEYS.chat.conversations())?.isInvalidated).toBe(true),
    );
  });

  it('cannot create a group without a name', async () => {
    renderStep();
    await screen.findByTestId('onboarding-group-agent');
    expect(screen.getByRole('button', { name: 'Create the group' })).toBeDisabled();
  });

  it('keeps the coach on the screen after a failure and retries without a second group', async () => {
    createInviteMock.mockRejectedValueOnce(new Error('network'));
    renderStep();
    await createTheGroup();

    expect(await screen.findByText("We couldn't create the group. Try again.")).toBeInTheDocument();
    // The group exists now: its name and agent can no longer change, nor be put off.
    expect(screen.getByTestId('onboarding-group-name')).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Later' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));

    expect(await screen.findByTestId('onboarding-group-link')).toBeInTheDocument();
    expect(createGroupMock).toHaveBeenCalledTimes(1);
    // Nor a second thread: the group's one thread is the one "Go to" opens.
    expect(createConversationMock).toHaveBeenCalledTimes(1);
  });

  it('says so when the agents cannot be loaded', async () => {
    listAgentsMock.mockRejectedValue(new Error('network'));
    renderStep();
    await describeTheAthlete();
    expect(await screen.findByText('Failed to load agents')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create the group' })).toBeDisabled();
  });

  it('puts the step off with Later and creates nothing', async () => {
    window.location.hash = '';
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByRole('button', { name: 'Later' }));
    expect(onComplete).toHaveBeenCalledWith('skipped');
    expect(createGroupMock).not.toHaveBeenCalled();
    // Later lands where the dashboard defaults, not in a thread.
    expect(window.location.hash).toBe('');
  });
});

describe('OnboardingCoachGroup — the invite has one job', () => {
  beforeEach(() => {
    authState.managesRoster = false;
    createGroupMock.mockReset().mockResolvedValue(group('u-1'));
    createConversationMock.mockReset().mockResolvedValue({ id: 'c-1' });
    createInviteMock.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    getCoachAccessRequestMock.mockReset().mockResolvedValue({ request: null });
    requestCoachAccessMock.mockReset();
    listAgentsMock.mockReset().mockResolvedValue({
      agents: [agent('a-2', 'Endurance Agent', { tags: ['endurance'] })],
      total: 1,
      metadata: {},
    });
  });

  it('is named for the athlete, copies the link, and keeps the QR behind a link', async () => {
    renderStep();
    await createTheGroup();

    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent('Invite Léa');
    expect(screen.getByText(/Send this link to Léa: one tap joins “Léa · Marathon de Montréal”/)).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-group-copy')).toHaveTextContent('Copy link');
    expect(screen.queryByTestId('onboarding-group-qr')).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Show QR code' }));
    expect(await screen.findByTestId('onboarding-group-qr')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hide QR code' })).toHaveAttribute('aria-expanded', 'true');
  });

  it('on a phone with a share sheet, « Share link » is the filled button and hands the link over', async () => {
    const share = vi.fn().mockResolvedValue(undefined);
    const matchMedia = window.matchMedia;
    Object.defineProperty(navigator, 'share', { value: share, configurable: true });
    window.matchMedia = vi.fn().mockReturnValue({ matches: true }) as unknown as typeof window.matchMedia;
    try {
      renderStep();
      await createTheGroup();

      const button = await screen.findByTestId('onboarding-group-share');
      expect(button).toHaveTextContent('Share link');
      expect(screen.queryByTestId('onboarding-group-copy')).not.toBeInTheDocument();
      await userEvent.click(button);
      expect(share).toHaveBeenCalledWith({
        title: 'Léa · Marathon de Montréal',
        url: `${window.location.origin}/groups/join/ABCD2345`,
      });
    } finally {
      delete (navigator as { share?: unknown }).share;
      window.matchMedia = matchMedia;
    }
  });

  it('says the coach coaches the group when the server made them its coach', async () => {
    renderStep();
    await createTheGroup();

    const seat = await screen.findByTestId('onboarding-group-seat');
    expect(seat).toHaveAttribute('data-state', 'coach');
    expect(seat).toHaveTextContent("You're this group's coach");
    expect(screen.queryByTestId('coach-access-request')).not.toBeInTheDocument();
  });

  it('never says pending before a request exists: not yet the coach, with the quiet request', async () => {
    createGroupMock.mockResolvedValue(group(null));
    renderStep();
    await createTheGroup();

    const seat = await screen.findByTestId('onboarding-group-seat');
    await waitFor(() => expect(seat).toHaveAttribute('data-state', 'none'));
    expect(seat).toHaveTextContent("You're not this group's coach yet");
    expect(seat).toHaveTextContent("You haven't asked for your seat yet.");
    expect(seat).not.toHaveTextContent(/pending/i);
    expect(document.querySelector('a[href^="mailto:"]')).toBeNull();
  });

  it('asks for coach access in one tap, naming the group, then shows the dated request', async () => {
    createGroupMock.mockResolvedValue(group(null));
    requestCoachAccessMock.mockResolvedValue({ request: accessRequest('pending') });
    renderStep();
    await createTheGroup();

    const button = await screen.findByTestId('coach-access-request');
    await waitFor(() => expect(button).toBeEnabled());
    await userEvent.click(button);

    expect(requestCoachAccessMock).toHaveBeenCalledWith('g-1');
    const seat = screen.getByTestId('onboarding-group-seat');
    await waitFor(() => expect(seat).toHaveAttribute('data-state', 'requested'));
    expect(seat).toHaveTextContent(/Request sent .*2026\. We'll let you know here and by email\./);
    expect(seat.textContent).not.toMatch(/24/);
    expect(screen.queryByTestId('coach-access-request')).not.toBeInTheDocument();
  });

  it('shows a request already waiting, with its date, instead of the button', async () => {
    createGroupMock.mockResolvedValue(group(null));
    getCoachAccessRequestMock.mockResolvedValue({ request: accessRequest('pending') });
    renderStep();
    await createTheGroup();

    const seat = await screen.findByTestId('onboarding-group-seat');
    await waitFor(() => expect(seat).toHaveAttribute('data-state', 'requested'));
    expect(screen.queryByTestId('coach-access-request')).not.toBeInTheDocument();
  });

  it('offers the request again after a declined one', async () => {
    createGroupMock.mockResolvedValue(group(null));
    getCoachAccessRequestMock.mockResolvedValue({ request: accessRequest('declined') });
    renderStep();
    await createTheGroup();

    expect(await screen.findByTestId('coach-access-declined')).toHaveTextContent('declined');
    expect(screen.getByTestId('coach-access-request')).toBeInTheDocument();
  });

  it('without coach access, completes the step and opens the group thread from « Go to Léa’s group »', async () => {
    window.location.hash = '';
    createGroupMock.mockResolvedValue(group(null));
    const { onComplete } = renderStep();
    await createTheGroup();
    const done = await screen.findByTestId('onboarding-group-done');
    expect(done).toHaveTextContent("Go to Léa's group");
    // The hash is the dashboard's hand-off: it is written only once the step is recorded.
    onComplete.mockImplementation(() => expect(window.location.hash).toBe(''));
    await userEvent.click(done);
    expect(onComplete).toHaveBeenCalledWith('complete');
    expect(window.location.hash).toBe('#chat/c-1');
    window.location.hash = '';
  });

  it('with a TrainingPeaks seat, the coach of the group leaves for the coach Home', async () => {
    window.location.hash = '';
    const { onComplete } = renderStep();
    await createTheGroup();
    const done = await screen.findByTestId('onboarding-group-done');
    expect(done).toHaveTextContent('Open my coach Home');
    // Only the invitation says the seat was granted by the team.
    expect(screen.queryByTestId('onboarding-group-seat-active')).toBeNull();
    onComplete.mockImplementation(() => expect(window.location.hash).toBe(''));
    await userEvent.click(done);
    expect(onComplete).toHaveBeenCalledWith('complete');
    expect(window.location.hash).toBe('#home');
    window.location.hash = '';
  });

  it('without an athlete name, keeps the generic heading and "Go to my group"', async () => {
    createGroupMock.mockResolvedValue(group(null, 'Les Rouleurs'));
    renderStep();
    await userEvent.type(screen.getByTestId('onboarding-group-name'), 'Les Rouleurs');
    await screen.findByTestId('onboarding-group-agent');
    await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));

    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent('Invite your athlete');
    expect(screen.getByTestId('onboarding-group-done')).toHaveTextContent('Go to my group');
  });
});

describe('OnboardingCoachGroup — a coach the team invited with their seat', () => {
  beforeEach(() => {
    authState.managesRoster = true;
    createGroupMock.mockReset().mockResolvedValue(group('u-1'));
    createConversationMock.mockReset().mockResolvedValue({ id: 'c-1' });
    createInviteMock.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    getCoachAccessRequestMock.mockReset().mockResolvedValue({ request: null });
    requestCoachAccessMock.mockReset();
    listAgentsMock.mockReset().mockResolvedValue({
      agents: [agent('a-2', 'Endurance Agent', { tags: ['endurance'] })],
      total: 1,
      metadata: {},
    });
  });

  it('says on the group screen that the seat is already active', () => {
    renderStep({ seatSettledByInvite: true });
    expect(screen.getByTestId('onboarding-group-seat-active')).toHaveTextContent(
      'Your coach seat is active: granted by the Dravr team.',
    );
  });

  it('leaves the invite for the coach Home once the step is recorded', async () => {
    window.location.hash = '';
    const { onComplete } = renderStep({ seatSettledByInvite: true });
    await createTheGroup();
    const done = await screen.findByTestId('onboarding-group-done');
    expect(done).toHaveTextContent('Open my coach Home');
    onComplete.mockImplementation(() => expect(window.location.hash).toBe(''));
    await userEvent.click(done);
    expect(onComplete).toHaveBeenCalledWith('complete');
    expect(window.location.hash).toBe('#home');
    window.location.hash = '';
  });

  it('a coach who answered for themselves meets no seat-granted line', () => {
    renderStep();
    expect(screen.queryByTestId('onboarding-group-seat-active')).toBeNull();
  });
});
