// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the group agent's one-time chat-app card — who meets it, and what each answer does
// ABOUTME: Only the owning coach, only while an app is offered, none is linked and the offer was never answered (carnet#1525)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachingGroup } from '@pierre/shared-types';
import { GroupChatAppOffer } from '../GroupChatAppOffer';
import { useGroupChatAppOffer } from '../useGroupChatAppOffer';

const api = vi.hoisted(() => ({
  getOnboardingStatus: vi.fn(),
  setOnboardingStep: vi.fn(),
  getAvailableChannels: vi.fn(),
  listLinks: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  userApi: { getOnboardingStatus: api.getOnboardingStatus, setOnboardingStep: api.setOnboardingStep },
  messagingLinkApi: { getAvailableChannels: api.getAvailableChannels, listLinks: api.listLinks },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'coach-1' } }),
}));

const TELEGRAM = { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true };
const WHATSAPP = { channel: 'whatsapp', display_name: 'WhatsApp', method: 'deep_link', recommended: false };

function group(overrides: Partial<CoachingGroup> = {}): CoachingGroup {
  return {
    id: 'g-1',
    tenant_id: 't-1',
    name: 'Léa · Marathon de Montréal',
    description: null,
    agent_id: 'agent-endurance',
    agent_title: 'Endurance',
    agent_handle: 'endurance',
    owner_id: 'coach-1',
    coach_user_id: 'coach-1',
    coach_display_name: 'Philippe',
    peer_data_sharing: false,
    respond_mode: 'all',
    digest_mode: 'off',
    max_members: 10,
    is_active: true,
    created_at: '2026-11-12T09:00:00Z',
    updated_at: '2026-11-12T09:00:00Z',
    ...overrides,
  };
}

function status(steps: { step_id: string; status: string }[] = [], coachesOthers = true) {
  return {
    needs_provider_connection: false,
    pillars_covered: 0,
    pillars_total: 7,
    onboarding_complete: false,
    steps,
    chosen_channel: null,
    coaches_others: coachesOthers,
  };
}

function Harness({ group: g, onConfigure }: { group: CoachingGroup | null; onConfigure: (c: string) => void }) {
  const offer = useGroupChatAppOffer(g);
  return offer ? <GroupChatAppOffer offer={offer} agentLabel="Endurance" onConfigure={onConfigure} /> : null;
}

function renderOffer(g: CoachingGroup | null = group()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onConfigure = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <Harness group={g} onConfigure={onConfigure} />
    </QueryClientProvider>,
  );
  return { onConfigure, client };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.getOnboardingStatus.mockResolvedValue(status());
  api.getAvailableChannels.mockResolvedValue([TELEGRAM, WHATSAPP]);
  api.listLinks.mockResolvedValue([]);
  api.setOnboardingStep.mockResolvedValue(undefined);
});

describe('GroupChatAppOffer', () => {
  it('offers the recommended app to the coach who owns the group, in the agent’s voice', async () => {
    const { onConfigure } = renderOffer();
    const card = await screen.findByTestId('group-chat-app-offer');
    expect(card).toHaveTextContent('Write to me from Telegram too?');
    expect(screen.getByText('Endurance')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Set up Telegram' }));
    expect(onConfigure).toHaveBeenCalledExactlyOnceWith('telegram');
    // Configuring answers nothing yet: only a landed link or « Not now » does.
    expect(api.setOnboardingStep).not.toHaveBeenCalled();
  });

  it('stores « Not now » on the onboarding record, so no device shows it again', async () => {
    // The server keeps the answer: the next status read carries it.
    api.setOnboardingStep.mockImplementation(async () => {
      api.getOnboardingStatus.mockResolvedValue(status([{ step_id: 'messaging_configure', status: 'skipped' }]));
    });
    renderOffer();
    await userEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('messaging_configure', 'skipped');
    await waitFor(() => expect(screen.queryByTestId('group-chat-app-offer')).toBeNull());
  });

  it('keeps the card up with the error when « Not now » was not stored', async () => {
    api.setOnboardingStep.mockRejectedValue(new Error('offline'));
    renderOffer();
    await userEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Your choice wasn’t saved. Try again.');
    expect(screen.getByTestId('group-chat-app-offer')).toBeInTheDocument();
  });

  it('is never shown again once answered, on any device', async () => {
    for (const answer of ['skipped', 'complete']) {
      api.getOnboardingStatus.mockResolvedValue(status([{ step_id: 'messaging_configure', status: answer }]));
      const { client } = renderOffer();
      await waitFor(() => expect(api.getOnboardingStatus).toHaveBeenCalled());
      expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
      client.clear();
    }
    // The answer settles it: the apps and links are not even read.
    expect(api.getAvailableChannels).not.toHaveBeenCalled();
  });

  it('is not shown to a coach who already linked an app', async () => {
    api.listLinks.mockResolvedValue([
      { channel: 'whatsapp', channel_user_id: 'wa-1', display_name: null, linked_at: '2026-11-01T10:00:00Z' },
    ]);
    renderOffer();
    await waitFor(() => expect(api.listLinks).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
  });

  it('is not shown when the workspace offers no app', async () => {
    api.getAvailableChannels.mockResolvedValue([]);
    renderOffer();
    await waitFor(() => expect(api.getAvailableChannels).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
  });

  it('is not shown in someone else’s group, nor to a member who does not coach', async () => {
    renderOffer(group({ owner_id: 'someone-else' }));
    await waitFor(() => expect(api.getOnboardingStatus).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
    expect(api.getAvailableChannels).not.toHaveBeenCalled();
  });

  it('is not shown to an owner who answered they do not coach', async () => {
    api.getOnboardingStatus.mockResolvedValue(status([], false));
    renderOffer();
    await waitFor(() => expect(api.getOnboardingStatus).toHaveBeenCalled());
    expect(screen.queryByTestId('group-chat-app-offer')).toBeNull();
  });

  it('continues the agent’s run under its own last words: no second name', () => {
    const offer = { app: TELEGRAM, decline: vi.fn() };
    const { rerender } = render(<GroupChatAppOffer offer={offer} agentLabel="Endurance" onConfigure={vi.fn()} />);
    expect(screen.getByText('Endurance')).toBeInTheDocument();
    rerender(<GroupChatAppOffer offer={offer} agentLabel="Endurance" continuesRun onConfigure={vi.fn()} />);
    expect(screen.queryByText('Endurance')).toBeNull();
    expect(screen.getByTestId('group-chat-app-offer')).toHaveTextContent('Write to me from Telegram too?');
  });

  it('reads nothing outside a group thread', () => {
    renderOffer(null);
    expect(api.getOnboardingStatus).not.toHaveBeenCalled();
    expect(api.getAvailableChannels).not.toHaveBeenCalled();
  });
});
