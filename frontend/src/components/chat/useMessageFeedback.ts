// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The thumbs up/down ratings and thumbs-down reasons on a thread's replies, kept in step with the server
// ABOUTME: Hydrated from each read of the thread; a rating is applied at once and reverted if the save fails

import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import type { MessagesResponse } from '@pierre/api-client';
import { chatApi } from '../../services/api';
import type { MessageFeedback } from './types';

/** The ratings drawn under the replies, and what a press on them does. */
export interface MessageFeedbackState {
  messageFeedback: Map<string, MessageFeedback>;
  messageFeedbackComment: Map<string, string>;
  thumbsUp: (messageId: string) => void;
  thumbsDown: (messageId: string) => void;
  submitReason: (messageId: string, comment: string) => Promise<void>;
}

/**
 * The open thread's reply ratings.
 *
 * The server is the source of truth: every read of the thread replaces the
 * local maps, so a rating survives a reload and a switch of thread. A save
 * that fails is worded through `onError`, the thread's own error line.
 */
export function useMessageFeedback(
  conversationId: string | null,
  messagesData: MessagesResponse | undefined,
  onError: (text: string) => void,
): MessageFeedbackState {
  const { t } = useTranslation();
  const [messageFeedback, setMessageFeedback] = useState<Map<string, MessageFeedback>>(new Map());
  // Saved thumbs-down reasons, keyed by message id. Hydrated from the server
  // alongside the ratings so a reload re-renders the comment.
  const [messageFeedbackComment, setMessageFeedbackComment] = useState<Map<string, string>>(new Map());

  // Hydrate thumbs up/down state (and any saved reason) from the server whenever
  // the messages load/refetch, so feedback survives reloads and conversation
  // switches. The server is the source of truth — this replaces local state.
  useEffect(() => {
    const feedback = messagesData?.feedback;
    if (!feedback) return;
    const ratings = new Map<string, MessageFeedback>();
    const comments = new Map<string, string>();
    for (const f of feedback) {
      ratings.set(f.message_id, f.rating);
      if (f.comment) comments.set(f.message_id, f.comment);
    }
    setMessageFeedback(ratings);
    setMessageFeedbackComment(comments);
  }, [messagesData]);

  // Apply a rating change optimistically and persist it. Clicking the active
  // rating again toggles it off (DELETE); otherwise the rating is upserted.
  // On failure the optimistic change is reverted and an error surfaced.
  const applyFeedback = useCallback(async (messageId: string, rating: 'up' | 'down') => {
    if (!conversationId) return;
    const previous = messageFeedback.get(messageId) ?? null;
    const next: MessageFeedback = previous === rating ? null : rating;

    setMessageFeedback(prev => {
      const newMap = new Map(prev);
      if (next) newMap.set(messageId, next);
      else newMap.delete(messageId);
      return newMap;
    });
    // Switching away from a down-rating drops its reason.
    if (next !== 'down') {
      setMessageFeedbackComment(prev => {
        if (!prev.has(messageId)) return prev;
        const newMap = new Map(prev);
        newMap.delete(messageId);
        return newMap;
      });
    }

    try {
      if (next === null) {
        await chatApi.deleteMessageFeedback(conversationId, messageId);
      } else {
        await chatApi.submitMessageFeedback(conversationId, messageId, next);
      }
    } catch (error) {
      // Revert the optimistic rating on failure.
      setMessageFeedback(prev => {
        const newMap = new Map(prev);
        if (previous) newMap.set(messageId, previous);
        else newMap.delete(messageId);
        return newMap;
      });
      onError(describeApiError(error, { t, fallbackKey: 'chat.feedbackSaveFailed' }));
    }
  }, [conversationId, messageFeedback, onError, t]);

  const thumbsUp = useCallback((messageId: string) => {
    void applyFeedback(messageId, 'up');
  }, [applyFeedback]);

  const thumbsDown = useCallback((messageId: string) => {
    void applyFeedback(messageId, 'down');
  }, [applyFeedback]);

  // Persist an optional thumbs-down reason on the existing feedback row. The
  // down rating is already saved; this only adds/updates the comment.
  const submitReason = useCallback(async (messageId: string, comment: string) => {
    if (!conversationId) return;
    const trimmed = comment.trim();
    setMessageFeedbackComment(prev => {
      const newMap = new Map(prev);
      if (trimmed) newMap.set(messageId, trimmed);
      else newMap.delete(messageId);
      return newMap;
    });
    try {
      await chatApi.submitMessageFeedback(
        conversationId,
        messageId,
        'down',
        trimmed || undefined,
      );
    } catch (error) {
      onError(describeApiError(error, { t, fallbackKey: 'chat.feedbackSaveFailed' }));
    }
  }, [conversationId, onError, t]);

  return { messageFeedback, messageFeedbackComment, thumbsUp, thumbsDown, submitReason };
}
