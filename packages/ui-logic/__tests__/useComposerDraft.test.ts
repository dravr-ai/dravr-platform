// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the composer's per-thread draft — each chat keeps its own unsent line, "new chat" included
// ABOUTME: Pins thread and user isolation, the seeded flag per thread, the carry into a thread made for a line, writes off screen, and a remount finding its line

import { describe, it, expect } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { carryNoThreadDraft, useComposerDraft, writeComposerDraft } from '../src/useComposerDraft';

// The store outlives every render on purpose, so each test speaks as its own user.
let users = 0;
function freshUser(): string {
  users += 1;
  return `user-${users}`;
}

function renderDraft(owner: string | null, thread: string | null) {
  return renderHook(({ threadId }) => useComposerDraft(owner, threadId), {
    initialProps: { threadId: thread },
  });
}

describe('useComposerDraft', () => {
  it('keeps each thread’s unsent line to that thread', () => {
    const owner = freshUser();
    const { result, rerender } = renderDraft(owner, 'thread-a');

    act(() => result.current.setText('je suis pret'));
    rerender({ threadId: 'thread-b' });
    expect(result.current.text).toBe('');

    act(() => result.current.setText('see you Saturday'));
    rerender({ threadId: 'thread-a' });
    expect(result.current.text).toBe('je suis pret');

    rerender({ threadId: 'thread-b' });
    expect(result.current.text).toBe('see you Saturday');
  });

  it('applies an updater to the open thread’s own line', () => {
    const { result } = renderDraft(freshUser(), 'thread-a');
    act(() => result.current.setText('Ride'));
    act(() => result.current.setText(current => `${current} tomorrow`));
    expect(result.current.text).toBe('Ride tomorrow');
  });

  it('keeps whether a draft affordance put the line there per thread, until the line is emptied', () => {
    const { result, rerender } = renderDraft(freshUser(), 'thread-a');

    act(() => result.current.seed('Back up this claim', true));
    act(() => result.current.setText('Back up this claim, please'));
    expect(result.current.seeded).toBe(true);

    rerender({ threadId: 'thread-b' });
    act(() => result.current.setText('typed by hand'));
    expect(result.current.seeded).toBe(false);

    rerender({ threadId: 'thread-a' });
    expect(result.current.seeded).toBe(true);
    act(() => result.current.setText(''));
    act(() => result.current.setText('my own words'));
    expect(result.current.seeded).toBe(false);
  });

  it('keeps the new-chat line in new chat when the athlete opens an existing thread', () => {
    const { result, rerender } = renderDraft(freshUser(), null);
    act(() => result.current.setText('typed in a new chat'));

    rerender({ threadId: 'thread-a' });
    expect(result.current.text).toBe('');

    rerender({ threadId: null });
    expect(result.current.text).toBe('typed in a new chat');
  });

  it('moves the new-chat line into the thread opened for it, origin and all', () => {
    const owner = freshUser();
    const { result, rerender } = renderDraft(owner, null);
    act(() => result.current.seed('How was my week?', true));

    act(() => carryNoThreadDraft(owner, 'thread-new'));
    rerender({ threadId: 'thread-new' });
    expect(result.current).toMatchObject({ text: 'How was my week?', seeded: true });

    // New chat is empty again for the next one.
    rerender({ threadId: null });
    expect(result.current.text).toBe('');
  });

  it('leaves a thread’s own line in place when a new-chat line would move into it', () => {
    const owner = freshUser();
    const { result, rerender } = renderDraft(owner, 'thread-a');
    act(() => result.current.setText('already here'));

    rerender({ threadId: null });
    act(() => result.current.setText('typed in a new chat'));
    act(() => carryNoThreadDraft(owner, 'thread-a'));
    rerender({ threadId: 'thread-a' });
    expect(result.current.text).toBe('already here');

    rerender({ threadId: null });
    expect(result.current.text).toBe('typed in a new chat');
  });

  it('moves nothing when new chat holds no line', () => {
    const owner = freshUser();
    const { result } = renderDraft(owner, 'thread-a');
    act(() => carryNoThreadDraft(owner, 'thread-a'));
    expect(result.current.text).toBe('');
  });

  it('never carries one thread’s line into the next thread opened from it', () => {
    const { result, rerender } = renderDraft(freshUser(), 'thread-a');
    act(() => result.current.setText('only for A'));
    rerender({ threadId: 'thread-b' });
    expect(result.current.text).toBe('');
  });

  it('writes to a thread that is not on screen without touching the one that is', () => {
    const owner = freshUser();
    const { result, rerender } = renderDraft(owner, 'thread-b');
    act(() => result.current.setText('on screen'));

    act(() => writeComposerDraft(owner, 'thread-a', held => (held.text ? held : { text: 'handed back', seeded: false })));
    expect(result.current.text).toBe('on screen');

    rerender({ threadId: 'thread-a' });
    expect(result.current.text).toBe('handed back');
  });

  it('shows the written line at once when the thread written to is on screen', () => {
    const owner = freshUser();
    const { result } = renderDraft(owner, 'thread-a');
    act(() => writeComposerDraft(owner, 'thread-a', () => ({ text: '/plan', seeded: false })));
    expect(result.current).toMatchObject({ text: '/plan', seeded: false });
  });

  it('keeps one user’s lines from another signed in on the same device', () => {
    const first = renderDraft(freshUser(), 'thread-a');
    act(() => first.result.current.setText('private to the first user'));

    const second = renderDraft(freshUser(), 'thread-a');
    expect(second.result.current.text).toBe('');
  });

  it('finds the line again after the composer’s host unmounts and remounts', () => {
    const owner = freshUser();
    const first = renderDraft(owner, 'thread-a');
    act(() => first.result.current.setText('still here'));
    first.unmount();

    const second = renderDraft(owner, 'thread-a');
    expect(second.result.current.text).toBe('still here');
  });
});
