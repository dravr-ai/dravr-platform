// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Group info's claim-coach block — the one button, the refreshed group queries, the confirmation and the failure line
// ABOUTME: The server's PUT /api/groups/:id/coach is mocked; the block draws no identity of its own

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Toast from 'react-native-toast-message';

const mockClaimCoach = jest.fn();
jest.mock('../../../services/api', () => ({
  groupsApi: { claimCoach: (groupId: string) => mockClaimCoach(groupId) },
}));

import { ClaimCoachBlock } from '../ClaimCoachBlock';

function renderBlock() {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  const invalidate = jest.spyOn(client, 'invalidateQueries');
  const screen = render(
    <QueryClientProvider client={client}>
      <ClaimCoachBlock groupId="g-1" />
    </QueryClientProvider>,
  );
  return { screen, invalidate };
}

describe('ClaimCoachBlock', () => {
  beforeEach(() => {
    mockClaimCoach.mockReset();
    (Toast.show as jest.Mock).mockClear();
  });

  it('says the group has no coach and offers the one button', () => {
    const { screen } = renderBlock();
    expect(screen.getByText('This group has no coach yet')).toBeTruthy();
    expect(screen.getByText('Become this group\'s coach')).toBeTruthy();
    expect(mockClaimCoach).not.toHaveBeenCalled();
  });

  it('claims the seat, refreshes the group queries and confirms', async () => {
    mockClaimCoach.mockResolvedValue({ group: { id: 'g-1' } });
    const { screen, invalidate } = renderBlock();

    fireEvent.press(screen.getByTestId('group-info-claim-coach-button'));

    await waitFor(() => expect(mockClaimCoach).toHaveBeenCalledWith('g-1'));
    await waitFor(() => expect(invalidate).toHaveBeenCalledWith({ queryKey: ['groups'] }));
    expect(Toast.show).toHaveBeenCalledWith(expect.objectContaining({ text1: 'You\'re now this group\'s coach.' }));
  });

  it('says it failed and keeps the button when the server refuses', async () => {
    mockClaimCoach.mockRejectedValue(new Error('403'));
    const { screen } = renderBlock();

    fireEvent.press(screen.getByTestId('group-info-claim-coach-button'));

    await waitFor(() =>
      expect(Toast.show).toHaveBeenCalledWith(expect.objectContaining({ text2: 'Couldn\'t make you the coach. Try again.' })),
    );
    expect(screen.getByTestId('group-info-claim-coach-button')).toBeTruthy();
  });
});
