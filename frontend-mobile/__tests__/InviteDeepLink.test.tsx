// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the group invite deep link — /groups/join/<code> redirects into a thread that runs /group join
// ABOUTME: There is no Groups tab to land on, so the link has to redeem the code the way an athlete would type it

import React from 'react';
import { render } from '@testing-library/react-native';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';

const mockRedirect = jest.fn();
let mockParams: { code?: string } = {};

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

import JoinGroupByInviteLink from '../app/groups/join/[code]';
import { CHAT_LIST_ROUTE, CHAT_THREAD_ROUTE } from '../src/navigation/routes';

describe('the group invite deep link', () => {
  beforeEach(() => {
    mockRedirect.mockClear();
    mockParams = {};
  });

  it('opens a fresh thread that sends /group join with the code', () => {
    mockParams = { code: 'HARRI-7X2' };

    render(<JoinGroupByInviteLink />);

    expect(mockRedirect).toHaveBeenCalledWith({
      pathname: CHAT_THREAD_ROUTE,
      params: { conversationId: 'new', send: COMMAND_DRAFTS.groupJoin('HARRI-7X2') },
    });
  });

  it('trims a code that arrived with whitespace around it', () => {
    mockParams = { code: '  HARRI-7X2  ' };

    render(<JoinGroupByInviteLink />);

    expect(mockRedirect).toHaveBeenCalledWith({
      pathname: CHAT_THREAD_ROUTE,
      params: { conversationId: 'new', send: COMMAND_DRAFTS.groupJoin('HARRI-7X2') },
    });
  });

  // A link with nothing to redeem must not open a thread that sends a broken
  // command; the conversation list is where it lands instead.
  it('lands on the conversation list when the link carries no code', () => {
    mockParams = { code: '   ' };

    render(<JoinGroupByInviteLink />);

    expect(mockRedirect).toHaveBeenCalledWith(CHAT_LIST_ROUTE);
  });
});
