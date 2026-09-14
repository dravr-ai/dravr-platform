// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the composer bar's shape — an in-flow bar with a top hairline, a 20-radius field, an empty leading slot
// ABOUTME: Behaviour: the send testID flips with the draft, send fires the callback, the mic is absent without recognition

import React, { useState } from 'react';
import { TouchableOpacity } from 'react-native';
import { fireEvent, render, screen } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ChatInputBar, composerBottomPadding } from '../src/screens/chat/ChatInputBar';

jest.mock('expo-router', () => ({
  useLocalSearchParams: () => ({ conversationId: 'conv-1' }),
}));
jest.mock('../src/services/api', () => ({
  chatApi: { listCommands: jest.fn(() => Promise.resolve([])) },
  coachesApi: { list: jest.fn(() => Promise.resolve({ agents: [] })) },
}));

const onSendMessage = jest.fn();
const onVoicePress = jest.fn();

/** The composer with real value state, so the send flip follows the draft. */
function Composer({ initial = '', voiceAvailable = false }: { initial?: string; voiceAvailable?: boolean }) {
  const [inputText, setInputText] = useState(initial);
  const inputRef = React.useRef(null);
  return (
    <ChatInputBar
      inputText={inputText}
      partialTranscript=""
      isListening={false}
      isSending={false}
      voiceAvailable={voiceAvailable}
      inputRef={inputRef}
      onChangeText={setInputText}
      onVoicePress={onVoicePress}
      onSendMessage={onSendMessage}
    />
  );
}

function renderComposer(props: { initial?: string; voiceAvailable?: boolean } = {}) {
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

describe('composerBottomPadding', () => {
  it('pays the home-indicator inset only while the keyboard is down', () => {
    expect(composerBottomPadding(false, 34)).toBe(42);
    expect(composerBottomPadding(true, 34)).toBe(8);
    expect(composerBottomPadding(false, 0)).toBe(8);
  });
});
