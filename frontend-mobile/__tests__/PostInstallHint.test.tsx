// ABOUTME: Unit tests for the post-install hint — the copy it teaches and the draft it hands to Open chat
// ABOUTME: Pins the /coach add @handle command and the @handle mention as the two ways to use a coach

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import { PostInstallHint } from '../src/screens/store/PostInstallHint';
import { coachAddDraft, coachMention } from '../src/screens/store/coachDraft';

describe('PostInstallHint', () => {
  it('teaches /coach add @handle and the @handle mention for the installed coach', () => {
    const { getByTestId } = render(
      <PostInstallHint
        coachTitle="Marathon Training Coach"
        handle="marathon-training-coach"
        onOpenChat={jest.fn()}
        onDismiss={jest.fn()}
      />,
    );

    expect(getByTestId('post-install-title')).toHaveTextContent('“Marathon Training Coach” is in your coaches');
    expect(getByTestId('post-install-body')).toHaveTextContent(
      'Use it in any chat: /coach add @marathon-training-coach — or mention @marathon-training-coach for one turn',
    );
  });

  it('hands the /coach add draft to Open chat', () => {
    const onOpenChat = jest.fn();
    const { getByTestId } = render(
      <PostInstallHint coachTitle="Tempo" handle="tempo-coach" onOpenChat={onOpenChat} onDismiss={jest.fn()} />,
    );

    fireEvent.press(getByTestId('post-install-open-chat'));

    expect(onOpenChat).toHaveBeenCalledWith('/coach add @tempo-coach');
  });

  it('dismisses through onDismiss', () => {
    const onDismiss = jest.fn();
    const { getByTestId } = render(
      <PostInstallHint coachTitle="Tempo" handle="tempo-coach" onOpenChat={jest.fn()} onDismiss={onDismiss} />,
    );

    fireEvent.press(getByTestId('post-install-dismiss'));

    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('reads as the literal @handle placeholder when the payload carries no handle', () => {
    expect(coachMention(undefined)).toBe('@handle');
    expect(coachAddDraft(undefined)).toBe('/coach add @handle');
    expect(coachAddDraft('tempo-coach')).toBe('/coach add @tempo-coach');
  });
});
