// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile @handle mention palette over the composer, including its keyboard
// ABOUTME: Asserts "@" offers installed coaches, arrows move the highlight, Enter inserts and Escape dismisses

import React, { useState } from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Coach } from '@pierre/shared-types';
import { ChatInputBar } from '../../screens/chat/ChatInputBar';
import { coachesApi, chatApi } from '../../services/api';

jest.mock('expo-router', () => ({
  useLocalSearchParams: () => ({ conversationId: 'conv-1' }),
}));
jest.mock('../../services/api', () => ({
  chatApi: { listCommands: jest.fn() },
  coachesApi: { list: jest.fn() },
}));

const listCoaches = coachesApi.list as jest.Mock;
const listCommands = chatApi.listCommands as jest.Mock;

function coach(overrides: Partial<Coach>): Coach {
  return {
    id: 'coach-1',
    title: 'Coach',
    description: null,
    system_prompt: '',
    category: 'training',
    tags: [],
    token_count: 0,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-08-01T00:00:00Z',
    updated_at: '2026-08-01T00:00:00Z',
    is_system: false,
    is_assigned: true,
    ...overrides,
  } as Coach;
}

/**
 * The athlete's own list: two installed catalogue coaches, one personal coach with
 * no handle, and one listed-but-never-installed coach. Only an assignment row makes
 * `@handle` route, so the last one must never be offered.
 */
const INSTALLED: Coach[] = [
  coach({ id: 'coach-tempo', title: 'Coach Tempo', handle: 'coach-tempo' }),
  coach({ id: 'coach-recovery', title: 'Recovery Guru', handle: 'recovery-guru' }),
  coach({ id: 'coach-personal', title: 'My private coach' }),
  coach({
    id: 'coach-catalogue',
    title: 'Coach Catalogue',
    handle: 'coach-catalogue',
    is_system: true,
    is_assigned: false,
  }),
];

const onSendMessage = jest.fn();

/** The composer with real value state, so an insertion is observable in the input. */
function Composer({ initial = '' }: { initial?: string }) {
  const [inputText, setInputText] = useState(initial);
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

function renderComposer(initial?: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer initial={initial} />
    </QueryClientProvider>,
  );
}

describe('@handle mention palette (mobile composer)', () => {
  beforeEach(() => {
    listCoaches.mockReset();
    listCommands.mockReset();
    onSendMessage.mockReset();
    listCoaches.mockResolvedValue({ coaches: INSTALLED });
  });

  // Turns red if "@" stops offering the athlete's installed coaches, or offers
  // a coach that has no handle to route to.
  it('offers the installed coaches by handle when the athlete types "@"', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '@');

    await waitFor(() => expect(screen.getByTestId('mention-palette')).toBeTruthy());
    expect(screen.getByTestId('mention-palette-option-coach-tempo')).toBeTruthy();
    expect(screen.getByTestId('mention-palette-option-recovery-guru')).toBeTruthy();
    expect(screen.getByText('@coach-tempo')).toBeTruthy();
    expect(screen.getByText('Coach Tempo')).toBeTruthy();
    expect(screen.queryByText('My private coach')).toBeNull();
    expect(listCoaches).toHaveBeenCalledTimes(1);
  });

  // Turns red if a phone keyboard's auto-capitalised letter stops finding the
  // coach, or if the inserted text carries that capital: the handle goes in
  // lowercase, exactly as the catalogue spells it, followed by a space.
  it('matches an auto-capitalised draft and inserts the lowercase handle verbatim', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '@Rec');

    await waitFor(() =>
      expect(screen.getByTestId('mention-palette-option-recovery-guru')).toBeTruthy(),
    );
    expect(screen.queryByTestId('mention-palette-option-coach-tempo')).toBeNull();

    fireEvent.press(screen.getByTestId('mention-palette-option-recovery-guru'));

    await waitFor(() =>
      expect(screen.getByTestId('message-input').props.value).toBe('@recovery-guru '),
    );
    expect(screen.queryByTestId('mention-palette')).toBeNull();
  });

  it('inserts the handle over the draft and leaves the text before it untouched', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), 'Salut @co');
    await waitFor(() => expect(screen.getByTestId('mention-palette-option-coach-tempo')).toBeTruthy());

    fireEvent.press(screen.getByTestId('mention-palette-option-coach-tempo'));

    await waitFor(() =>
      expect(screen.getByTestId('message-input').props.value).toBe('Salut @coach-tempo '),
    );
  });

  // Turns red if the palette opens on prose. An "@" inside an address is not
  // a mention, and a palette over ordinary typing is a regression.
  it('stays closed for an "@" that is part of a word', async () => {
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), 'write to jf@dravr.ai');

    expect(screen.queryByTestId('mention-palette')).toBeNull();
    expect(listCoaches).not.toHaveBeenCalled();
  });

  it('offers nothing to an athlete with no installed coach', async () => {
    listCoaches.mockResolvedValue({ coaches: [] });
    renderComposer();

    fireEvent.changeText(screen.getByTestId('message-input'), '@');

    await waitFor(() => expect(listCoaches).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('mention-palette')).toBeNull();
  });

  it('walks the handles with the arrows and inserts the highlighted one on Enter', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '@');
    await waitFor(() =>
      expect(screen.getByTestId('mention-palette-option-coach-tempo')).toBeTruthy(),
    );
    expect(screen.getByTestId('mention-palette-option-coach-tempo').props.accessibilityState)
      .toEqual({ selected: true });

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'ArrowDown' } });
    await waitFor(() =>
      expect(screen.getByTestId('mention-palette-option-recovery-guru').props.accessibilityState)
        .toEqual({ selected: true }),
    );

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'Enter' } });
    await waitFor(() =>
      expect(screen.getByTestId('message-input').props.value).toBe('@recovery-guru '),
    );
  });

  it('Escape dismisses the mention palette until the next edit', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '@co');
    await waitFor(() => expect(screen.getByTestId('mention-palette')).toBeTruthy());

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'Escape' } });
    await waitFor(() => expect(screen.queryByTestId('mention-palette')).toBeNull());

    fireEvent.changeText(input, '@coa');
    await waitFor(() => expect(screen.getByTestId('mention-palette')).toBeTruthy());
  });

  // A handle typed in full belongs to the composer: the athlete addressed the
  // coach and Enter must not re-insert what is already there.
  it('leaves Enter to the composer once the handle is complete', async () => {
    renderComposer();
    const input = screen.getByTestId('message-input');

    fireEvent.changeText(input, '@coach-tempo');
    await waitFor(() =>
      expect(screen.getByTestId('mention-palette-option-coach-tempo')).toBeTruthy(),
    );

    fireEvent(input, 'keyPress', { nativeEvent: { key: 'Enter' } });
    expect(screen.getByTestId('message-input').props.value).toBe('@coach-tempo');
  });
});
