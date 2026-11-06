// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Regenerate is drawn only on a reply whose turn the server says changed nothing
// ABOUTME: It re-sends the question on a fresh budget, so a blocked or writing turn, or a row that does not say, offers none

import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import MessageList from '../MessageList';
import type { Message } from '../types';

const QUESTION: Message = {
  id: 'q',
  role: 'user',
  content: 'Delete my tempo plan',
  created_at: '2026-11-06T14:00:00Z',
};

function renderReply(extra: Partial<Message>, onRetryMessage = vi.fn()) {
  const reply: Message = {
    id: 'r',
    role: 'assistant',
    content: 'Here is the reply.',
    created_at: '2026-11-06T14:00:05Z',
    finish_reason: 'stop',
    ...extra,
  };
  render(
    <MessageList
      messages={[QUESTION, reply]}
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
      onRetryMessage={onRetryMessage}
    />,
  );
}

describe('MessageList Regenerate', () => {
  it('offers it when the turn attempted no write, and re-runs that reply', async () => {
    const onRetryMessage = vi.fn();
    renderReply({ turn_attempted_write: false }, onRetryMessage);

    await userEvent.click(screen.getByTitle('Regenerate response'));

    expect(onRetryMessage).toHaveBeenCalledWith('r');
  });

  it('offers none when the turn was blocked or attempted a write', () => {
    renderReply({ turn_attempted_write: true });
    expect(screen.queryByTitle('Regenerate response')).toBeNull();
    // The rest of the reply's actions stay.
    expect(screen.getByTitle('Good response')).toBeInTheDocument();
  });

  it('offers none on a row that does not say', () => {
    renderReply({ turn_attempted_write: null });
    expect(screen.queryByTitle('Regenerate response')).toBeNull();
  });

  it('offers none on a row from before the field existed', () => {
    renderReply({});
    expect(screen.queryByTitle('Regenerate response')).toBeNull();
  });
});
