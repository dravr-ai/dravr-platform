// ABOUTME: Hook for managing chat messages state and operations
// ABOUTME: Handles loading, sending, insights, feedback, and message rendering logic

import React, { useState, useCallback, useRef, useEffect } from 'react';
import { type FlashListRef } from '@shopify/flash-list';
import { v4 as uuidv4 } from 'uuid';
import { chatApi } from '../../services/api';
import { isInsightPrompt, detectInsightMessages, createInsightPrompt, filterDisplayMessages } from '@pierre/chat-utils';
import type { Message } from '../../types';

/**
 * Slash-command action button (e.g. per-coach select on `/coach`).
 * Attached to an assistant message for the current turn only.
 */
export interface MessageActionItem {
  label: string;
  action_type: string;
  value: string;
}

export interface MessagesState {
  messages: Message[];
  isSending: boolean;
  error: string | null;
  messageFeedback: Record<string, 'up' | 'down' | null>;
  /** Saved thumbs-down reasons, keyed by message id. */
  messageFeedbackComment: Record<string, string>;
  insightMessages: Set<string>;
  /** Activity lists keyed by assistant message ID (from new API field) */
  activityLists: Record<string, string>;
  /**
   * Slash-command action buttons keyed by assistant message id. Present
   * when the server returned a card with selectable options (e.g.
   * `/coach` → per-coach buttons). Not persisted — cleared on
   * conversation switch; history re-renders show the text body only.
   */
  messageActions: Record<string, MessageActionItem[]>;
  /**
   * AG-UI run id for the in-flight turn, or `null` between turns.
   * Components pass this into `useAgUiProgress` to render pipeline
   * progress (e.g. "reading your question…") while the assistant is
   * working. Reset to `null` once the HTTP turn response lands.
   */
  aguiRunId: string | null;
}

export interface MessagesActions {
  loadMessages: (conversationId: string) => Promise<void>;
  sendMessage: (
    conversationId: string,
    messageText: string,
    onConversationNeeded?: () => Promise<string | null>
  ) => Promise<void>;
  createInsight: (
    content: string,
    conversationId: string | undefined,
    onConversationNeeded?: () => Promise<string | null>
  ) => Promise<void>;
  retryMessage: (messageId: string, conversationId: string) => Promise<void>;
  handleThumbsUp: (messageId: string, conversationId: string) => Promise<void>;
  handleThumbsDown: (messageId: string, conversationId: string) => Promise<void>;
  submitFeedbackReason: (
    messageId: string,
    conversationId: string,
    comment: string
  ) => Promise<void>;
  clearMessages: () => void;
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
  setActivityLists: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  setIsSending: (sending: boolean) => void;
  scrollToBottom: () => void;
  flatListRef: React.RefObject<FlashListRef<Message> | null>;
}

export function useMessages(): MessagesState & MessagesActions {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [messageFeedback, setMessageFeedback] = useState<Record<string, 'up' | 'down' | null>>({});
  const [messageFeedbackComment, setMessageFeedbackComment] = useState<Record<string, string>>({});
  const [insightMessages, setInsightMessages] = useState<Set<string>>(new Set());
  const [activityLists, setActivityLists] = useState<Record<string, string>>({});
  const [messageActions, setMessageActions] = useState<Record<string, MessageActionItem[]>>({});
  const [aguiRunId, setAguiRunId] = useState<string | null>(null);
  const flatListRef = useRef<FlashListRef<Message>>(null);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const scrollToBottom = useCallback(() => {
    if (flatListRef.current && messages.length > 0) {
      flatListRef.current.scrollToEnd({ animated: true });
    }
  }, [messages.length]);

  // Safe deferred scroll that auto-clears previous timer
  const deferredScrollToBottom = useCallback((delayMs: number) => {
    if (scrollTimerRef.current) {
      clearTimeout(scrollTimerRef.current);
    }
    scrollTimerRef.current = setTimeout(() => {
      scrollToBottom();
      scrollTimerRef.current = null;
    }, delayMs);
  }, [scrollToBottom]);

  // Cleanup scroll timer on unmount
  useEffect(() => {
    return () => {
      if (scrollTimerRef.current) {
        clearTimeout(scrollTimerRef.current);
      }
    };
  }, []);

  const loadMessages = useCallback(async (conversationId: string) => {
    try {
      setError(null);
      const response = await chatApi.getConversationMessages(conversationId);
      const allMessages = response.messages || [];

      const detectedInsights = detectInsightMessages(allMessages);
      if (detectedInsights.size > 0) {
        setInsightMessages(prev => {
          const merged = new Set(prev);
          detectedInsights.forEach(id => merged.add(id));
          return merged;
        });
      }

      // Drop internal LLM plumbing rows (tool_call / tool_result) so their raw
      // <tool_call>/<tool_result> XML never renders — critical for
      // messaging-origin conversations (Telegram etc.) that carry the same
      // scaffolding rows as native chat. Then drop insight prompt user turns.
      const filteredMessages = filterDisplayMessages(allMessages).filter(
        (msg: Message) => !(msg.role === 'user' && isInsightPrompt(msg.content))
      );
      setMessages(filteredMessages);

      // Hydrate thumbs up/down state (and any saved reason) from the server so
      // feedback survives reloads and conversation switches.
      const ratings: Record<string, 'up' | 'down' | null> = {};
      const comments: Record<string, string> = {};
      for (const f of response.feedback ?? []) {
        ratings[f.message_id] = f.rating;
        if (f.comment) comments[f.message_id] = f.comment;
      }
      setMessageFeedback(ratings);
      setMessageFeedbackComment(comments);

      deferredScrollToBottom(100);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load messages';
      setError(errorMessage);
      console.error('Failed to load messages:', err);
    }
  }, [deferredScrollToBottom]);

  const sendMessage = useCallback(async (
    conversationId: string,
    messageText: string,
  ) => {
    if (!messageText.trim() || isSending) return;

    setIsSending(true);
    setError(null);

    const userMessage: Message = {
      id: `temp-${Date.now()}`,
      role: 'user',
      content: messageText,
      created_at: new Date().toISOString(),
    };
    setMessages(prev => [...prev, userMessage]);
    deferredScrollToBottom(200);

    // Fresh run id per turn — the SSE consumer in `useAgUiProgress`
    // opens a subscription against `/api/agui/runs/{runId}/stream`
    // while the REST request is in flight. Resetting to null in the
    // `finally` below closes the subscription once the assistant
    // reply has rendered via the REST response.
    const runId = uuidv4();
    setAguiRunId(runId);

    try {
      const response = await chatApi.sendMessage(conversationId, messageText, {
        aguiRunId: runId,
      });

      // Store activity list if the API returned one
      if (response.activity_list && response.assistant_message?.id) {
        setActivityLists(prev => ({
          ...prev,
          [response.assistant_message.id]: response.activity_list as string,
        }));
      }

      // Slash-command responses (e.g. /coach) carry clickable action
      // buttons. Attach them by assistant message id so the list
      // renderer can show them below the body.
      if (
        Array.isArray(response.actions) &&
        response.actions.length > 0 &&
        response.assistant_message?.id
      ) {
        setMessageActions(prev => ({
          ...prev,
          [response.assistant_message.id]: response.actions as MessageActionItem[],
        }));
      }

      setMessages(prev => {
        const filtered = prev.filter(m => m.id !== userMessage.id);
        const newMessages: Message[] = [];
        if (response.user_message?.id) {
          newMessages.push(response.user_message);
        }
        if (response.assistant_message?.id) {
          newMessages.push({
            ...response.assistant_message,
            model: response.model,
            execution_time_ms: response.execution_time_ms,
          });
        }
        return [...filtered, ...newMessages];
      });
      deferredScrollToBottom(200);
    } catch (sendErr) {
      const errorMsg = sendErr instanceof Error ? sendErr.message : 'Failed to send message';
      setError(errorMsg);
      const errorResponse: Message = {
        id: `error-${Date.now()}`,
        role: 'assistant',
        content: `⚠️ ${errorMsg}\n\nPlease try again.`,
        created_at: new Date().toISOString(),
        isError: true,
      };
      setMessages(prev => {
        const updated = prev.map(m =>
          m.id === userMessage.id ? { ...m, id: `user-${Date.now()}` } : m
        );
        return [...updated, errorResponse];
      });
      deferredScrollToBottom(200);
    } finally {
      setIsSending(false);
      setAguiRunId(null);
    }
  }, [isSending, deferredScrollToBottom]);

  const createInsight = useCallback(async (
    content: string,
    conversationId: string | undefined,
    onConversationNeeded?: () => Promise<string | null>
  ) => {
    if (isSending) return;

    let resolvedConversationId = conversationId;
    if (!resolvedConversationId && onConversationNeeded) {
      resolvedConversationId = (await onConversationNeeded()) ?? undefined;
      if (!resolvedConversationId) return;
    }
    if (!resolvedConversationId) return;

    setIsSending(true);
    setError(null);
    const insightPrompt = createInsightPrompt(content);
    deferredScrollToBottom(200);

    // Insight generation intentionally skips `aguiRunId` — the server
    // refuses it on the insight path with a 400 because the insight
    // endpoint returns a single JSON payload rather than a streaming
    // pipeline run. Don't regress that contract.
    try {
      const response = await chatApi.sendMessage(resolvedConversationId, insightPrompt);

      if (response.assistant_message?.id) {
        setInsightMessages(prev => {
          const updated = new Set(prev);
          updated.add(response.assistant_message.id);
          return updated;
        });

        setMessages(prev => [...prev, {
          ...response.assistant_message,
          model: response.model,
          execution_time_ms: response.execution_time_ms,
        }]);
      }
      deferredScrollToBottom(200);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to generate insight';
      setError(errorMsg);
      console.error('Failed to create insight:', err);
    } finally {
      setIsSending(false);
    }
  }, [isSending, deferredScrollToBottom]);

  const retryMessage = useCallback(async (messageId: string, conversationId: string) => {
    const messageIndex = messages.findIndex(m => m.id === messageId);
    if (messageIndex <= 0) return;

    const userMessage = messages[messageIndex - 1];
    if (userMessage.role !== 'user') return;

    setMessages(prev => prev.filter(m => m.id !== messageId));
    setIsSending(true);
    setError(null);

    const runId = uuidv4();
    setAguiRunId(runId);

    try {
      const response = await chatApi.sendMessage(conversationId, userMessage.content, {
        aguiRunId: runId,
      });

      // Store activity list if the API returned one
      if (response.activity_list && response.assistant_message?.id) {
        setActivityLists(prev => ({
          ...prev,
          [response.assistant_message.id]: response.activity_list as string,
        }));
      }

      setMessages(prev => {
        if (response.assistant_message?.id) {
          return [...prev, {
            ...response.assistant_message,
            model: response.model,
            execution_time_ms: response.execution_time_ms,
          }];
        }
        return prev;
      });
      deferredScrollToBottom(200);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to get response';
      setError(errorMsg);
      const errorMessage: Message = {
        id: `error-${Date.now()}`,
        role: 'assistant',
        content: `⚠️ ${errorMsg}\n\nPlease try again.`,
        created_at: new Date().toISOString(),
        isError: true,
      };
      setMessages(prev => [...prev, errorMessage]);
      deferredScrollToBottom(200);
    } finally {
      setIsSending(false);
      setAguiRunId(null);
    }
  }, [messages, deferredScrollToBottom]);

  // Apply a rating change optimistically and persist it. Clicking the active
  // rating again toggles it off (DELETE); otherwise the rating is upserted.
  // On failure the optimistic change is reverted and an error surfaced.
  const applyFeedback = useCallback(
    async (messageId: string, conversationId: string, rating: 'up' | 'down') => {
      const previous = messageFeedback[messageId] ?? null;
      const next: 'up' | 'down' | null = previous === rating ? null : rating;

      setMessageFeedback(prev => ({ ...prev, [messageId]: next }));
      // Switching away from a down-rating drops its reason.
      if (next !== 'down') {
        setMessageFeedbackComment(prev => {
          if (!(messageId in prev)) return prev;
          const updated = { ...prev };
          delete updated[messageId];
          return updated;
        });
      }

      try {
        if (next === null) {
          await chatApi.deleteMessageFeedback(conversationId, messageId);
        } else {
          await chatApi.submitMessageFeedback(conversationId, messageId, next);
        }
      } catch (err) {
        setMessageFeedback(prev => ({ ...prev, [messageId]: previous }));
        setError(err instanceof Error ? err.message : 'Failed to save feedback');
      }
    },
    [messageFeedback]
  );

  const handleThumbsUp = useCallback(
    (messageId: string, conversationId: string) => applyFeedback(messageId, conversationId, 'up'),
    [applyFeedback]
  );

  const handleThumbsDown = useCallback(
    (messageId: string, conversationId: string) => applyFeedback(messageId, conversationId, 'down'),
    [applyFeedback]
  );

  // Persist an optional thumbs-down reason on the existing feedback row. The
  // down rating is already saved; this only adds/updates the comment.
  const submitFeedbackReason = useCallback(
    async (messageId: string, conversationId: string, comment: string) => {
      const trimmed = comment.trim();
      setMessageFeedbackComment(prev => ({ ...prev, [messageId]: trimmed }));
      try {
        await chatApi.submitMessageFeedback(
          conversationId,
          messageId,
          'down',
          trimmed || undefined
        );
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to save feedback');
      }
    },
    []
  );

  const clearMessages = useCallback(() => {
    setMessages([]);
    setError(null);
  }, []);

  return {
    messages,
    isSending,
    error,
    messageFeedback,
    messageFeedbackComment,
    insightMessages,
    activityLists,
    messageActions,
    aguiRunId,
    loadMessages,
    sendMessage,
    createInsight,
    retryMessage,
    handleThumbsUp,
    handleThumbsDown,
    submitFeedbackReason,
    clearMessages,
    setMessages,
    setActivityLists,
    setIsSending,
    scrollToBottom,
    flatListRef,
  };
}
