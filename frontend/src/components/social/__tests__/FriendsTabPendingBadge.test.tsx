// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that FriendsTab populates its pending-request counts on mount
// ABOUTME: The toolbar pill and tab bubble must show before the Pending tab is opened

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import '@testing-library/jest-dom';
import FriendsTab from '../FriendsTab';
import { socialApi } from '../../../services/api';

vi.mock('../../../services/api', () => ({
  socialApi: {
    listFriends: vi.fn(),
    searchUsers: vi.fn(),
    getPendingFriendRequests: vi.fn(),
    sendFriendRequest: vi.fn(),
    acceptFriendRequest: vi.fn(),
    rejectFriendRequest: vi.fn(),
    removeFriend: vi.fn(),
  },
}));

const metadata = { timestamp: '2024-01-01T00:00:00Z', api_version: 'v1' };

const mockFriends = {
  friends: [
    {
      id: 'conn-1',
      initiator_id: 'user-1',
      receiver_id: 'user-2',
      status: 'accepted',
      created_at: '2024-01-01T00:00:00Z',
      updated_at: '2024-01-01T00:00:00Z',
      accepted_at: '2024-01-02T00:00:00Z',
      friend_display_name: 'Jane Doe',
      friend_email: 'jane@example.com',
      friend_user_id: 'user-2',
    },
  ],
  total: 1,
  metadata,
};

// Two received requests so the rendered count is "2" and cannot be confused
// with an unrelated "1" elsewhere in the tab chrome.
const mockPendingRequests = {
  sent: [],
  received: [
    {
      id: 'conn-2',
      initiator_id: 'user-4',
      receiver_id: 'user-1',
      status: 'pending',
      created_at: '2024-01-01T00:00:00Z',
      updated_at: '2024-01-01T00:00:00Z',
      accepted_at: null,
      user_display_name: 'Alice Johnson',
      user_email: 'alice@example.com',
      user_id: 'user-4',
    },
    {
      id: 'conn-3',
      initiator_id: 'user-5',
      receiver_id: 'user-1',
      status: 'pending',
      created_at: '2024-01-01T00:00:00Z',
      updated_at: '2024-01-01T00:00:00Z',
      accepted_at: null,
      user_display_name: 'Bob Startup',
      user_email: 'bob@startup.io',
      user_id: 'user-5',
    },
  ],
  metadata,
};

describe('FriendsTab pending badges', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(socialApi.listFriends).mockResolvedValue(mockFriends);
    vi.mocked(socialApi.getPendingFriendRequests).mockResolvedValue(mockPendingRequests);
  });

  it('shows the toolbar pill on mount without opening the Pending tab', async () => {
    render(<FriendsTab />);

    await waitFor(() => {
      expect(screen.getByText('2 pending')).toBeInTheDocument();
    });
  });

  it('shows the count bubble on the Pending tab button on mount', async () => {
    render(<FriendsTab />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Pending/i })).toHaveTextContent('2');
    });
  });

  it('fetches pending requests on mount while the Friends tab is active', async () => {
    render(<FriendsTab />);

    await waitFor(() => {
      expect(socialApi.getPendingFriendRequests).toHaveBeenCalledTimes(1);
    });
  });

  it('keeps the friend list rendered while the badge count loads', async () => {
    render(<FriendsTab />);

    await waitFor(() => {
      expect(screen.getByText('2 pending')).toBeInTheDocument();
    });

    expect(screen.getByText('Jane Doe')).toBeInTheDocument();
  });
});
