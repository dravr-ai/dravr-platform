// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the transcript read in windows — reaching the top asks for the window before, a messaging row is tagged
// ABOUTME: The personal thread is shared with the athlete's DM and never rotates, so it loads from the newest back (carnet#840)

import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import MessageList from '../MessageList';
import type { Message } from '../types';

function row(id: string, extra: Partial<Message> = {}): Message {
  return { id, role: 'user', content: `words ${id}`, created_at: '2026-10-20T14:00:00Z', ...extra };
}

function renderList(props: Partial<Parameters<typeof MessageList>[0]> = {}) {
  return render(
    <MessageList
      messages={[row('a'), row('b', { role: 'assistant' })]}
      messageMetadata={new Map()}
      messageFeedback={new Map()}
      messageFeedbackComment={new Map()}
      isLoading={false}
      isStreaming={false}
      streamingContent=""
      errorMessage={null}
      oauthNotification={null}
      onDismissError={vi.fn()}
      onDismissOAuthNotification={vi.fn()}
      onCopyMessage={vi.fn()}
      onShareMessage={vi.fn()}
      onThumbsUp={vi.fn()}
      onThumbsDown={vi.fn()}
      onSubmitFeedbackReason={vi.fn()}
      onRetryMessage={vi.fn()}
      {...props}
    />,
  );
}

/** An observer that reports its target in view as soon as it is observed. */
class VisibleObserver {
  constructor(private readonly callback: IntersectionObserverCallback) {}
  observe(target: Element) {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this as never);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

describe('MessageList windows', () => {
  const original = global.IntersectionObserver;
  afterEach(() => {
    global.IntersectionObserver = original;
  });

  it('asks for the window before once the reader reaches the top', () => {
    global.IntersectionObserver = VisibleObserver as unknown as typeof IntersectionObserver;
    const onReachStart = vi.fn();
    renderList({ hasEarlier: true, onReachStart });
    expect(screen.getByTestId('earlier-messages')).toBeInTheDocument();
    expect(onReachStart).toHaveBeenCalled();
  });

  it('asks for nothing when the whole thread is held', () => {
    global.IntersectionObserver = VisibleObserver as unknown as typeof IntersectionObserver;
    const onReachStart = vi.fn();
    renderList({ hasEarlier: false, onReachStart });
    expect(screen.queryByTestId('earlier-messages')).not.toBeInTheDocument();
    expect(onReachStart).not.toHaveBeenCalled();
  });

  it('asks for nothing while a window is already being read, and says so', () => {
    global.IntersectionObserver = VisibleObserver as unknown as typeof IntersectionObserver;
    const onReachStart = vi.fn();
    renderList({ hasEarlier: true, loadingEarlier: true, onReachStart });
    expect(screen.getByRole('status')).toBeInTheDocument();
    expect(onReachStart).not.toHaveBeenCalled();
  });

  it('opens on the newest row at once, so the top of the window is never taken for a read back', () => {
    const scroll = vi.spyOn(Element.prototype, 'scrollIntoView');
    renderList({ hasEarlier: true, onReachStart: vi.fn() });
    expect(scroll).toHaveBeenCalledWith({ behavior: 'auto' });
    scroll.mockRestore();
  });

  it('tags the athlete words that came from a messaging app', () => {
    renderList({
      messages: [
        row('t', { channel_type: 'telegram', content: 'sent from telegram' }),
        row('w', { channel_type: 'web', content: 'typed in the app', created_at: '2026-10-20T15:00:00Z' }),
      ],
    });
    expect(screen.getAllByText(/From Telegram/)).toHaveLength(1);
  });
});
