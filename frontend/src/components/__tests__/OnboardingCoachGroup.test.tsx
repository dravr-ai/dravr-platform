// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingCoachGroup — the coach names a group, picks its agent, and leaves with its invite link
// ABOUTME: Pins the name → agent → create → thread → 30-day invite sequence, the access-pending state, retry and skip

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
} = vi.hoisted(() => ({
  createGroupMock: vi.fn(),
  createInviteMock: vi.fn(),
  createConversationMock: vi.fn(),
  listAgentsMock: vi.fn(),
  getCoachAccessRequestMock: vi.fn(),
  requestCoachAccessMock: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  groupsApi: { createGroup: createGroupMock, createInvite: createInviteMock },
  chatApi: { createConversation: createConversationMock },
  coachesApi: { list: listAgentsMock },
  userApi: {
    getCoachAccessRequest: getCoachAccessRequestMock,
    requestCoachAccess: requestCoachAccessMock,
  },
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

const group = (coachUserId: string | null) => ({
  id: 'g-1',
  tenant_id: 't-1',
  name: 'Les Rouleurs',
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
  ...extra,
});

function renderStep() {
  const onComplete = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingCoachGroup onComplete={onComplete} />
    </QueryClientProvider>,
  );
  return { onComplete };
}

async function nameTheGroup() {
  await userEvent.type(screen.getByTestId('onboarding-group-name'), 'Les Rouleurs');
  await userEvent.click(screen.getByRole('button', { name: 'Next' }));
}

async function nameAndCreate() {
  await nameTheGroup();
  await userEvent.click(await screen.findByTestId('onboarding-group-agent-a-2'));
  await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));
}

describe('OnboardingCoachGroup', () => {
  beforeEach(() => {
    createGroupMock.mockReset().mockResolvedValue(group('u-1'));
    createConversationMock.mockReset().mockResolvedValue({ id: 'c-1' });
    createInviteMock.mockReset().mockResolvedValue({ code: 'ABCD2345' });
    getCoachAccessRequestMock.mockReset().mockResolvedValue({ request: null });
    requestCoachAccessMock.mockReset();
    listAgentsMock.mockReset().mockResolvedValue({
      agents: [
        agent('a-1', 'Endurance Agent'),
        agent('a-2', 'Triathlon Agent'),
        agent('a-3', 'Hidden Agent', { is_hidden: true }),
        agent('a-4', 'Roster Agent', { tags: ['coach-tool'] }),
      ],
      total: 4,
      metadata: {},
    });
  });

  it('creates the group with the chosen agent, opens its thread and issues a 30-day invite', async () => {
    renderStep();
    await nameAndCreate();

    await waitFor(() =>
      expect(createGroupMock).toHaveBeenCalledWith({
        name: 'Les Rouleurs',
        agent_id: 'a-2',
        coach_is_me: true,
      }),
    );
    expect(createConversationMock).toHaveBeenCalledWith({ group_id: 'g-1', agent_id: 'a-2' });
    expect(createInviteMock).toHaveBeenCalledWith('g-1', { expires_in_days: 30 });

    expect(await screen.findByTestId('onboarding-group-link')).toHaveTextContent(
      '/groups/join/ABCD2345',
    );
    expect(await screen.findByTestId('onboarding-group-qr')).toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-group-access-pending')).not.toBeInTheDocument();
  });

  it('marks the cached chat list stale once the group thread exists, so the group is on it', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData(QUERY_KEYS.chat.conversations(), { pages: [], pageParams: [] });
    render(
      <QueryClientProvider client={client}>
        <OnboardingCoachGroup onComplete={vi.fn()} />
      </QueryClientProvider>,
    );
    await nameAndCreate();

    await waitFor(() => expect(createConversationMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(client.getQueryState(QUERY_KEYS.chat.conversations())?.isInvalidated).toBe(true),
    );
  });

  it('offers the visible athlete-facing catalogue unranked and creates nothing until an agent is picked', async () => {
    renderStep();
    await nameTheGroup();

    expect(await screen.findByTestId('onboarding-group-agent-a-1')).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-group-agent-a-2')).toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-group-agent-a-3')).not.toBeInTheDocument();
    // A coach-facing agent never answers a group's athletes.
    expect(screen.queryByTestId('onboarding-group-agent-a-4')).not.toBeInTheDocument();
    expect(screen.getAllByRole('radio').every((r) => r.getAttribute('aria-checked') === 'false')).toBe(
      true,
    );
    expect(screen.getByRole('button', { name: 'Create the group' })).toBeDisabled();
    expect(createGroupMock).not.toHaveBeenCalled();
  });

  it('goes back to the name keeping what was typed', async () => {
    renderStep();
    await nameTheGroup();
    await userEvent.click(await screen.findByRole('button', { name: 'Back' }));
    expect(screen.getByTestId('onboarding-group-name')).toHaveValue('Les Rouleurs');
  });

  it('says coach access is pending when the group comes back without a coach', async () => {
    createGroupMock.mockResolvedValue(group(null));
    renderStep();
    await nameAndCreate();

    expect(await screen.findByTestId('onboarding-group-access-pending')).toHaveTextContent(
      'Coach access pending',
    );
    expect(document.querySelector('a[href^="mailto:"]')).toBeNull();
  });

  it('asks for coach access in one tap, naming the group, then says the request was sent', async () => {
    createGroupMock.mockResolvedValue(group(null));
    requestCoachAccessMock.mockResolvedValue({ request: accessRequest('pending') });
    renderStep();
    await nameAndCreate();

    const button = await screen.findByTestId('coach-access-request');
    await waitFor(() => expect(button).toBeEnabled());
    await userEvent.click(button);

    expect(requestCoachAccessMock).toHaveBeenCalledWith('g-1');
    expect(await screen.findByTestId('coach-access-pending')).toHaveTextContent('Request sent');
    expect(screen.queryByTestId('coach-access-request')).not.toBeInTheDocument();
  });

  it('shows a request already waiting instead of the button', async () => {
    createGroupMock.mockResolvedValue(group(null));
    getCoachAccessRequestMock.mockResolvedValue({ request: accessRequest('pending') });
    renderStep();
    await nameAndCreate();

    expect(await screen.findByTestId('coach-access-pending')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-access-request')).not.toBeInTheDocument();
  });

  it('offers the button again after a declined request', async () => {
    createGroupMock.mockResolvedValue(group(null));
    getCoachAccessRequestMock.mockResolvedValue({ request: accessRequest('declined') });
    renderStep();
    await nameAndCreate();

    expect(await screen.findByTestId('coach-access-declined')).toHaveTextContent('declined');
    expect(screen.getByTestId('coach-access-request')).toBeInTheDocument();
  });

  it('keeps the button with an error when the request fails, and retries', async () => {
    createGroupMock.mockResolvedValue(group(null));
    requestCoachAccessMock
      .mockRejectedValueOnce(new Error('network'))
      .mockResolvedValueOnce({ request: accessRequest('pending') });
    renderStep();
    await nameAndCreate();

    const button = await screen.findByTestId('coach-access-request');
    await waitFor(() => expect(button).toBeEnabled());
    await userEvent.click(button);
    expect(await screen.findByRole('alert')).toHaveTextContent("We couldn't send your request");

    await userEvent.click(screen.getByTestId('coach-access-request'));
    expect(await screen.findByTestId('coach-access-pending')).toBeInTheDocument();
    expect(requestCoachAccessMock).toHaveBeenCalledTimes(2);
  });

  it('completes the step from the share screen and opens the group thread', async () => {
    window.location.hash = '';
    const { onComplete } = renderStep();
    await nameAndCreate();
    const done = await screen.findByTestId('onboarding-group-done');
    expect(done).toHaveTextContent('Go to my group');
    // The hash is the dashboard's hand-off: it is written only once the step is recorded.
    onComplete.mockImplementation(() => expect(window.location.hash).toBe(''));
    await userEvent.click(done);
    expect(onComplete).toHaveBeenCalledWith('complete');
    expect(window.location.hash).toBe('#chat/c-1');
    window.location.hash = '';
  });

  it('keeps the coach on the step after a failure and retries without a second group', async () => {
    createInviteMock.mockRejectedValueOnce(new Error('network'));
    renderStep();
    await nameAndCreate();

    expect(await screen.findByText("We couldn't create the group. Try again.")).toBeInTheDocument();
    // The group exists now: its name and agent can no longer change.
    expect(screen.queryByRole('button', { name: 'Back' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));

    expect(await screen.findByTestId('onboarding-group-link')).toBeInTheDocument();
    expect(createGroupMock).toHaveBeenCalledTimes(1);
    // Nor a second thread: the group's one thread is the one "Go to my group" opens.
    expect(createConversationMock).toHaveBeenCalledTimes(1);
  });

  it('says so when the agents cannot be loaded', async () => {
    listAgentsMock.mockRejectedValue(new Error('network'));
    renderStep();
    await nameTheGroup();
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

  it('cannot move on without a name', () => {
    renderStep();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
  });
});
