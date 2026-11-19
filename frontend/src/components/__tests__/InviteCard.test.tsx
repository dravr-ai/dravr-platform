// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the invite card a /groups/join/<code> link opens on, signed in or not, before any form (carnet#1649)
// ABOUTME: Red if the card reads the link more than once, joins before the click, keeps a dead code, or shows a second filled button

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { GroupInvitePreview } from '@pierre/shared-types';
import InviteCard from '../InviteCard';
import { readCarriedGroupInvite } from '../../utils/groupInvite';

const previewInvite = vi.fn<(code: string) => Promise<GroupInvitePreview>>();

vi.mock('../../services/api', () => ({
  invitePreviewApi: { previewInvite: (code: string) => previewInvite(code) },
}));

const CARRY_KEY = 'dravr.pending_invite_code';

const LIVE: GroupInvitePreview = {
  status: 'live',
  kind: 'member',
  inviter_name: 'Jeremy B.',
  inviter_initials: 'JB',
  has_coach: true,
  coach_name: 'Jeremy B.',
  group_name: 'Morning Riders',
  expires_at: '2099-12-13T14:40:00Z',
};

function refusal(status: number) {
  return { isAxiosError: true, response: { status, data: {} } };
}

function renderCard(overrides: Partial<React.ComponentProps<typeof InviteCard>> = {}) {
  const props = {
    code: 'MRT2026X',
    signedIn: false,
    onCreateAccount: vi.fn(),
    onSignIn: vi.fn(),
    onJoin: vi.fn().mockResolvedValue('joined'),
    onDone: vi.fn(),
    ...overrides,
  };
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <InviteCard {...props} />
    </QueryClientProvider>,
  );
  return { ...view, ...props, client };
}

describe('InviteCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
  });

  it('opens a signed-out visitor on who invited them, the group, what joining shares and the one filled button', async () => {
    previewInvite.mockResolvedValue(LIVE);
    renderCard();

    expect(await screen.findByTestId('invite-card-title')).toHaveTextContent('Jeremy B. invites you');
    expect(screen.getByTestId('invite-card-avatar')).toHaveTextContent('JB');
    expect(screen.getByTestId('invite-card-group')).toHaveTextContent('Morning Riders');
    expect(screen.getByTestId('invite-card-shares')).toHaveTextContent(
      'When you join, Jeremy B. will see your activities, your load and your plan.',
    );
    expect(screen.getByTestId('invite-card-valid-until')).toHaveTextContent('Link valid until');
    expect(screen.getByRole('button', { name: 'Create my account and join' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'I already have an account' })).toBeInTheDocument();
    expect(previewInvite).toHaveBeenCalledExactlyOnceWith('MRT2026X');
  });

  it('opens the create-account form from the filled button and the sign-in form from the ink one', async () => {
    previewInvite.mockResolvedValue(LIVE);
    const { onCreateAccount, onSignIn } = renderCard();

    await userEvent.click(await screen.findByTestId('invite-card-create'));
    await userEvent.click(screen.getByTestId('invite-card-have-account'));
    expect(onCreateAccount).toHaveBeenCalledTimes(1);
    expect(onSignIn).toHaveBeenCalledTimes(1);
  });

  it('says what joining shares about the group coach, not the inviter', async () => {
    previewInvite.mockResolvedValue({ ...LIVE, coach_name: 'Ana L.' });
    renderCard();

    expect(await screen.findByTestId('invite-card-title')).toHaveTextContent('Jeremy B. invites you');
    expect(screen.getByTestId('invite-card-shares')).toHaveTextContent(
      'When you join, Ana L. will see your activities, your load and your plan.',
    );
  });

  it('promises no sharing in a group that has no coach', async () => {
    previewInvite.mockResolvedValue({ ...LIVE, has_coach: false, coach_name: null });
    renderCard();

    expect(await screen.findByTestId('invite-card-shares')).toHaveTextContent('No human coach in this group yet.');
  });

  it('says a coach link is not for an account without coach access, and carries on', async () => {
    previewInvite.mockResolvedValue({ ...LIVE, kind: 'coach' });
    const { onDone } = renderCard({ signedIn: true, onJoin: vi.fn().mockResolvedValue('not_for_account') });

    await userEvent.click(await screen.findByTestId('invite-card-join'));
    expect(await screen.findByTestId('invite-card-not-for-account')).toHaveTextContent(
      'This link adds a coach to the group, and your account has no coach access.',
    );
    expect(screen.queryByText(/turned this link off/)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Continue without joining' }));
    expect(onDone).toHaveBeenCalled();
  });

  it('names the group, never a stand-in, when the inviter has no name', async () => {
    previewInvite.mockResolvedValue({ ...LIVE, inviter_name: null, inviter_initials: null });
    renderCard();

    expect(await screen.findByTestId('invite-card-title')).toHaveTextContent('An invite to “Morning Riders”');
    expect(screen.queryByTestId('invite-card-avatar')).not.toBeInTheDocument();
    expect(screen.queryByTestId('invite-card-group')).not.toBeInTheDocument();
  });

  it('words a coach seat for a coach, with no sharing promise to an athlete', async () => {
    previewInvite.mockResolvedValue({ ...LIVE, kind: 'coach', expires_at: null });
    renderCard();

    expect(await screen.findByTestId('invite-card-title')).toHaveTextContent(
      'Jeremy B. invites you to coach this group',
    );
    expect(screen.getByTestId('invite-card-shares')).toHaveTextContent(
      "As the group's coach, you'll see the training of the athletes who share it with you.",
    );
    expect(screen.queryByTestId('invite-card-valid-until')).not.toBeInTheDocument();
  });

  it('asks a signed-in viewer for the join, and runs it only on the click', async () => {
    previewInvite.mockResolvedValue(LIVE);
    const { onJoin, onDone, onCreateAccount } = renderCard({ signedIn: true });

    const join = await screen.findByTestId('invite-card-join');
    expect(screen.queryByTestId('invite-card-have-account')).not.toBeInTheDocument();
    expect(onJoin).not.toHaveBeenCalled();

    await userEvent.click(join);
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(onJoin).toHaveBeenCalledTimes(1);
    expect(onCreateAccount).not.toHaveBeenCalled();
  });

  it('stays on the card with a retry when the join did not answer', async () => {
    previewInvite.mockResolvedValue(LIVE);
    const onJoin = vi.fn().mockResolvedValue('failed');
    const { onDone } = renderCard({ signedIn: true, onJoin });

    await userEvent.click(await screen.findByTestId('invite-card-join'));
    expect(await screen.findByTestId('invite-card-join-failed')).toHaveTextContent("Couldn't join the group.");
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByTestId('invite-card-join')).toBeEnabled();
  });

  it.each([
    ['expired', 'This link has expired. Ask Jeremy B. to send you a new one.'],
    ['used_up', 'This link has been used as many times as it allows. Ask Jeremy B. to send you a new one.'],
    ['inactive', 'Jeremy B. turned this link off. Ask them for a new one.'],
  ] as const)('says a %s link is dead, drops its code and carries on', async (status, sentence) => {
    previewInvite.mockResolvedValue({ ...LIVE, status });
    const { onDone } = renderCard();

    expect(await screen.findByTestId('invite-card-dead')).toHaveTextContent(sentence);
    expect(screen.queryByTestId('invite-card-create')).not.toBeInTheDocument();
    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());

    await userEvent.click(screen.getByRole('button', { name: 'Continue without joining' }));
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('says an unknown code does not exist, drops it, and does not ask again', async () => {
    previewInvite.mockRejectedValue(refusal(404));
    renderCard();

    expect(await screen.findByTestId('invite-card-dead')).toHaveTextContent("This invite link doesn't exist.");
    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());
    expect(previewInvite).toHaveBeenCalledTimes(1);
  });

  it('says too many tries on a 429, keeps the code and reads again only on retry', async () => {
    previewInvite.mockRejectedValueOnce(refusal(429));
    renderCard();

    const error = await screen.findByTestId('invite-card-error');
    expect(error).toHaveTextContent('Too many tries.');
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
    expect(previewInvite).toHaveBeenCalledTimes(1);

    previewInvite.mockResolvedValueOnce(LIVE);
    await userEvent.click(within(document.body).getByTestId('invite-card-retry'));
    expect(await screen.findByTestId('invite-card-title')).toBeInTheDocument();
    expect(previewInvite).toHaveBeenCalledTimes(2);
  });

  it('says the invite could not be read on any other failure, with a retry and a way on', async () => {
    previewInvite.mockRejectedValue(refusal(500));
    const { onDone } = renderCard();

    expect(await screen.findByTestId('invite-card-error')).toHaveTextContent("Couldn't read this invite.");
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
    await userEvent.click(screen.getByTestId('invite-card-continue'));
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('shows the dead-link card when the join finds the link dead after all', async () => {
    previewInvite.mockResolvedValue(LIVE);
    const onJoin = vi.fn().mockResolvedValue('dead');
    renderCard({ signedIn: true, onJoin });

    await userEvent.click(await screen.findByTestId('invite-card-join'));
    expect(await screen.findByTestId('invite-card-dead')).toHaveTextContent('Jeremy B. turned this link off.');
    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());
  });
});
