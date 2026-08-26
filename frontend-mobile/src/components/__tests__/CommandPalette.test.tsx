// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile slash-command palette over the composer
// ABOUTME: Asserts "/" surfaces a real server command name and selecting it fills the composer

import React, { useState } from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CommandEntry } from '@pierre/shared-types';
import { ChatInputBar } from '../../screens/chat/ChatInputBar';
import { chatApi } from '../../services/api';

jest.mock('expo-router', () => ({
  useLocalSearchParams: () => ({ conversationId: 'conv-1' }),
}));
jest.mock('../../services/api', () => ({
  chatApi: { listCommands: jest.fn() },
}));

const listCommands = chatApi.listCommands as jest.Mock;

/** The catalogue rows the server returns, in its domain-then-command order. */
const CATALOGUE: CommandEntry[] = [
  {
    name: 'group-invite',
    command: '/group invite',
    args: '<email>',
    description: 'Invite an athlete to this group',
    domain: 'group',
  },
  {
    name: 'group-status',
    command: '/group status',
    args: null,
    description: 'Show this group and your role in it',
    domain: 'group',
  },
  {
    name: 'plan',
    command: '/plan',
    args: '[week|today]',
    description: 'Show your training plan',
    domain: 'training',
  },
];

const onSendMessage = jest.fn();

/** The composer with real value state, so a fill is observable in the input. */
function Composer() {
  const [inputText, setInputText] = useState('');
  const inputRef = React.useRef(null);
  return (
    <ChatInputBar
      inputText={inputText}
      partialTranscript=""
      isListening={false}
      isSending={false}
      voiceAvailable={false}
      inputRef={inputRef}
      onChangeText={setInputText}
      onVoicePress={() => {}}
      onSendMessage={onSendMessage}
    />
  );
}

function renderComposer() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer />
    </QueryClientProvider>,
  );
}

describe('slash-command palette (mobile composer)', () => {
  beforeEach(() => {
    listCommands.mockReset();
    onSendMessage.mockReset();
    listCommands.mockResolvedValue(CATALOGUE);
  });

  // Turns red if "/" stops surfacing the server's catalogue — the exact
  // "23 commands, discoverable only on messaging" gap this closes.
  it('surfaces a real server command name when the athlete types "/"', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '/');

    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
    expect(screen.getByTestId('command-palette-option-plan')).toBeTruthy();
    expect(screen.getByText('/plan')).toBeTruthy();
    expect(screen.getByText('Show your training plan')).toBeTruthy();
    // The palette asked the server for THIS conversation, so group-scoped
    // commands are resolved against the group it is bound to.
    expect(listCommands).toHaveBeenCalledWith('conv-1');
  });

  // Turns red if the client ever hardcodes a command list: only what the
  // server returned may appear, so a shorter answer means a shorter palette.
  it('offers only the commands the server returned for this caller', async () => {
    listCommands.mockResolvedValue([CATALOGUE[2]]);
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '/');

    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
    expect(screen.getByTestId('command-palette-option-plan')).toBeTruthy();
    expect(screen.queryByTestId('command-palette-option-group-invite')).toBeNull();
  });

  // Turns red if the prefix filter breaks — typing narrows to the matching
  // commands rather than showing the whole catalogue forever.
  it('narrows the list as the command is typed', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '/group i');

    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-invite')).toBeTruthy(),
    );
    expect(screen.queryByTestId('command-palette-option-plan')).toBeNull();
    expect(screen.queryByTestId('command-palette-option-group-status')).toBeNull();
  });

  // Turns red if selecting stops filling the composer, or fills it without the
  // trailing space a command with arguments needs.
  it('fills the composer with the selected command', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '/gro');
    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-invite')).toBeTruthy(),
    );

    fireEvent.press(screen.getByTestId('command-palette-option-group-invite'));

    await waitFor(() =>
      expect(screen.getByTestId('message-input').props.value).toBe('/group invite '),
    );
  });

  // Turns red if the palette opens on prose. A "/" mid-sentence is a slash,
  // not a command, and a palette over ordinary typing is a regression.
  it('stays closed for text that is not a command draft', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), 'how many km/h was that');

    expect(screen.queryByTestId('command-palette')).toBeNull();
    expect(listCommands).not.toHaveBeenCalled();
  });
});
