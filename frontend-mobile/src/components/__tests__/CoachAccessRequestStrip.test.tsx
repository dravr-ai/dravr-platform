// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach seat's room strip and group-less panel on the phone — to ask, pending with its date and cancel, declined with its reason
// ABOUTME: Pins the shared testIDs, the full-width filled request, the way to Group info, withdrawing, and the granted panel (carnet#1520, #1521, #1522)

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachAccessRequest as Request } from '@pierre/shared-types';

const mockGetRequest = jest.fn();
const mockRequest = jest.fn();
const mockCancel = jest.fn();
jest.mock('../../services/api', () => ({
  oauthApi: { getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }) },
  userApi: {
    getCoachAccessRequest: () => mockGetRequest(),
    requestCoachAccess: (groupId?: string) => mockRequest(groupId),
    cancelCoachAccessRequest: () => mockCancel(),
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

function renderPanel(props: { managesRoster?: boolean; onOpenHome?: () => void } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CoachAccessRequest variant="panel" {...props} />
    </QueryClientProvider>,
  );
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
    expect(await findByTestId('coach-seat-strip-pending')).toHaveTextContent(/^Coach access requested$/);
    // Dated, no wait time promised: the answer comes here and by email.
    expect(getByTestId('coach-access-pending')).toHaveTextContent(
      /^Request sent .*2026\. We'll let you know here and by email\.$/,
    );
    expect(getByTestId('coach-seat-strip')).not.toHaveTextContent(/24/);
    expect(queryByTestId('coach-access-request')).toBeNull();
    expect(getByTestId('coach-access-cancel')).toHaveTextContent('Cancel request');
  });

  it('withdraws the waiting request and offers it again at once', async () => {
    mockGetRequest.mockResolvedValue({ request: pending });
    mockCancel.mockResolvedValue({ request: null });
    const { findByTestId, getByTestId } = renderStrip();

    fireEvent.press(await findByTestId('coach-access-cancel'));

    await waitFor(() => expect(mockCancel).toHaveBeenCalled());
    expect(await findByTestId('coach-seat-strip-to_request')).toBeTruthy();
    expect(getByTestId('coach-access-cancelled')).toHaveTextContent(/Request cancelled/);
    expect(getByTestId('coach-access-request')).toBeTruthy();
  });

  it('says a withdrawal failed and keeps the request waiting', async () => {
    mockGetRequest.mockResolvedValue({ request: pending });
    mockCancel.mockRejectedValue(new Error('boom'));
    const { findByTestId } = renderStrip();

    fireEvent.press(await findByTestId('coach-access-cancel'));

    expect(await findByTestId('coach-access-cancel-failed')).toHaveTextContent("We couldn't cancel your request. Please try again.");
    expect(await findByTestId('coach-seat-strip-pending')).toBeTruthy();
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

  it("shows the team's reason with a decline, when it gave one", async () => {
    mockGetRequest.mockResolvedValue({
      request: {
        ...pending,
        status: 'declined',
        decided_at: '2026-11-13T15:00:00Z',
        decided_by: 'admin-1',
        decline_reason: 'No coaching account we could verify.',
      },
    });
    const { findByTestId, getByTestId } = renderStrip();

    expect(await findByTestId('coach-access-decline-reason')).toHaveTextContent(
      'Reason: No coaching account we could verify.',
    );
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

describe('CoachAccessRequest panel (no group, carnet#1522)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetRequest.mockResolvedValue({ request: null });
  });

  it('asks with no group, then dates the request and offers to withdraw it', async () => {
    mockRequest.mockResolvedValue({ request: { ...pending, group_id: null, group_tenant_id: null } });
    const { findByTestId, getByTestId } = renderPanel();

    expect(await findByTestId('coach-seat-panel-to_request')).toHaveTextContent('Get your coach access'); // humanCoach.seat.panelTitle
    fireEvent.press(getByTestId('coach-access-request'));

    await waitFor(() => expect(mockRequest).toHaveBeenCalledWith(undefined));
    expect(await findByTestId('coach-seat-panel-pending')).toHaveTextContent('Coach access requested');
    expect(getByTestId('coach-access-pending')).toHaveTextContent(/We'll let you know here and by email\./);
    expect(getByTestId('coach-access-cancel')).toBeTruthy();
  });

  it('says the access is on once granted, and opens the coach Home', async () => {
    const onOpenHome = jest.fn();
    const { findByTestId } = renderPanel({ managesRoster: true, onOpenHome });

    expect(await findByTestId('coach-seat-panel-granted')).toHaveTextContent('Your coach access is on');
    fireEvent.press(await findByTestId('coach-seat-open-home'));
    expect(onOpenHome).toHaveBeenCalled();
  });
});
