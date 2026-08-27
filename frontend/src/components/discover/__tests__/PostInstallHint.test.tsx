// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the post-install hint — the copy it teaches and the draft it hands to Open chat
// ABOUTME: Pins the /coach add @handle command and the @handle mention as the two ways to use a coach

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import PostInstallHint from '../PostInstallHint';
import { coachAddDraft, coachMention } from '../coachDraft';

describe('PostInstallHint', () => {
  it('teaches /coach add @handle and the @handle mention for the installed coach', () => {
    render(
      <PostInstallHint
        coachTitle="Marathon Training Coach"
        handle="marathon-training-coach"
        onOpenChat={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    const hint = screen.getByTestId('post-install-hint');
    expect(hint).toHaveTextContent('“Marathon Training Coach” is in your coaches');
    expect(hint).toHaveTextContent(
      'Use it in any chat: /coach add @marathon-training-coach — or mention @marathon-training-coach for one turn',
    );
  });

  it('hands the /coach add draft to Open chat', () => {
    const onOpenChat = vi.fn();
    render(
      <PostInstallHint coachTitle="Tempo" handle="tempo-coach" onOpenChat={onOpenChat} onDismiss={vi.fn()} />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));

    expect(onOpenChat).toHaveBeenCalledWith('/coach add @tempo-coach');
  });

  it('dismisses through onDismiss', () => {
    const onDismiss = vi.fn();
    render(<PostInstallHint coachTitle="Tempo" handle="tempo-coach" onOpenChat={vi.fn()} onDismiss={onDismiss} />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));

    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('reads as the literal @handle placeholder when the payload carries no handle', () => {
    expect(coachMention(undefined)).toBe('@handle');
    expect(coachAddDraft(undefined)).toBe('/coach add @handle');
    expect(coachAddDraft('tempo-coach')).toBe('/coach add @tempo-coach');
  });
});
