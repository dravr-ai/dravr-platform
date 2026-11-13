// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach seat's room strip on the phone (carnet#1520) — to ask, pending with its date, declined with Ask again
// ABOUTME: Pins the shared testIDs, the full-width filled request, the way to Group info and the failed-request line

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachAccessRequest as Request } from '@pierre/shared-types';

const mockGetRequest = jest.fn();
const mockRequest = jest.fn();
jest.mock('../../services/api', () => ({
  oauthApi: { getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }) },
  userApi: {
    getCoachAccessRequest: () => mockGetRequest(),
    requestCoachAccess: (groupId: string) => mockRequest(groupId),
  },
}));

import { CoachAccessRequest } from '../CoachAccessRequest';

const pending: Request = {
  id: 'req-1',
  user_id: 'user-owner',
  group_id: 'group-1',
  group_tenant_id: 'tenant-1',
  status: 'pending',
  created_at: '2026-11-12T19:32:00Z',
  decided_at: null,
  decided_by: null,
};

function renderStrip(onOpenInfo = jest.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <CoachAccessRequest groupId="group-1" variant="strip" onOpenInfo={onOpenInfo} />
    </QueryClientProvider>,
  );
  return { ...view, onOpenInfo };
}

describe('CoachAccessRequest strip', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetRequest.mockResolvedValue({ request: null });
  });

  it('says the coach is not the coach yet and offers the one filled request, naming this group', async () => {
    mockRequest.mockResolvedValue({ request: pending });
    const { findByTestId, getByTestId, queryByTestId } = renderStrip();

    expect(await findByTestId('coach-seat-strip-to_request')).toHaveTextContent("You're not this group's coach yet");
    // Nothing is pending before a request exists.
    expect(getByTestId('coach-seat-strip')).not.toHaveTextContent(/pending/i);
    fireEvent.press(getByTestId('coach-access-request'));

    await waitFor(() => expect(mockRequest).toHaveBeenCalledWith('group-1'));
    expect(await findByTestId('coach-seat-strip-pending')).toHaveTextContent(/^Coach access requested on .*2026$/);
    expect(getByTestId('coach-access-pending')).toHaveTextContent(/usually within 24 hours/);
    expect(queryByTestId('coach-access-request')).toBeNull();
  });

  it('opens Group info from its second action', async () => {
    const { findByTestId, onOpenInfo } = renderStrip();

    fireEvent.press(await findByTestId('coach-seat-open-info'));

    expect(onOpenInfo).toHaveBeenCalled();
  });

  it('dates a declined request and offers to ask again', async () => {
    mockGetRequest.mockResolvedValue({
      request: { ...pending, status: 'declined', decided_at: '2026-11-13T15:00:00Z', decided_by: 'admin-1' },
    });
    const { findByTestId, getByTestId } = renderStrip();

    expect(await findByTestId('coach-seat-strip-declined')).toHaveTextContent('Your coach access request was declined');
    expect(getByTestId('coach-access-declined')).toHaveTextContent(/You can ask again\./);
    expect(getByTestId('coach-access-request')).toHaveTextContent('Ask again');
  });

  it('says the request failed when the server refuses it, and keeps the button', async () => {
    mockRequest.mockRejectedValue(new Error('boom'));
    const { findByTestId, getByTestId } = renderStrip();

    fireEvent.press(await findByTestId('coach-access-request'));

    expect(await findByTestId('coach-access-failed')).toHaveTextContent("We couldn't send your request. Please try again.");
    expect(getByTestId('coach-access-request')).toBeTruthy();
  });
});
