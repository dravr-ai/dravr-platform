// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the composer bar's shape — an in-flow bar with a top hairline, a 20-radius field, an empty leading slot
// ABOUTME: Behaviour: the send testID flips with the draft, send fires the callback, the mic is absent without recognition

import React, { useState } from 'react';
import { TouchableOpacity } from 'react-native';
import { fireEvent, render, screen } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { privateLaneComposerText } from '@pierre/chat-utils';
import type { MessageAudience } from '@pierre/shared-types';
import {
  ChatInputBar,
  composerBottomPadding,
  type ComposerAudience,
  type ComposerQuote,
} from '../src/screens/chat/ChatInputBar';

jest.mock('expo-router', () => ({
  useLocalSearchParams: () => ({ conversationId: 'conv-1' }),
}));
jest.mock('../src/services/api', () => ({
  chatApi: { listCommands: jest.fn(() => Promise.resolve([])) },
  coachesApi: { list: jest.fn(() => Promise.resolve({ agents: [] })) },
}));

const onSendMessage = jest.fn();
const onVoicePress = jest.fn();
const onStopTurn = jest.fn();

interface ComposerProps {
  initial?: string;
  voiceAvailable?: boolean;
  isSending?: boolean;
  isStopping?: boolean;
  audience?: ComposerAudience;
  quote?: ComposerQuote | null;
}

/** The composer with real value state, so the send flip follows the draft. */
function Composer({
  initial = '',
  voiceAvailable = false,
  isSending = false,
  isStopping = false,
  audience,
  quote,
}: ComposerProps) {
  const [inputText, setInputText] = useState(initial);
  const inputRef = React.useRef(null);
  return (
    <ChatInputBar
      inputText={inputText}
      partialTranscript=""
      isListening={false}
      isSending={isSending}
      isStopping={isStopping}
      voiceAvailable={voiceAvailable}
      inputRef={inputRef}
      onChangeText={setInputText}
      onVoicePress={onVoicePress}
      onSendMessage={onSendMessage}
      onStopTurn={onStopTurn}
      audience={audience}
      quote={quote}
    />
  );
}

const onToggleAudience = jest.fn();

/** The switch of a coach's one-athlete room, worded as the catalogue words it. */
function audienceFor(value: MessageAudience): ComposerAudience {
  return {
    value,
    text: privateLaneComposerText(value, { agent: 'Dravr', athlete: 'Camille' }, i18n.t),
    onToggle: onToggleAudience,
  };
}

/**
 * WCAG 2.5.3 Label in Name: the switch's spoken name starts with the words it
 * shows, then says what a press does. The name carries the state, so the
 * switch announces no selected state on top of it.
 */
function expectNamedByItsLabel(toggle: ReturnType<typeof screen.getByTestId>, label: string) {
  expect(String(toggle.props.accessibilityLabel).startsWith(`${label}.`)).toBe(true);
  expect(toggle.props.accessibilityState?.selected).toBeUndefined();
}

function renderComposer(props: ComposerProps = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Composer {...props} />
    </QueryClientProvider>,
  );
}

describe('ChatInputBar', () => {
  beforeEach(() => {
    onSendMessage.mockReset();
    onVoicePress.mockReset();
    onStopTurn.mockReset();
  });

  // carnet#705: while a turn runs the send slot is a stop button. It used to
  // hold a spinner on a disabled send, with no way to end the reply.
  it('replaces send with a stop button while a turn runs, and stops on press', () => {
    renderComposer({ initial: 'still typing', isSending: true });

    expect(screen.queryByTestId('send-button')).toBeNull();
    expect(screen.queryByTestId('send-button-disabled')).toBeNull();
    const stop = screen.getByTestId('stop-turn-button');
    expect(stop.props.accessibilityLabel).toBe('Stop this reply');
    expect(stop.props.accessibilityState).toEqual({ disabled: false });

    fireEvent.press(stop);
    expect(onStopTurn).toHaveBeenCalledTimes(1);
    expect(onSendMessage).not.toHaveBeenCalled();
  });

  it('holds the stop button disabled once the stop was sent', () => {
    renderComposer({ isSending: true, isStopping: true });

    const stop = screen.getByTestId('stop-turn-button');
    expect(stop.props.accessibilityState).toEqual({ disabled: true });
    fireEvent.press(stop);
    expect(onStopTurn).not.toHaveBeenCalled();
  });

  it('shows no stop button while no turn runs', () => {
    renderComposer({ initial: 'hello' });
    expect(screen.getByTestId('send-button')).toBeTruthy();
    expect(screen.queryByTestId('stop-turn-button')).toBeNull();
  });

  // Boreal v2.2 P3.3: the leading slot is empty. Turns red if a button comes
  // back beside the field.
  it('has no slash-command button in the tree', () => {
    renderComposer();
    expect(screen.queryByTestId('slash-command-button')).toBeNull();
  });

  // P3.4: the field is 40 tall at rest with a 20 radius, on the container tint.
  it('lays the field out as a 20-radius container-tinted well', () => {
    renderComposer();
    const field = screen.getByTestId('message-field');
    expect(field.props.className).toContain('rounded-[20px]');
    expect(field.props.className).toContain('min-h-[40px]');
    expect(field.props.className).toContain('bg-surface-container');
    expect(field.props.className).not.toContain('border');
  });

  // The bar is paper with a top hairline, in the layout — not a floating pill.
  it('is a hairline-topped bar in the layout, not a floating pill', () => {
    renderComposer();
    const bar = screen.getByTestId('chat-input-bar');
    expect(bar.props.className).toContain('border-t');
    expect(bar.props.className).toContain('bg-background-primary');
    expect(bar.props.className).not.toContain('rounded-full');
    const styles = ([] as Array<Record<string, unknown>>).concat(bar.props.style);
    expect(styles.some((s) => s?.position === 'absolute')).toBe(false);
    expect(styles.some((s) => typeof s?.paddingBottom === 'number')).toBe(true);
  });

  // Four Maestro flows wait on this flip; it is the contract, not the styling.
  it('flips the send testID from disabled to enabled with the draft', () => {
    renderComposer();
    expect(screen.getByTestId('send-button-disabled')).toBeTruthy();
    expect(screen.queryByTestId('send-button')).toBeNull();

    fireEvent.changeText(screen.getByTestId('message-input'), 'Bonjour');

    expect(screen.getByTestId('send-button')).toBeTruthy();
    expect(screen.queryByTestId('send-button-disabled')).toBeNull();
  });

  // The fill is the primary token, applied only once there is something to send.
  it('paints the send circle with the primary fill only when it can send', () => {
    renderComposer();
    const idle = screen.UNSAFE_getByType(TouchableOpacity);
    expect(idle.props.className).toContain('w-8 h-8 rounded-full');
    expect(idle.props.style).toBeUndefined();

    fireEvent.changeText(screen.getByTestId('message-input'), 'Bonjour');

    const armed = screen.UNSAFE_getByType(TouchableOpacity);
    expect(typeof armed.props.style?.backgroundColor).toBe('string');
  });

  it('sends on a press of the armed send button', () => {
    renderComposer({ initial: 'Bonjour' });
    fireEvent.press(screen.getByTestId('send-button'));
    expect(onSendMessage).toHaveBeenCalledTimes(1);
  });

  it('shows the mic only when voice recognition is available', () => {
    renderComposer({ voiceAvailable: false });
    expect(screen.queryByTestId('voice-input-button')).toBeNull();

    renderComposer({ voiceAvailable: true });
    expect(screen.getByTestId('voice-input-button')).toBeTruthy();
  });
});

// carnet#1249: a coach in a room of one athlete chooses who the next line reaches.
describe('ChatInputBar audience switch', () => {
  beforeEach(() => {
    onToggleAudience.mockReset();
  });

  it('has no switch in a thread without a private lane', () => {
    renderComposer();
    expect(screen.queryByTestId('composer-audience-row')).toBeNull();
    expect(screen.queryByTestId('composer-audience-switch')).toBeNull();
    expect(screen.getByTestId('message-input').props.placeholder).toBe('Message Dravr...');
  });

  it('reads Everyone, with the room placeholder and a plain field, while the next line reaches the room', () => {
    renderComposer({ audience: audienceFor('room') });

    const toggle = screen.getByTestId('composer-audience-switch');
    expect(toggle).toHaveTextContent(/Everyone/);
    expect(toggle.props.accessibilityRole).toBe('button');
    expectNamedByItsLabel(toggle, 'Everyone');
    expect(toggle.props.accessibilityLabel).toBe('Everyone. Switch to only Dravr');
    // A 44-point target, whatever the chip inside it measures.
    expect(toggle.props.className).toContain('min-h-11');
    expect(screen.getByTestId('message-input').props.placeholder).toBe(
      'Message Camille and Dravr. Type / for commands',
    );
    const field = screen.getByTestId('message-field');
    expect(field.props.className).toContain('bg-surface-container');
    expect(field.props.className).not.toContain('border-dashed');
  });

  it('reads Only Dravr and dashes the field in the private accent while the next line is private', () => {
    renderComposer({ audience: audienceFor('private') });

    const toggle = screen.getByTestId('composer-audience-switch');
    expect(toggle).toHaveTextContent(/Only Dravr/);
    expectNamedByItsLabel(toggle, 'Only Dravr');
    expect(toggle.props.accessibilityLabel).toBe('Only Dravr. Switch to everyone');
    expect(screen.getByTestId('composer-audience-chip').props.className).toContain('bg-warning/15');
    expect(screen.getByTestId('message-input').props.placeholder).toBe(
      "Ask Dravr to change Camille's plan. Camille won't see this.",
    );
    const field = screen.getByTestId('message-field');
    expect(field.props.className).toContain('border-dashed');
    expect(field.props.className).toContain('border-warning/60');
    expect(field.props.className).not.toContain('bg-surface-container');
  });

  it('flips on a press', () => {
    renderComposer({ audience: audienceFor('room') });
    fireEvent.press(screen.getByTestId('composer-audience-switch'));
    expect(onToggleAudience).toHaveBeenCalledTimes(1);
  });

  it('shows the quote as a chip with a 44-point remove control', () => {
    const onRemove = jest.fn();
    renderComposer({
      audience: audienceFor('private'),
      quote: { label: "From Camille's message", removeLabel: 'Remove quote', onRemove },
    });

    expect(screen.getByTestId('composer-quote-chip')).toHaveTextContent(/From Camille's message/);
    const remove = screen.getByTestId('composer-quote-remove');
    expect(remove.props.accessibilityRole).toBe('button');
    expect(remove.props.accessibilityLabel).toBe('Remove quote');
    expect(remove.props.className).toContain('w-11 h-11');
    fireEvent.press(remove);
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});

describe('composerBottomPadding', () => {
  it('pays the home-indicator inset only while the keyboard is down', () => {
    expect(composerBottomPadding(false, 34)).toBe(42);
    expect(composerBottomPadding(true, 34)).toBe(8);
    expect(composerBottomPadding(false, 0)).toBe(8);
  });
});
