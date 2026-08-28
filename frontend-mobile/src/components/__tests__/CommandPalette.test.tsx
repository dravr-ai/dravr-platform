// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile slash-command palette over the composer, including its keyboard
// ABOUTME: Asserts the "/" button and key open it, arrows move the highlight, Enter takes it and Escape dismisses

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
      restingOffset={68}
      keyboardHeight={0}
      keyboardDuration={250}
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

  // The visible affordance: new athletes do not know "/" exists until
  // something tells them, which is why Telegram gives its bots a menu button.
  it('the "/" button opens the palette', async () => {
    renderComposer();

    fireEvent.press(screen.getByTestId('slash-command-button'));

    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
    expect(screen.getByTestId('message-input').props.value).toBe('/');
  });

  it('walks the list with the arrows and takes the highlighted row on Enter', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '/group');
    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-invite')).toBeTruthy(),
    );
    // The first row is highlighted before a key is pressed.
    expect(screen.getByTestId('command-palette-option-group-invite').props.accessibilityState)
      .toEqual({ selected: true });

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'ArrowDown' } });
    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-status').props.accessibilityState)
        .toEqual({ selected: true }),
    );

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'Enter' } });
    await waitFor(() => expect(screen.getByTestId('message-input').props.value).toBe('/group status'));
  });

  it('wraps the highlight upward past the first row', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '/group');
    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-invite')).toBeTruthy(),
    );

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'ArrowUp' } });
    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-status').props.accessibilityState)
        .toEqual({ selected: true }),
    );
  });

  it('Escape dismisses the palette until the next edit', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '/gro');
    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'Escape' } });
    await waitFor(() => expect(screen.queryByTestId('command-palette')).toBeNull());

    fireEvent.changeText(input, '/grou');
    await waitFor(() => expect(screen.getByTestId('command-palette')).toBeTruthy());
  });

  // A finished command belongs to the composer: the athlete typed the whole
  // thing and Enter must send it, not re-fill it.
  it('leaves Enter to the composer once the command is complete', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '/group status');
    await waitFor(() =>
      expect(screen.getByTestId('command-palette-option-group-status')).toBeTruthy(),
    );

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'Enter' } });
    expect(onSendMessage).toHaveBeenCalledTimes(1);
  });
});
