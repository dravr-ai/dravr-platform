// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's join strip over a mocked held invite — the one button, the joined room, a full group, a dropped hold
// ABOUTME: The phone never joins on its own: nothing is sent until the athlete presses the button

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Toast from 'react-native-toast-message';
import type { HeldGroupInvite } from '@pierre/shared-types';

const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../../../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
    useFocusEffect: (callback: () => void) => {
      const React = require('react');
      React.useEffect(() => callback(), [callback]);
    },
  }),
);

const mockHeldInvite = jest.fn();
const mockJoinHeldInvite = jest.fn();
jest.mock('../../../services/api', () => ({
  groupsApi: {
    heldInvite: () => mockHeldInvite(),
    joinHeldInvite: () => mockJoinHeldInvite(),
  },
}));

import { HomeJoinStrip } from '../HomeJoinStrip';
import { threadHref } from '../../../navigation/routes';

const HELD: HeldGroupInvite = {
  kind: 'member',
  group_name: 'Marathon de Montréal',
  inviter_name: 'Phil T.',
  inviter_initials: 'PT',
  has_coach: true,
  coach_name: 'Phil T.',
  held_at: '2026-11-15T10:00:00Z',
  state: 'pending',
};

function renderStrip() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = jest.spyOn(client, 'invalidateQueries');
  const screen = render(
    <QueryClientProvider client={client}>
      <HomeJoinStrip />
    </QueryClientProvider>,
  );
  return { screen, invalidate };
}

describe('Home join strip', () => {
  beforeEach(() => {
    mockPush.mockClear();
    mockHeldInvite.mockReset();
    mockJoinHeldInvite.mockReset();
    (Toast.show as jest.Mock).mockClear();
  });

  it('draws nothing when no invite is held', async () => {
    mockHeldInvite.mockResolvedValue({ held: null });
    const { screen } = renderStrip();
    await waitFor(() => expect(mockHeldInvite).toHaveBeenCalled());
    expect(screen.queryByTestId('home-join-strip')).toBeNull();
  });

  it('shows who invites the athlete, what joining shares and one button, and joins only on press', async () => {
    mockHeldInvite.mockResolvedValue({ held: HELD });
    const { screen } = renderStrip();

    await screen.findByTestId('home-join-strip');
    expect(screen.getByText('Phil T. invites you to “Marathon de Montréal”.')).toBeTruthy();
    expect(screen.getByText(/Phil T\. will see your activities/)).toBeTruthy();
    expect(screen.getByText('Join Phil T.\'s group')).toBeTruthy();
    expect(mockJoinHeldInvite).not.toHaveBeenCalled();
  });

  it('opens the group room and refreshes the lists once joined', async () => {
    mockHeldInvite.mockResolvedValue({ held: HELD });
    mockJoinHeldInvite.mockResolvedValue({
      outcome: 'joined',
      kind: 'member',
      group_id: 'g-1',
      group_name: HELD.group_name,
      conversation_id: 'conv-9',
    });
    const { screen, invalidate } = renderStrip();

    fireEvent.press(await screen.findByTestId('home-join-strip-join'));

    await waitFor(() => expect(mockPush).toHaveBeenCalledWith(threadHref('conv-9')));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-conversations'] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['group-invite', 'held'] });
  });

  it('just refreshes when the athlete was already a member', async () => {
    mockHeldInvite.mockResolvedValue({ held: HELD });
    mockJoinHeldInvite.mockResolvedValue({ outcome: 'already_member', kind: 'member', group_id: 'g-1', group_name: 'G' });
    const { screen, invalidate } = renderStrip();

    fireEvent.press(await screen.findByTestId('home-join-strip-join'));

    await waitFor(() => expect(invalidate).toHaveBeenCalledWith({ queryKey: ['group-invite', 'held'] }));
    expect(mockPush).not.toHaveBeenCalled();
  });

  it('says the group is full, keeps the strip, and offers a retry', async () => {
    mockHeldInvite.mockResolvedValue({ held: HELD });
    mockJoinHeldInvite.mockResolvedValue({ outcome: 'refused', reason: 'group_full', message: 'Full' });
    const { screen } = renderStrip();

    fireEvent.press(await screen.findByTestId('home-join-strip-join'));

    await screen.findByTestId('home-join-strip-full');
    expect(screen.getByText('Try again')).toBeTruthy();
    expect(screen.getByTestId('home-join-strip')).toBeTruthy();
  });

  it('shows the full state straight away for a hold the server marked group_full', async () => {
    mockHeldInvite.mockResolvedValue({ held: { ...HELD, state: 'group_full' } });
    const { screen } = renderStrip();

    await screen.findByTestId('home-join-strip-full');
    expect(screen.getByText('Try again')).toBeTruthy();
  });

  it('toasts the server message and hides the strip when the hold is invalid', async () => {
    mockHeldInvite.mockResolvedValueOnce({ held: HELD }).mockResolvedValue({ held: null });
    mockJoinHeldInvite.mockResolvedValue({ outcome: 'refused', reason: 'invalid', message: 'This invite expired.' });
    const { screen } = renderStrip();

    fireEvent.press(await screen.findByTestId('home-join-strip-join'));

    await waitFor(() => expect(Toast.show).toHaveBeenCalledWith(expect.objectContaining({ text2: 'This invite expired.' })));
    await waitFor(() => expect(screen.queryByTestId('home-join-strip')).toBeNull());
  });

  it('says it failed on a network error and offers a retry', async () => {
    mockHeldInvite.mockResolvedValue({ held: HELD });
    mockJoinHeldInvite.mockRejectedValue(new Error('Network Error'));
    const { screen } = renderStrip();

    fireEvent.press(await screen.findByTestId('home-join-strip-join'));

    await screen.findByTestId('home-join-strip-failed');
    expect(screen.getByText('Try again')).toBeTruthy();
  });

  it('titles a coach seat as an invitation to coach and does not claim sharing', async () => {
    mockHeldInvite.mockResolvedValue({ held: { ...HELD, kind: 'coach' } });
    const { screen } = renderStrip();

    await screen.findByText('Phil T. invites you to coach “Marathon de Montréal”.');
    expect(screen.queryByText(/will see your activities/)).toBeNull();
  });
});
