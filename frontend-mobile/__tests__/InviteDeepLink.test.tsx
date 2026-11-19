// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the group invite deep link — /groups/join/<code> asks the server to hold the invite, then lands on Home
// ABOUTME: The link sends no /group join: Home's join strip is the consent moment (carnet#1648, carnet#1650)

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Toast from 'react-native-toast-message';

const mockRedirect = jest.fn();
let mockParams: { code?: string } = {};
const mockHoldInvite = jest.fn();

jest.mock('expo-router', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    useLocalSearchParams: () => mockParams,
    Redirect: (props: { href: unknown }) => {
      mockRedirect(props.href);
      return React.createElement(View, { testID: 'redirect' });
    },
  };
});
jest.mock('../src/services/api', () => ({
  groupsApi: { holdInvite: (code: string) => mockHoldInvite(code) },
}));

import JoinGroupByInviteLink from '../app/groups/join/[code]';
import { CHAT_LIST_ROUTE, HOME_ROUTE } from '../src/navigation/routes';

function renderLink() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <JoinGroupByInviteLink />
    </QueryClientProvider>,
  );
}

describe('the group invite deep link', () => {
  beforeEach(() => {
    mockRedirect.mockClear();
    mockHoldInvite.mockReset();
    (Toast.show as jest.Mock).mockClear();
    mockParams = {};
  });

  it('holds the code on the server, then lands on Home', async () => {
    mockParams = { code: 'HARRI-7X2' };
    mockHoldInvite.mockResolvedValue({ held: true, kind: 'member' });

    renderLink();

    await waitFor(() => expect(mockRedirect).toHaveBeenCalledWith(HOME_ROUTE));
    expect(mockHoldInvite).toHaveBeenCalledWith('HARRI-7X2');
    expect(mockRedirect).not.toHaveBeenCalledWith(expect.objectContaining({ params: expect.anything() }));
  });

  it('trims a code that arrived with whitespace around it', async () => {
    mockParams = { code: '  HARRI-7X2  ' };
    mockHoldInvite.mockResolvedValue({ held: true, kind: 'member' });

    renderLink();

    await waitFor(() => expect(mockHoldInvite).toHaveBeenCalledWith('HARRI-7X2'));
  });

  it('toasts the failure and still lands on Home when the server refuses the code', async () => {
    mockParams = { code: 'BAD' };
    mockHoldInvite.mockRejectedValue(new Error('refused'));

    renderLink();

    await waitFor(() => expect(mockRedirect).toHaveBeenCalledWith(HOME_ROUTE));
    expect(Toast.show).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });

  // A link with nothing to hold must not call the server; the conversation
  // list is where it lands instead.
  it('lands on the conversation list when the link carries no code', () => {
    mockParams = { code: '   ' };

    renderLink();

    expect(mockHoldInvite).not.toHaveBeenCalled();
    expect(mockRedirect).toHaveBeenCalledWith(CHAT_LIST_ROUTE);
  });
});
