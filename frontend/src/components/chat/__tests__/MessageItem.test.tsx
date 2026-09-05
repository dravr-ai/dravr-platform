// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for MessageItem component
// ABOUTME: Verifies action bar behavior, error state, and feedback states

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import MessageItem from '../MessageItem';
import type { Message, MessageMetadata } from '../types';

const mockAssistantMessage: Message = {
  id: 'msg-1',
  role: 'assistant',
  content: 'This is a test response from Dravr.',
  created_at: new Date().toISOString(),
};

const mockUserMessage: Message = {
  id: 'msg-2',
  role: 'user',
  content: 'Hello Dravr!',
  created_at: new Date().toISOString(),
};

const mockMetadata: MessageMetadata = {
  model: 'gemini-1.5-flash',
  executionTimeMs: 2500,
};

describe('MessageItem', () => {
  describe('basic rendering', () => {
    it('should render assistant message with Dravr avatar', () => {
      render(<MessageItem message={mockAssistantMessage} />);

      expect(screen.getByText('Dravr')).toBeInTheDocument();
      expect(screen.getByText('This is a test response from Dravr.')).toBeInTheDocument();
      expect(screen.getByRole('img', { name: 'Dravr' })).toBeInTheDocument();
    });

    it('should render user message with user avatar', () => {
      render(<MessageItem message={mockUserMessage} />);

      expect(screen.getByText('You')).toBeInTheDocument();
      expect(screen.getByText('Hello Dravr!')).toBeInTheDocument();
    });

    it('should display metadata when provided', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          metadata={mockMetadata}
          onCopy={vi.fn()}
        />
      );

      expect(screen.getByText(/gemini-1.5-flash/)).toBeInTheDocument();
      expect(screen.getByText(/2\.5s/)).toBeInTheDocument();
    });
  });

  describe('action buttons', () => {
    it('renders the action row for assistant messages without any insight affordance', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          onCopy={vi.fn()}
          onShare={vi.fn()}
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
          onRetry={vi.fn()}
        />
      );

      expect(screen.getByTitle('Copy message')).toBeInTheDocument();
      expect(screen.getByTitle('Share')).toBeInTheDocument();
      expect(screen.getByTitle('Good response')).toBeInTheDocument();
      expect(screen.getByTitle('Poor response')).toBeInTheDocument();
      expect(screen.getByTitle('Regenerate response')).toBeInTheDocument();
      // The social feed was retired by the Chat-First Cutover: neither the
      // "create insight" lightbulb nor the "share to feed" button exists.
      expect(screen.queryByTitle('Create shareable insight')).not.toBeInTheDocument();
      expect(screen.queryByTitle('Share insight')).not.toBeInTheDocument();
    });

    it('should not render action buttons for user messages', () => {
      render(
        <MessageItem
          message={mockUserMessage}
          onCopy={vi.fn()}
          onShare={vi.fn()}
        />
      );

      expect(screen.queryByTitle('Copy message')).not.toBeInTheDocument();
      expect(screen.queryByTitle('Share')).not.toBeInTheDocument();
    });

    it('should call onCopy when copy button is clicked', async () => {
      const user = userEvent.setup();
      const onCopy = vi.fn();

      render(
        <MessageItem
          message={mockAssistantMessage}
          onCopy={onCopy}
        />
      );

      await user.click(screen.getByTitle('Copy message'));
      expect(onCopy).toHaveBeenCalledTimes(1);
    });

    it('should call onShare when share button is clicked', async () => {
      const user = userEvent.setup();
      const onShare = vi.fn();

      render(
        <MessageItem
          message={mockAssistantMessage}
          onShare={onShare}
        />
      );

      await user.click(screen.getByTitle('Share'));
      expect(onShare).toHaveBeenCalledTimes(1);
    });

    it('should call onRetry when retry button is clicked', async () => {
      const user = userEvent.setup();
      const onRetry = vi.fn();

      render(
        <MessageItem
          message={mockAssistantMessage}
          onRetry={onRetry}
        />
      );

      await user.click(screen.getByTitle('Regenerate response'));
      expect(onRetry).toHaveBeenCalledTimes(1);
    });
  });

  describe('feedback states', () => {
    it('should show thumbs up as active when feedback is up', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback="up"
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
        />
      );

      const thumbsUpButton = screen.getByTitle('Good response');
      expect(thumbsUpButton).toHaveClass('text-primary');
    });

    it('should show thumbs down as active when feedback is down', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback="down"
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
        />
      );

      const thumbsDownButton = screen.getByTitle('Poor response');
      expect(thumbsDownButton).toHaveClass('text-error');
    });

    it('should show both buttons as inactive when feedback is null', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback={null}
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
        />
      );

      const thumbsUpButton = screen.getByTitle('Good response');
      const thumbsDownButton = screen.getByTitle('Poor response');

      expect(thumbsUpButton).toHaveClass('text-outline');
      expect(thumbsDownButton).toHaveClass('text-outline');
    });
  });

  describe('error state', () => {
    it('should show only Retry button with label when isError is true', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          isError={true}
          onCopy={vi.fn()}
          onShare={vi.fn()}
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
          onRetry={vi.fn()}
        />
      );

      // Should show retry button with label
      expect(screen.getByText('Retry')).toBeInTheDocument();

      // Should NOT show other action buttons
      expect(screen.queryByTitle('Copy message')).not.toBeInTheDocument();
      expect(screen.queryByTitle('Share')).not.toBeInTheDocument();
      expect(screen.queryByTitle('Good response')).not.toBeInTheDocument();
      expect(screen.queryByTitle('Poor response')).not.toBeInTheDocument();
    });

    it('should apply error styling to content when isError is true', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          isError={true}
          onRetry={vi.fn()}
        />
      );

      const contentDiv = screen.getByText('This is a test response from Dravr.').closest('div');
      expect(contentDiv).toHaveClass('text-error');
    });

    it('should call onRetry when error retry button is clicked', async () => {
      const user = userEvent.setup();
      const onRetry = vi.fn();

      render(
        <MessageItem
          message={mockAssistantMessage}
          isError={true}
          onRetry={onRetry}
        />
      );

      await user.click(screen.getByText('Retry'));
      expect(onRetry).toHaveBeenCalledTimes(1);
    });
  });

  describe('thumbs-down reason', () => {
    it('shows the reason form when feedback is down', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback="down"
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
          onSubmitReason={vi.fn()}
        />
      );

      expect(screen.getByPlaceholderText('What went wrong? (optional)')).toBeInTheDocument();
    });

    it('does not show the reason form when feedback is up', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback="up"
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
          onSubmitReason={vi.fn()}
        />
      );

      expect(screen.queryByPlaceholderText('What went wrong? (optional)')).not.toBeInTheDocument();
    });

    it('pre-fills the form with the saved reason', () => {
      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback="down"
          feedbackComment="not enough detail"
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
          onSubmitReason={vi.fn()}
        />
      );

      expect(screen.getByDisplayValue('not enough detail')).toBeInTheDocument();
    });

    it('submits the trimmed reason and shows a saved state', async () => {
      const user = userEvent.setup();
      const onSubmitReason = vi.fn();

      render(
        <MessageItem
          message={mockAssistantMessage}
          feedback="down"
          onThumbsUp={vi.fn()}
          onThumbsDown={vi.fn()}
          onSubmitReason={onSubmitReason}
        />
      );

      await user.type(screen.getByPlaceholderText('What went wrong? (optional)'), '  too generic  ');
      await user.click(screen.getByRole('button', { name: 'Send' }));

      expect(onSubmitReason).toHaveBeenCalledWith('too generic');
      expect(screen.getByRole('button', { name: 'Saved' })).toBeInTheDocument();
    });
  });

  describe('tool scaffolding stripping', () => {
    it('strips residual <tool_result> XML embedded in displayed content', () => {
      // Defensive guard: whole tool_call/tool_result rows are filtered out in
      // MessageList, but if scaffolding leaks into a visible turn's content it
      // must not be dumped at the user (matters for messaging-origin chats).
      const messageWithScaffolding: Message = {
        id: 'msg-scaffold',
        role: 'assistant',
        content:
          'Your recovery looks good. <tool_result>{"hrv":65}</tool_result> Keep it up.',
        created_at: new Date().toISOString(),
      };

      render(<MessageItem message={messageWithScaffolding} />);

      expect(screen.getByText(/Your recovery looks good\./)).toBeInTheDocument();
      expect(screen.getByText(/Keep it up\./)).toBeInTheDocument();
      expect(screen.queryByText(/tool_result/)).not.toBeInTheDocument();
      expect(screen.queryByText(/hrv/)).not.toBeInTheDocument();
    });
  });
});
